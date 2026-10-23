//! A small formatting language for status lines, titles, page headers and
//! panels, modeled on foobar2000's title formatting.
//!
//! ```text
//! %words% words                 a field
//! 'literal %text%'              quoted text is printed as is
//! [%component_status%   ]       shown only if a field inside has a value
//! $if(%dirty%,•,)               functions: $if $if2 $upper $lower $plural $num $pad
//! ```
//!
//! A field with no value (or an empty one) counts as false, which is what
//! `$if` and `[ ]` test.

#[derive(Clone, Debug, PartialEq)]
enum Node {
    Text(String),
    Field(String),
    Optional(Vec<Node>),
    Call(String, Vec<Vec<Node>>),
}

/// A parsed format string, ready to evaluate many times.
#[derive(Clone, Debug, Default)]
pub struct Format {
    nodes: Vec<Node>,
}

/// The value of a piece of a format: its text, and whether it counts as true.
struct Value {
    text: String,
    truthy: bool,
}

impl Format {
    pub fn parse(src: &str) -> Result<Self, String> {
        let chars: Vec<char> = src.chars().collect();
        let mut parser = Parser { chars: &chars, i: 0 };
        Ok(Self {
            nodes: parser.sequence(&[])?,
        })
    }

    /// Parses `src`, turning a mistake into visible text instead of failing,
    /// so a typo in a config file shows up where the output would be.
    pub fn parse_or_message(src: &str) -> Self {
        Self::parse(src).unwrap_or_else(|e| Self {
            nodes: vec![Node::Text(format!("[format error: {e}]"))],
        })
    }

    pub fn eval(&self, fields: &dyn Fn(&str) -> Option<String>) -> String {
        eval_seq(&self.nodes, fields).text
    }
}

struct Parser<'a> {
    chars: &'a [char],
    i: usize,
}

impl Parser<'_> {
    fn error(&self, msg: &str) -> String {
        format!("{msg} at column {}", self.i + 1)
    }

    /// Parses until end of input or one of `stops` (not consumed).
    fn sequence(&mut self, stops: &[char]) -> Result<Vec<Node>, String> {
        let mut nodes = Vec::new();
        let mut text = String::new();
        while let Some(&c) = self.chars.get(self.i) {
            if stops.contains(&c) {
                break;
            }
            match c {
                '%' => {
                    flush(&mut text, &mut nodes);
                    self.i += 1;
                    let start = self.i;
                    while self.chars.get(self.i).is_some_and(|c| *c != '%') {
                        self.i += 1;
                    }
                    if self.i >= self.chars.len() {
                        self.i = start - 1;
                        return Err(self.error("field is missing its closing '%'"));
                    }
                    let name: String = self.chars[start..self.i].iter().collect();
                    self.i += 1;
                    nodes.push(Node::Field(name.trim().to_lowercase()));
                }
                '\'' => {
                    self.i += 1;
                    // '' is an escaped quote.
                    if self.chars.get(self.i) == Some(&'\'') {
                        text.push('\'');
                        self.i += 1;
                        continue;
                    }
                    let start = self.i;
                    while self.chars.get(self.i).is_some_and(|c| *c != '\'') {
                        self.i += 1;
                    }
                    if self.i >= self.chars.len() {
                        self.i = start - 1;
                        return Err(self.error("quoted text is missing its closing '"));
                    }
                    text.extend(&self.chars[start..self.i]);
                    self.i += 1;
                }
                '[' => {
                    flush(&mut text, &mut nodes);
                    let open = self.i;
                    self.i += 1;
                    let inner = self.sequence(&[']'])?;
                    if self.chars.get(self.i) != Some(&']') {
                        self.i = open;
                        return Err(self.error("'[' is missing its closing ']'"));
                    }
                    self.i += 1;
                    nodes.push(Node::Optional(inner));
                }
                '$' => {
                    flush(&mut text, &mut nodes);
                    let dollar = self.i;
                    self.i += 1;
                    let start = self.i;
                    while self.chars.get(self.i).is_some_and(|c| c.is_alphanumeric() || *c == '_') {
                        self.i += 1;
                    }
                    let name: String = self.chars[start..self.i].iter().collect();
                    if name.is_empty() || self.chars.get(self.i) != Some(&'(') {
                        self.i = dollar;
                        return Err(self.error("'$' must start a function like $if(...)"));
                    }
                    self.i += 1;
                    let mut args = Vec::new();
                    loop {
                        args.push(self.sequence(&[',', ')'])?);
                        match self.chars.get(self.i) {
                            Some(',') => self.i += 1,
                            Some(')') => {
                                self.i += 1;
                                break;
                            }
                            _ => {
                                self.i = dollar;
                                return Err(self.error(&format!("${name}( is missing its closing ')'")));
                            }
                        }
                    }
                    nodes.push(Node::Call(name.to_lowercase(), args));
                }
                _ => {
                    text.push(c);
                    self.i += 1;
                }
            }
        }
        flush(&mut text, &mut nodes);
        Ok(nodes)
    }
}

