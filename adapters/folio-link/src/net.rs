//! Peer-to-peer links: settings, identity, invites, and framed messages over
//! iroh. The host keeps one connection per guest and relays between them.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use data_encoding::{BASE64URL_NOPAD, HEXLOWER};
use iroh::endpoint::{presets, Connection, RecvStream, SendStream};
use iroh::{Endpoint, EndpointAddr, RelayMode, RelayUrl, SecretKey};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

pub const ALPN: &[u8] = b"folio-link/1";
const INVITE_PREFIX: &str = "folio-link:";
const MAX_FRAME: usize = 256 << 20;

/// Where settings and the identity key live. `FOLIO_LINK_DIR` overrides it,
/// which lets two copies run side by side.
pub fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("FOLIO_LINK_DIR") {
        return dir.into();
    }
    let home = std::env::var_os("HOME").unwrap_or_default();
    PathBuf::from(home).join("Library/Application Support/Folio/folio-link")
}

#[derive(Debug, Clone, PartialEq)]
pub enum Relay {
    Default,
    None,
    Url(RelayUrl),
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub relay: Relay,
    pub name: Option<String>,
}

/// Reads `settings.toml` if there is one. Only `key = "value"` lines matter.
pub fn load_settings() -> Settings {
    let text = std::fs::read_to_string(config_dir().join("settings.toml")).unwrap_or_default();
    parse_settings(&text)
}

fn parse_settings(text: &str) -> Settings {
    let mut settings = Settings {
        relay: Relay::Default,
        name: None,
    };
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').to_string();
        match key.trim() {
            "relay" => {
                settings.relay = match value.as_str() {
                    "default" => Relay::Default,
                    "none" => Relay::None,
                    url => match url.parse() {
                        Ok(url) => Relay::Url(url),
                        Err(_) => {
                            eprintln!("folio-link: ignoring bad relay {url:?}");
                            Relay::Default
                        }
                    },
                }
            }
            "name" if !value.is_empty() => settings.name = Some(value),
            _ => {}
        }
    }
    settings
}

/// The display name: `name` from settings, else the account's full name.
pub fn display_name(settings: &Settings) -> String {
    if let Some(name) = &settings.name {
        return name.clone();
    }
    let full = std::process::Command::new("id")
        .arg("-F")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    full.or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "Someone".into())
}

/// This install's identity, created on first use.
pub fn load_key() -> Result<SecretKey> {
    let path = config_dir().join("key");
    if let Ok(bytes) = std::fs::read(&path) {
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| anyhow!("{} is not a key", path.display()))?;
        return Ok(SecretKey::from_bytes(&bytes));
    }
    let key = SecretKey::generate();
    std::fs::create_dir_all(config_dir())?;
    std::fs::write(&path, key.to_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(key)
}

pub fn random_hex(len: usize) -> String {
    let mut bytes = vec![0u8; len];
    getrandom::fill(&mut bytes).expect("system randomness");
    HEXLOWER.encode(&bytes)
}

/// Everything a guest needs to reach the host and prove it was invited.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Invite {
    pub addr: EndpointAddr,
    pub doc: String,
    pub secret: String,
}

impl Invite {
    pub fn encode(&self) -> String {
        let json = serde_json::to_vec(self).expect("invite serializes");
        format!("{INVITE_PREFIX}{}", BASE64URL_NOPAD.encode(&json))
    }

    pub fn decode(s: &str) -> Result<Self> {
        let body = s
            .trim()
            .strip_prefix(INVITE_PREFIX)
            .context("not a Folio Link invite")?;
        let json = BASE64URL_NOPAD.decode(body.as_bytes())?;
        Ok(serde_json::from_slice(&json)?)
    }
}

/// Starts an endpoint. Only hosts accept connections.
pub async fn bind(settings: &Settings, key: SecretKey, host: bool) -> Result<Endpoint> {
    let builder = match &settings.relay {
        // No relays and no address publishing: only direct addresses work.
        Relay::None => Endpoint::builder(presets::Minimal).relay_mode(RelayMode::Disabled),
        Relay::Default => Endpoint::builder(presets::N0),
        Relay::Url(url) => {
            Endpoint::builder(presets::N0).relay_mode(RelayMode::custom([url.clone()]))
        }
    };
    let builder = builder.secret_key(key);
    let builder = if host {
        builder.alpns(vec![ALPN.to_vec()])
    } else {
        builder
    };
    builder.bind().await.map_err(|e| anyhow!("{e}"))
}

