//! A small, bounded YAML reader.
//!
//! Deployment files come from repositories CodeDna does not trust, so the
//! tree is built from parser events with limits: nesting deeper than
//! [`MAX_DEPTH`] is dropped, and aliases stop expanding once a file has
//! produced [`MAX_NODES`] nodes, which defuses "billion laughs" files. Every
//! node keeps its 1-based line. Tags such as CloudFormation's `!Ref` are
//! ignored and their value kept. Documents read before a syntax error are
//! still returned.

use std::collections::HashMap;
use yaml_rust2::parser::{Event, MarkedEventReceiver, Parser};
use yaml_rust2::scanner::{Marker, TScalarStyle};

/// Deepest nesting kept.
const MAX_DEPTH: usize = 64;
/// Most nodes built from one file, aliases included.
const MAX_NODES: usize = 200_000;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Str(String),
    Seq(Vec<Node>),
    Map(Vec<(String, Node)>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub line: usize,
    pub value: Value,
}

impl Node {
    /// Value under `key` when this is a mapping.
    pub fn get(&self, key: &str) -> Option<&Node> {
        self.entries()
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    /// Value under a chain of keys.
    pub fn at(&self, keys: &[&str]) -> Option<&Node> {
        keys.iter().try_fold(self, |node, key| node.get(key))
    }

    /// Text of a scalar.
    pub fn str(&self) -> Option<&str> {
        match &self.value {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Text of a scalar under `key`.
    pub fn str_at(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Node::str)
    }

    /// A scalar, or a list of scalars joined with spaces (how commands are
    /// often written).
    pub fn words(&self) -> Option<String> {
        match &self.value {
            Value::Str(s) => Some(s.clone()),
            Value::Seq(items) => {
                let words: Vec<&str> = items.iter().filter_map(Node::str).collect();
                (!words.is_empty()).then(|| words.join(" "))
            }
            _ => None,
        }
    }

    /// Items of a sequence; empty for anything else.
    pub fn items(&self) -> &[Node] {
        match &self.value {
            Value::Seq(items) => items,
            _ => &[],
        }
    }

    /// Entries of a mapping; empty for anything else.
    pub fn entries(&self) -> &[(String, Node)] {
        match &self.value {
            Value::Map(entries) => entries,
            _ => &[],
        }
    }
}

/// Every document in `text`, up to the first syntax error.
pub fn load(text: &str) -> Vec<Node> {
    let mut builder = Builder::default();
    let mut parser = Parser::new_from_str(text);
    let _ = parser.load(&mut builder, true);
    if let Some(root) = builder.root.take() {
        builder.docs.push(root);
    }
    builder.docs
}

enum Frame {
    Seq {
        line: usize,
        anchor: usize,
        size: usize,
        items: Vec<Node>,
    },
    Map {
        line: usize,
        anchor: usize,
        size: usize,
        entries: Vec<(String, Node)>,
        key: Option<String>,
    },
}

#[derive(Default)]
struct Builder {
    stack: Vec<Frame>,
    /// Containers being skipped because they nest too deeply.
    skipping: usize,
    anchors: HashMap<usize, (Node, usize)>,
    nodes: usize,
    root: Option<Node>,
    docs: Vec<Node>,
}

impl Builder {
    fn add(&mut self, node: Node, size: usize, anchor: usize) {
        if anchor > 0 {
            self.anchors.insert(anchor, (node.clone(), size));
        }
        match self.stack.last_mut() {
            None => self.root = Some(node),
            Some(Frame::Seq {
                items, size: total, ..
            }) => {
                *total += size;
                items.push(node);
            }
            Some(Frame::Map {
                entries,
                key,
                size: total,
                ..
            }) => {
                *total += size;
                match key.take() {
                    None => {
                        *key = Some(match node.value {
                            Value::Str(s) => s,
                            // Complex keys are not used by deployment files.
                            _ => String::new(),
                        })
                    }
                    Some(k) => entries.push((k, node)),
                }
            }
        }
    }

    fn open(&mut self, frame: Frame) {
        if self.skipping > 0 || self.stack.len() >= MAX_DEPTH {
            self.skipping += 1;
        } else {
            self.stack.push(frame);
        }
    }

    fn close(&mut self) {
        if self.skipping > 0 {
            self.skipping -= 1;
            // Keep the parent's key/value pairing intact.
            if self.skipping == 0 {
                self.add(
                    Node {
                        line: 0,
                        value: Value::Null,
                    },
                    1,
                    0,
                );
            }
            return;
        }
        let (node, size, anchor) = match self.stack.pop() {
            Some(Frame::Seq {
                line,
                anchor,
                size,
                items,
            }) => (
                Node {
                    line,
                    value: Value::Seq(items),
                },
                size + 1,
                anchor,
            ),
            Some(Frame::Map {
                line,
                anchor,
                size,
                entries,
                ..
            }) => (
                Node {
                    line,
                    value: Value::Map(merge(entries)),
                },
                size + 1,
                anchor,
            ),
            None => return,
        };
        self.add(node, size, anchor);
    }
}

/// Apply `<<` merge keys: merged entries never override explicit ones.
fn merge(entries: Vec<(String, Node)>) -> Vec<(String, Node)> {
    if !entries.iter().any(|(k, _)| k == "<<") {
        return entries;
    }
    let mut own = Vec::new();
    let mut merged = Vec::new();
    for (k, v) in entries {
        if k != "<<" {
            own.push((k, v));
            continue;
        }
        let sources = match v.value {
            Value::Seq(items) => items,
            other => vec![Node {
                line: v.line,
                value: other,
            }],
        };
        for source in sources {
            if let Value::Map(pairs) = source.value {
                merged.extend(pairs);
            }
        }
    }
    for (k, v) in merged {
        if !own.iter().any(|(o, _)| *o == k) {
            own.push((k, v));
        }
    }
    own
}

impl MarkedEventReceiver for Builder {
    fn on_event(&mut self, ev: Event, mark: Marker) {
        let line = mark.line();
        match ev {
            Event::DocumentEnd => {
                if let Some(root) = self.root.take() {
                    self.docs.push(root);
                }
                self.anchors.clear();
            }
            Event::SequenceStart(anchor, _) => self.open(Frame::Seq {
                line,
                anchor,
                size: 0,
                items: Vec::new(),
            }),
            Event::MappingStart(anchor, _) => self.open(Frame::Map {
                line,
                anchor,
                size: 0,
                entries: Vec::new(),
                key: None,
            }),
            Event::SequenceEnd | Event::MappingEnd => self.close(),
            _ if self.skipping > 0 => {}
            Event::Scalar(text, style, anchor, _) => {
                self.nodes += 1;
                let plain = style == TScalarStyle::Plain;
                let value = if plain && matches!(text.as_str(), "" | "~" | "null" | "Null" | "NULL")
                {
                    Value::Null
                } else {
                    Value::Str(text)
                };
                self.add(Node { line, value }, 1, anchor);
            }
            Event::Alias(id) => {
                let found = self.anchors.get(&id).cloned();
                match found {
                    Some((node, size)) if self.nodes + size <= MAX_NODES => {
                        self.nodes += size;
                        self.add(node, size, 0);
                    }
                    _ => self.add(
                        Node {
                            line,
                            value: Value::Null,
                        },
                        1,
                        0,
                    ),
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_documents_with_lines() {
        let docs = load("kind: Deployment\nmetadata:\n  name: api\n---\nkind: Service\n");
        assert_eq!(docs.len(), 2);
        assert_eq!(
            docs[0].at(&["metadata", "name"]).unwrap().str(),
            Some("api")
        );
        assert_eq!(docs[0].get("metadata").unwrap().line, 3);
        assert_eq!(docs[1].str_at("kind"), Some("Service"));
    }

    #[test]
    fn ignores_tags_and_reads_words() {
        let docs = load("Role: !GetAtt R.Arn\ncommand: [\"node\", \"server.js\"]\n");
        assert_eq!(docs[0].str_at("Role"), Some("R.Arn"));
        assert_eq!(
            docs[0].get("command").unwrap().words().as_deref(),
            Some("node server.js")
        );
    }

    #[test]
    fn applies_merge_keys() {
        let docs = load("base: &b\n  image: x\n  port: 1\nweb:\n  <<: *b\n  port: 2\n");
        let web = docs[0].get("web").unwrap();
        assert_eq!(web.str_at("image"), Some("x"));
        assert_eq!(web.str_at("port"), Some("2"));
    }

    #[test]
    fn alias_bombs_stop_expanding() {
        let mut text = String::from("a0: &a0 [x, x, x, x, x, x, x, x, x, x]\n");
        for i in 1..12 {
            let p = i - 1;
            text.push_str(&format!(
                "a{i}: &a{i} [*a{p}, *a{p}, *a{p}, *a{p}, *a{p}, *a{p}, *a{p}, *a{p}, *a{p}, *a{p}]\n"
            ));
        }
        text.push_str("tail: ok\n");
        let docs = load(&text);
        assert_eq!(docs[0].str_at("tail"), Some("ok"));
    }

    #[test]
    fn deep_nesting_is_dropped_not_overflowed() {
        let text = format!("a: {}{}\nb: ok\n", "[".repeat(200), "]".repeat(200));
        let docs = load(&text);
        assert_eq!(docs[0].str_at("b"), Some("ok"));
        // Far deeper input is refused by the parser instead of overflowing.
        let text = format!("a: {}{}\n", "[".repeat(100_000), "]".repeat(100_000));
        let _ = load(&text);
    }

    #[test]
    fn keeps_documents_before_an_error() {
        let docs = load("kind: Pod\n---\nkey: [unclosed\n");
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].str_at("kind"), Some("Pod"));
    }
}
