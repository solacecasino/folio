//! Turns a manifest's capabilities into a WASI context, plus the resource
//! limits every plugin runs under.
//!
//! The starting point is WASI with nothing granted: no preopened folders
//! (so no filesystem at all), every socket address refused, no DNS, no
//! environment variables or arguments. Clocks and random numbers are always
//! available. Each capability then opens exactly one thing.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::time::Duration;

use wasmtime::{StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::sockets::SocketAddrUse;
use wasmtime_wasi::{FsPerms, WasiCtx, WasiCtxBuilder};

use crate::PluginError;
use crate::manifest::{Access, Manifest, Network};

/// Limits that apply to every plugin, whatever its manifest says. Set by
/// the editor, not by plugins.
#[derive(Debug, Clone)]
pub struct Limits {
    /// How long a plugin may run for one event before it's stopped.
    pub event_timeout: Duration,
    /// Largest memory a plugin may grow to.
    pub memory_bytes: usize,
    /// Edits, notifications and log lines a plugin may send per event.
    pub messages_per_event: u32,
    /// Total text a plugin may insert or send per event.
    pub bytes_per_event: usize,
    /// Failures in a row before a plugin is turned off.
    pub max_failures: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            event_timeout: Duration::from_secs(2),
            memory_bytes: 64 << 20,
            messages_per_event: 1_000,
            bytes_per_event: 4 << 20,
            max_failures: 3,
        }
    }
}

impl Limits {
    pub(crate) fn store_limits(&self) -> StoreLimits {
        StoreLimitsBuilder::new()
            .memory_size(self.memory_bytes)
            .instances(16)
            .tables(64)
            .memories(4)
            .trap_on_grow_failure(true)
            .build()
    }
}

/// Builds the WASI context for one plugin from its manifest.
pub(crate) async fn wasi_context(manifest: &Manifest, path: &std::path::Path) -> Result<WasiCtx, PluginError> {
    let fail = |message: String| PluginError::Manifest {
        path: path.to_path_buf(),
        message,
    };
    let mut wasi = WasiCtxBuilder::new();
    // A plugin's own stderr is handy for debugging and reveals nothing.
    wasi.inherit_stderr();

    for grant in &manifest.capabilities.filesystem {
        let host = grant.host_path();
        if !host.is_dir() {
            return Err(fail(format!("filesystem path {} isn't a folder on this computer", host.display())));
        }
        let perms = match grant.access {
            Access::Read => FsPerms::ReadOnly,
            Access::ReadWrite => FsPerms::ReadWrite,
        };
        wasi.preopened_dir(&host, grant.guest_path(), perms)
            .map_err(|e| fail(format!("couldn't open {}: {e}", host.display())))?;
    }

    // Every switch is set explicitly rather than trusting the builder's
    // defaults, which have differed from its documentation.
    match &manifest.capabilities.network {
        Network::None => {
            wasi.allow_tcp(false).allow_udp(false).allow_ip_name_lookup(false);
        }
        Network::Any => {
            wasi.inherit_network().allow_tcp(true).allow_udp(true).allow_ip_name_lookup(true);
        }
        Network::Hosts(hosts) => {
            let mut allowed: HashSet<SocketAddr> = HashSet::new();
            for h in hosts {
                let found = tokio::net::lookup_host(h.as_str())
                    .await
                    .map_err(|e| fail(format!("couldn't look up {h}: {e}")))?;
                allowed.extend(found);
            }
            let allowed = std::sync::Arc::new(allowed);
            wasi.allow_tcp(true).allow_udp(true).allow_ip_name_lookup(true).socket_addr_check(move |addr, use_| {
                let ok = match use_ {
                    // Connecting out to an allowed address, and the implicit
                    // bind to a wildcard address that comes with it.
                    SocketAddrUse::TcpConnect | SocketAddrUse::UdpSend | SocketAddrUse::UdpReceive => {
                        allowed.contains(&addr)
                    }
                    SocketAddrUse::TcpBind | SocketAddrUse::UdpBind => addr.ip().is_unspecified() && addr.port() == 0,
                    // No servers.
                    _ => false,
                };
                Box::pin(async move { ok })
            });
        }
    }
    Ok(wasi.build())
}