/// The host's address once it is reachable.
pub async fn ready_addr(endpoint: &Endpoint, relay: &Relay) -> EndpointAddr {
    if *relay != Relay::None {
        let _ = tokio::time::timeout(Duration::from_secs(10), endpoint.online()).await;
    }
    for _ in 0..50 {
        let addr = endpoint.addr();
        if !addr.addrs.is_empty() {
            return addr;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    endpoint.addr()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Presence {
    /// Set by the host from the connection, so nobody can speak for another.
    pub who: String,
    pub name: String,
    pub color: String,
    pub cursor: Option<Vec<u8>>,
    pub anchor: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    /// Guest → host, first thing.
    Hello {
        doc: String,
        secret: String,
        name: String,
    },
    /// Host → guest: let in. The next `Update` is the full document.
    Welcome {
        name: String,
    },
    Denied {
        reason: String,
    },
    /// Loro updates, or a snapshot right after `Welcome`.
    Update(Vec<u8>),
    Presence(Presence),
    Leave {
        who: String,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
enum JsonFrame {
    Hello {
        doc: String,
        secret: String,
        name: String,
    },
    Welcome {
        name: String,
    },
    Denied {
        reason: String,
    },
    Presence(Presence),
    Leave {
        who: String,
    },
}

const TAG_JSON: u8 = 0;
const TAG_UPDATE: u8 = 1;

impl Frame {
    fn encode(&self) -> Vec<u8> {
        let (tag, body) = match self.clone() {
            Frame::Update(bytes) => (TAG_UPDATE, bytes),
            Frame::Hello { doc, secret, name } => {
                (TAG_JSON, json(JsonFrame::Hello { doc, secret, name }))
            }
            Frame::Welcome { name } => (TAG_JSON, json(JsonFrame::Welcome { name })),
            Frame::Denied { reason } => (TAG_JSON, json(JsonFrame::Denied { reason })),
            Frame::Presence(p) => (TAG_JSON, json(JsonFrame::Presence(p))),
            Frame::Leave { who } => (TAG_JSON, json(JsonFrame::Leave { who })),
        };
        let mut out = Vec::with_capacity(body.len() + 5);
        out.extend_from_slice(&(body.len() as u32 + 1).to_be_bytes());
        out.push(tag);
        out.extend_from_slice(&body);
        out
    }

    fn decode(tag: u8, body: Vec<u8>) -> Result<Self> {
        Ok(match tag {
            TAG_UPDATE => Frame::Update(body),
            TAG_JSON => match serde_json::from_slice(&body)? {
                JsonFrame::Hello { doc, secret, name } => Frame::Hello { doc, secret, name },
                JsonFrame::Welcome { name } => Frame::Welcome { name },
                JsonFrame::Denied { reason } => Frame::Denied { reason },
                JsonFrame::Presence(p) => Frame::Presence(p),
                JsonFrame::Leave { who } => Frame::Leave { who },
            },
            _ => bail!("unknown frame {tag}"),
        })
    }
}

fn json(frame: JsonFrame) -> Vec<u8> {
    serde_json::to_vec(&frame).expect("frame serializes")
}

async fn read_frame(recv: &mut RecvStream) -> Result<Frame> {
    let mut len = [0u8; 4];
    recv.read_exact(&mut len).await?;
    let len = u32::from_be_bytes(len) as usize;
    if len == 0 || len > MAX_FRAME {
        bail!("bad frame length {len}");
    }
    let mut body = vec![0u8; len];
    recv.read_exact(&mut body).await?;
    let tag = body.remove(0);
    Frame::decode(tag, body)
}

/// What connections tell the main loop. `peer` numbers connections.
#[derive(Debug)]
pub enum NetEvent {
    Opened {
        peer: u64,
        who: String,
        tx: UnboundedSender<Frame>,
    },
    Frame {
        peer: u64,
        frame: Frame,
    },
    Closed {
        peer: u64,
    },
}

/// Short, readable form of an endpoint id.
pub fn short_id(id: &iroh::EndpointId) -> String {
    id.to_string().chars().take(10).collect()
}

/// Pumps frames both ways until either side stops, then reports `Closed`.
async fn run_conn(
    conn: Connection,
    mut send: SendStream,
    mut recv: RecvStream,
    peer: u64,
    mut out: UnboundedReceiver<Frame>,
    events: UnboundedSender<NetEvent>,
) {
    let reader = {
        let events = events.clone();
        async move {
            while let Ok(frame) = read_frame(&mut recv).await {
                if events.send(NetEvent::Frame { peer, frame }).is_err() {
                    break;
                }
            }
        }
    };
    let writer = async {
        while let Some(frame) = out.recv().await {
            let denied = matches!(frame, Frame::Denied { .. });
            if send.write_all(&frame.encode()).await.is_err() {
                return;
            }
            if denied {
                let _ = send.finish();
                tokio::time::sleep(Duration::from_millis(500)).await;
                return;
            }
        }
        // Dropped by the main loop: say goodbye cleanly.
        let _ = send.finish();
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    tokio::select! {
        _ = reader => {}
        _ = writer => {}
    }
    conn.close(0u32.into(), b"bye");
    let _ = events.send(NetEvent::Closed { peer });
}

/// Accepts guests until the endpoint closes. Peers are numbered from 1.
pub async fn accept_loop(endpoint: Endpoint, events: UnboundedSender<NetEvent>) {
    let mut next = 1u64;
    while let Some(incoming) = endpoint.accept().await {
        let peer = next;
        next += 1;
        let events = events.clone();
        tokio::spawn(async move {
            let Ok(conn) = incoming.await else { return };
            let Ok((send, recv)) = conn.accept_bi().await else {
                return;
            };
            let (tx, rx) = mpsc::unbounded_channel();
            let who = short_id(&conn.remote_id());
            if events.send(NetEvent::Opened { peer, who, tx }).is_err() {
                return;
            }
            run_conn(conn, send, recv, peer, rx, events).await;
        });
    }
}

/// Connects to the host as peer 0 and sends `hello` first.
pub async fn connect(
    endpoint: &Endpoint,
    invite: &Invite,
    hello: Frame,
    events: UnboundedSender<NetEvent>,
) -> Result<(String, UnboundedSender<Frame>)> {
    let conn = tokio::time::timeout(
        Duration::from_secs(30),
        endpoint.connect(invite.addr.clone(), ALPN),
    )
    .await
    .context("timed out reaching the host")?
    .map_err(|e| anyhow!("{e}"))?;
    let (send, recv) = conn.open_bi().await?;
    let who = short_id(&conn.remote_id());
    let (tx, rx) = mpsc::unbounded_channel();
    tx.send(hello).ok();
    tokio::spawn(run_conn(conn, send, recv, 0, rx, events));
    Ok((who, tx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_round_trips() {
        let key = SecretKey::generate();
        let invite = Invite {
            addr: EndpointAddr::new(key.public()),
            doc: random_hex(8),
            secret: random_hex(16),
        };
        let text = invite.encode();
        assert!(text.starts_with("folio-link:"));
        assert_eq!(Invite::decode(&format!(" {text}\n")).unwrap(), invite);
        assert!(Invite::decode("hello").is_err());
    }

    #[test]
    fn frames_round_trip() {
        for frame in [
            Frame::Hello {
                doc: "d".into(),
                secret: "s".into(),
                name: "Ana".into(),
            },
            Frame::Update(vec![1, 2, 3]),
            Frame::Leave { who: "x".into() },
        ] {
            let bytes = frame.encode();
            let len = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
            assert_eq!(len, bytes.len() - 4);
            assert_eq!(Frame::decode(bytes[4], bytes[5..].to_vec()).unwrap(), frame);
        }
    }

    #[test]
    fn settings_parse() {
        let s = parse_settings("# hi\nrelay = \"none\"\nname = \"Ana B\"\n");
        assert_eq!(s.relay, Relay::None);
        assert_eq!(s.name.as_deref(), Some("Ana B"));
        assert_eq!(parse_settings("").relay, Relay::Default);
        let url = parse_settings("relay = \"https://relay.example.com\"").relay;
        assert!(matches!(url, Relay::Url(_)));
    }
}