fn flush(text: &mut String, nodes: &mut Vec<Node>) {
    if !text.is_empty() {
        nodes.push(Node::Text(std::mem::take(text)));
    }
}

fn eval_seq(nodes: &[Node], fields: &dyn Fn(&str) -> Option<String>) -> Value {
    let mut out = Value {
        text: String::new(),
        truthy: false,
    };
    for node in nodes {
        let v = eval(node, fields);
        out.text += &v.text;
        out.truthy |= v.truthy;
    }
    out
}

fn eval(node: &Node, fields: &dyn Fn(&str) -> Option<String>) -> Value {
    match node {
        Node::Text(t) => Value {
            text: t.clone(),
            truthy: false,
        },
        Node::Field(name) => {
            let text = fields(name).unwrap_or_default();
            Value {
                truthy: !text.is_empty(),
                text,
            }
        }
        Node::Optional(inner) => {
            let v = eval_seq(inner, fields);
            if v.truthy {
                v
            } else {
                Value {
                    text: String::new(),
                    truthy: false,
                }
            }
        }
        Node::Call(name, args) => call(name, args, fields),
    }
}

fn call(name: &str, args: &[Vec<Node>], fields: &dyn Fn(&str) -> Option<String>) -> Value {
    let arg = |i: usize| args.get(i).map_or(
        Value {
            text: String::new(),
            truthy: false,
        },
        |a| eval_seq(a, fields),
    );
    let text = |s: String| Value {
        truthy: !s.is_empty(),
        text: s,
    };
    let number = |v: &Value| v.text.trim().parse::<i64>().unwrap_or(0);

    match name {
        "if" => {
            if arg(0).truthy {
                arg(1)
            } else {
                arg(2)
            }
        }
        "if2" => {
            let first = arg(0);
            if first.truthy { first } else { arg(1) }
        }
        "upper" => text(arg(0).text.to_uppercase()),
        "lower" => text(arg(0).text.to_lowercase()),
        // $plural(n,word) → "word" or "words"; $plural(n,child,children) for irregular words.
        "plural" => {
            let n = number(&arg(0));
            let word = arg(1).text;
            if n == 1 {
                text(word)
            } else if args.len() > 2 {
                text(arg(2).text)
            } else {
                text(word + "s")
            }
        }
        // $num(n,width) → zero-padded number.
        "num" => {
            let width = number(&arg(1)).max(0) as usize;
            text(format!("{:0width$}", number(&arg(0))))
        }
        // $pad(text,width) → padded with spaces on the right.
        "pad" => {
            let width = number(&arg(1)).max(0) as usize;
            text(format!("{:width$}", arg(0).text))
        }
        other => text(format!("[unknown function ${other}]")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> String {
        let fields = |name: &str| match name {
            "words" => Some("12".to_owned()),
            "one" => Some("1".to_owned()),
            "theme" => Some("Paper".to_owned()),
            "empty" => Some(String::new()),
            _ => None,
        };
        Format::parse(src).unwrap().eval(&fields)
    }

    #[test]
    fn fields_and_text() {
        assert_eq!(run("%words% words"), "12 words");
        assert_eq!(run("%WORDS%"), "12");
        assert_eq!(run("'%words%' and ''"), "%words% and '");
    }

    #[test]
    fn optional_sections() {
        assert_eq!(run("[%missing% · ]%theme%"), "Paper");
        assert_eq!(run("[%theme% · ]x"), "Paper · x");
        assert_eq!(run("[%empty%!]"), "");
        assert_eq!(run("[plain text]"), "", "a section with no fields never shows");
    }

    #[test]
    fn functions() {
        assert_eq!(run("$if(%missing%,yes,no)"), "no");
        assert_eq!(run("$if(%theme%,yes,no)"), "yes");
        assert_eq!(run("$if2(%missing%,%theme%)"), "Paper");
        assert_eq!(run("$upper(%theme%)"), "PAPER");
        assert_eq!(run("%words% $plural(%words%,word)"), "12 words");
        assert_eq!(run("%one% $plural(%one%,word)"), "1 word");
        assert_eq!(run("$plural(%words%,child,children)"), "children");
        assert_eq!(run("$num(%one%,3)"), "001");
        assert_eq!(run("[$pad(%theme%,7)|]"), "Paper  |");
        assert_eq!(run("$nope(x)"), "[unknown function $nope]");
        assert_eq!(run("$if(%theme%,$upper(a$lower(B)),z)"), "AB");
    }

    #[test]
    fn errors_point_at_the_problem() {
        assert_eq!(Format::parse("ab %words").unwrap_err(), "field is missing its closing '%' at column 4");
        assert_eq!(Format::parse("[x").unwrap_err(), "'[' is missing its closing ']' at column 1");
        assert!(Format::parse("$if(a,b").is_err());
        assert_eq!(Format::parse("x ]").unwrap().eval(&|_| None), "x ]", "stray brackets are plain text");
        assert!(Format::parse_or_message("[x").eval(&|_| None).starts_with("[format error"));
    }
}
