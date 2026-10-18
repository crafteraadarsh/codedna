//! Module resolution: turning an import specifier into the repository files
//! it loads.
//!
//! Each language gets its own resolver (only [`javascript`] so far; Python
//! and Rust still resolve inside [`crate::dependency_graph`]). A resolver
//! answers with an [`Outcome`]: how the specifier resolved and, when it
//! did, every file it can load. A specifier can load different files for
//! different consumers (a package's `browser` and `node` builds, its types
//! and its code), so a target carries the condition it applies under
//! instead of the resolver picking one for everybody.
//!
//! Resolution is fenced to the scanned files: a resolver only ever answers
//! with a file the scanner listed, and only reads configuration files the
//! scanner listed. The scanner does not follow symbolic links, so a
//! configuration file cannot lead resolution outside the repository.

pub mod deps;
pub mod javascript;

use crate::facts::{Certainty, Resolution};
use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use std::fmt;
use std::path::PathBuf;

/// Largest configuration file (`package.json`, `tsconfig.json`) read.
pub const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

/// One file a specifier can load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub path: PathBuf,
    /// When this file is the one loaded, if not always: the package
    /// condition that selects it (`browser`, `types`, ...), several joined
    /// with `+`.
    pub condition: Option<String>,
    /// `Direct` when configuration or the file system says so; `Inferred`
    /// when CodeDna guessed (a built `dist/x.js` mapped back to
    /// `src/x.ts` with no configuration saying so, a framework's default
    /// alias).
    pub certainty: Certainty,
}

/// What resolving one specifier found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// `Resolved` with at least one target; `Ambiguous` with every
    /// candidate; otherwise no targets.
    pub resolution: Resolution,
    pub targets: Vec<Target>,
}

impl Outcome {
    pub fn external() -> Self {
        Outcome {
            resolution: Resolution::External,
            targets: Vec::new(),
        }
    }

    pub fn unresolved() -> Self {
        Outcome {
            resolution: Resolution::Unresolved,
            targets: Vec::new(),
        }
    }

    /// `Resolved` with `targets`, or `Unresolved` when there are none.
    pub fn found(targets: Vec<Target>) -> Self {
        if targets.is_empty() {
            Self::unresolved()
        } else {
            Outcome {
                resolution: Resolution::Resolved,
                targets,
            }
        }
    }

    /// Whether the specifier led to at least one file.
    pub fn is_found(&self) -> bool {
        matches!(
            self.resolution,
            Resolution::Resolved | Resolution::Ambiguous
        ) && !self.targets.is_empty()
    }
}

/// JSON that keeps the order of object keys, which `package.json`
/// `exports` and `imports` depend on: conditions are tried in the order
/// they are written.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    pub fn parse(text: &str) -> Option<Json> {
        serde_json::from_str(text).ok()
    }

    /// Parse JSON with comments and trailing commas, as `tsconfig.json`
    /// allows.
    pub fn parse_jsonc(text: &str) -> Option<Json> {
        Self::parse(&strip_jsonc(text))
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(s) => Some(s),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn as_object(&self) -> Option<&[(String, Json)]> {
        match self {
            Json::Object(entries) => Some(entries),
            _ => None,
        }
    }

    /// The strings of an array, or a lone string as a one-item list.
    pub fn strings(&self) -> Vec<&str> {
        match self {
            Json::String(s) => vec![s.as_str()],
            Json::Array(items) => items.iter().filter_map(Json::as_str).collect(),
            _ => Vec::new(),
        }
    }
}

impl<'de> Deserialize<'de> for Json {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct JsonVisitor;

        impl<'de> Visitor<'de> for JsonVisitor {
            type Value = Json;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a JSON value")
            }

            fn visit_unit<E>(self) -> Result<Json, E> {
                Ok(Json::Null)
            }

            fn visit_none<E>(self) -> Result<Json, E> {
                Ok(Json::Null)
            }

            fn visit_bool<E>(self, v: bool) -> Result<Json, E> {
                Ok(Json::Bool(v))
            }

            fn visit_i64<E>(self, v: i64) -> Result<Json, E> {
                Ok(Json::Number(v as f64))
            }

            fn visit_u64<E>(self, v: u64) -> Result<Json, E> {
                Ok(Json::Number(v as f64))
            }

            fn visit_f64<E>(self, v: f64) -> Result<Json, E> {
                Ok(Json::Number(v))
            }

            fn visit_str<E>(self, v: &str) -> Result<Json, E> {
                Ok(Json::String(v.to_string()))
            }

            fn visit_string<E>(self, v: String) -> Result<Json, E> {
                Ok(Json::String(v))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Json, A::Error> {
                let mut items = Vec::new();
                while let Some(item) = seq.next_element()? {
                    items.push(item);
                }
                Ok(Json::Array(items))
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Json, A::Error> {
                let mut entries: Vec<(String, Json)> = Vec::new();
                while let Some((key, value)) = map.next_entry::<String, Json>()? {
                    // A repeated key keeps its first position and last value,
                    // as `JSON.parse` does.
                    match entries.iter_mut().find(|(k, _)| *k == key) {
                        Some(entry) => entry.1 = value,
                        None => entries.push((key, value)),
                    }
                }
                Ok(Json::Object(entries))
            }
        }

        deserializer.deserialize_any(JsonVisitor)
    }
}

/// Turn JSON with comments and trailing commas (as `tsconfig.json` allows)
/// into plain JSON.
pub fn strip_jsonc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                out.push(c);
                while let Some(next) = chars.next() {
                    out.push(next);
                    if next == '\\' {
                        if let Some(escaped) = chars.next() {
                            out.push(escaped);
                        }
                    } else if next == '"' {
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'/') => {
                for next in chars.by_ref() {
                    if next == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut last = ' ';
                for next in chars.by_ref() {
                    if last == '*' && next == '/' {
                        break;
                    }
                    last = next;
                }
                out.push(' ');
            }
            _ => out.push(c),
        }
    }
    drop_trailing_commas(&out)
}

/// Remove commas directly before `}` or `]`, outside strings.
fn drop_trailing_commas(json: &str) -> String {
    let chars: Vec<char> = json.chars().collect();
    let mut out = String::with_capacity(json.len());
    let mut in_string = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            if c == '\\' {
                out.push(c);
                i += 1;
                if let Some(&escaped) = chars.get(i) {
                    out.push(escaped);
                }
            } else {
                in_string = c != '"';
                out.push(c);
            }
            i += 1;
            continue;
        }
        if c == ',' {
            let next = chars[i + 1..].iter().find(|n| !n.is_whitespace());
            if matches!(next, Some('}') | Some(']')) {
                i += 1;
                continue;
            }
        }
        in_string = c == '"';
        out.push(c);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_jsonc_without_touching_strings() {
        let json = strip_jsonc("{\"a\": \"http://x/*y*/\", /* c */ \"b\": [1, 2,], // d\n}");
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["a"], "http://x/*y*/");
        assert_eq!(value["b"], serde_json::json!([1, 2]));
    }

    #[test]
    fn json_keeps_key_order() {
        let json = Json::parse(r#"{"z": 1, "a": {"y": "s", "b": null}, "z": 2}"#).unwrap();
        let keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, _)| k.as_str())
            .collect();
        assert_eq!(keys, ["z", "a"]);
        assert_eq!(json.get("z"), Some(&Json::Number(2.0)));
        let inner: Vec<&str> = json
            .get("a")
            .unwrap()
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, _)| k.as_str())
            .collect();
        assert_eq!(inner, ["y", "b"]);
    }
}
