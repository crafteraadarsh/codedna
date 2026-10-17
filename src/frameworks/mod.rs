//! Framework adapters: the files a framework, a package manifest or a test
//! runner loads without anything importing them.
//!
//! Each adapter is active per package, decided by that package's own
//! `package.json` (and, for test runners, its own config file), never by a
//! dependency somewhere else in the repository: `next` in
//! `examples/blog/package.json` makes `examples/blog/app/` a router and
//! nothing at the root. An active adapter names each file it loads as a
//! [`DependencyKind::ConfiguredEntry`] fact whose source and evidence are the
//! manifest line declaring the framework, with a versioned rule such as
//! `next.app_router@v1`. Dead-code detection starts from those files.
//!
//! Adapters read declarations and literal config values only. Config files
//! are code, so a value computed at runtime is not seen; the name-based
//! heuristics in [`crate::dead_code_detector`] stay as the fallback.

mod expo;
mod next;
mod node;
mod testing;

use crate::dependency_graph::normalize;
use crate::facts::{Certainty, DependencyFact, DependencyKind, FrameworkAdapter};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// What the adapters found in a repository.
#[derive(Debug, Default)]
pub struct Adapters {
    /// One `configured_entry` fact per file an adapter loads.
    pub entries: Vec<DependencyFact>,
    /// The adapters active in each package.
    pub active: Vec<FrameworkAdapter>,
    /// Directories where the Next.js adapter decides which files are
    /// routes, so the name heuristics for Next files do not apply there.
    pub next_roots: Vec<PathBuf>,
}

impl Adapters {
    /// Every file an adapter names, for dead-code detection.
    pub fn entry_files(&self) -> Vec<PathBuf> {
        self.entries
            .iter()
            .flat_map(|f| f.targets.iter().cloned())
            .collect()
    }
}

/// Run every adapter over the scanned files.
pub fn detect(files: &[PathBuf]) -> Adapters {
    let repo = Repo::new(files);
    let mut out = Adapters::default();
    for package in &repo.packages {
        node::detect(&repo, package, &mut out);
        next::detect(&repo, package, &mut out);
        expo::detect(&repo, package, &mut out);
        testing::detect(&repo, package, &mut out);
    }
    crate::facts::sort(&mut out.entries);
    out.entries.dedup();
    out.active
        .sort_by(|a, b| (&a.package, &a.adapter).cmp(&(&b.package, &b.adapter)));
    out
}

/// The scanned files, and the packages they belong to.
pub(crate) struct Repo {
    packages: Vec<Package>,
    /// Normalised path → the path as scanned.
    by_path: HashMap<PathBuf, PathBuf>,
}

/// A directory with a `package.json`, and the files it owns: those below
/// it that no nested package owns.
pub(crate) struct Package {
    pub dir: PathBuf,
    pub manifest: PathBuf,
    pub text: String,
    pub json: serde_json::Value,
    pub files: Vec<PathBuf>,
}

/// The manifest sections whose packages a package itself uses. Peer
/// dependencies are provided by whoever installs it.
const OWN_DEPENDENCIES: &[&str] = &["dependencies", "devDependencies"];

impl Repo {
    fn new(files: &[PathBuf]) -> Repo {
        let by_path: HashMap<PathBuf, PathBuf> =
            files.iter().map(|f| (normalize(f), f.clone())).collect();
        let mut packages: Vec<Package> = files
            .iter()
            .filter(|f| f.file_name().is_some_and(|n| n == "package.json"))
            .filter_map(|manifest| {
                let text = fs::read_to_string(manifest).ok()?;
                let json = serde_json::from_str(&text).ok()?;
                Some(Package {
                    dir: manifest.parent()?.to_path_buf(),
                    manifest: manifest.clone(),
                    text,
                    json,
                    files: Vec::new(),
                })
            })
            .collect();
        let index: HashMap<PathBuf, usize> = packages
            .iter()
            .enumerate()
            .map(|(i, p)| (p.dir.clone(), i))
            .collect();
        for file in files {
            if let Some(i) = file.ancestors().skip(1).find_map(|dir| index.get(dir)) {
                packages[*i].files.push(file.clone());
            }
        }
        packages.sort_by(|a, b| a.dir.cmp(&b.dir));
        Repo { packages, by_path }
    }

    /// The scanned file at `path`, if any.
    pub fn file(&self, path: &Path) -> Option<&PathBuf> {
        self.by_path.get(&normalize(path))
    }

    /// The scanned file a manifest or config names: the path itself, or
    /// the path with a JavaScript extension or `/index` added.
    pub fn resolve(&self, base: &Path, path: &str) -> Option<PathBuf> {
        let path = path.trim();
        if path.is_empty() || path.starts_with('/') || path.contains("://") {
            return None;
        }
        let joined = base.join(path);
        if let Some(found) = self.file(&joined) {
            return Some(found.clone());
        }
        for ext in JS_EXTENSIONS {
            let mut with = joined.as_os_str().to_owned();
            with.push(format!(".{ext}"));
            if let Some(found) = self.file(Path::new(&with)) {
                return Some(found.clone());
            }
            if let Some(found) = self.file(&joined.join(format!("index.{ext}"))) {
                return Some(found.clone());
            }
        }
        None
    }
}

impl Package {
    /// The line declaring `name` in `dependencies` or `devDependencies`,
    /// if the package uses it itself. `Some(None)` when declared but the
    /// line cannot be told.
    pub fn depends_on(&self, name: &str) -> Option<Option<usize>> {
        OWN_DEPENDENCIES.iter().find_map(|section| {
            self.json.get(section)?.get(name)?;
            Some(declaration_line(&self.text, section, name))
        })
    }

    /// The first of `names` this package depends on, with its line.
    pub fn depends_on_any<'a>(&self, names: &[&'a str]) -> Option<(&'a str, Option<usize>)> {
        names
            .iter()
            .find_map(|name| self.depends_on(name).map(|line| (*name, line)))
    }

    /// The package's own file named `name` directly in `dir`.
    pub fn file_in(&self, dir: &Path, name: &str) -> Option<&PathBuf> {
        self.files
            .iter()
            .find(|f| f.parent() == Some(dir) && f.file_name().is_some_and(|n| n == name))
    }

    /// The first of `names` present in the package directory.
    pub fn config(&self, names: &[&str]) -> Option<&PathBuf> {
        names.iter().find_map(|name| self.file_in(&self.dir, name))
    }

    /// `file` relative to the package, with forward slashes.
    pub fn relative(&self, file: &Path) -> String {
        relative(&self.dir, file)
    }

    /// A `configured_entry` fact: the manifest names `target` through
    /// `rule`, seen on `line`.
    pub fn entry(
        &self,
        target: &Path,
        rule: &str,
        certainty: Certainty,
        line: Option<usize>,
    ) -> DependencyFact {
        let mut fact = DependencyFact::new(
            &self.manifest,
            &self.relative(target),
            DependencyKind::ConfiguredEntry,
            certainty,
            rule,
        )
        .resolved(vec![target.to_path_buf()]);
        fact.evidence.line = line;
        fact
    }

    /// Record that `adapter` is active here.
    pub fn adapter(
        &self,
        adapter: &str,
        version: u32,
        line: Option<usize>,
        config: Vec<PathBuf>,
    ) -> FrameworkAdapter {
        FrameworkAdapter {
            package: self.dir.clone(),
            adapter: adapter.to_string(),
            version,
            manifest: self.manifest.clone(),
            line,
            config,
        }
    }
}

/// Extensions of JavaScript and TypeScript modules.
pub(crate) const JS_EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"];

pub(crate) fn is_js(file: &Path) -> bool {
    file.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| JS_EXTENSIONS.contains(&e))
}

/// `file` relative to `dir`, with forward slashes.
pub(crate) fn relative(dir: &Path, file: &Path) -> String {
    file.strip_prefix(dir)
        .unwrap_or(file)
        .iter()
        .filter_map(|c| c.to_str())
        .filter(|c| *c != ".")
        .collect::<Vec<_>>()
        .join("/")
}

/// The line of `"name"` inside the `section` object of a manifest's text.
fn declaration_line(text: &str, section: &str, name: &str) -> Option<usize> {
    let start = text.find(&format!("\"{section}\""))?;
    let at = start + text[start..].find(&format!("\"{name}\""))?;
    Some(text[..at].matches('\n').count() + 1)
}

/// The literal strings a JavaScript or TypeScript config gives `key`:
/// `key: './a.ts'` or `key: ['./a.ts', './b.ts']`, every time the key is
/// written. `None` when the key is absent; an empty list when it is set to
/// something computed, which cannot be read statically.
pub(crate) fn config_strings(text: &str, key: &str) -> Option<Vec<String>> {
    let mut found = None;
    let bytes = text.as_bytes();
    let mut from = 0;
    while let Some(pos) = text[from..].find(key) {
        let start = from + pos;
        from = start + key.len();
        let before = start.checked_sub(1).map(|i| bytes[i]);
        // A whole property name, quoted or not.
        if before.is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'$') {
            continue;
        }
        let mut rest = &text[from..];
        if let Some(quote) = before.filter(|b| matches!(b, b'"' | b'\'')) {
            if !rest.starts_with(quote as char) {
                continue;
            }
            rest = &rest[1..];
        }
        let Some(value) = rest.trim_start().strip_prefix(':') else {
            continue;
        };
        let list = found.get_or_insert_with(Vec::new);
        list.extend(literals(value.trim_start()));
    }
    found
}

/// The string literals at the start of `value`: one string, or the strings
/// of an array (skipping computed items).
fn literals(value: &str) -> Vec<String> {
    if let Some(array) = value.strip_prefix('[') {
        let end = array.find(']').unwrap_or(array.len());
        let mut out = Vec::new();
        let mut rest = &array[..end];
        while let Some((s, after)) = next_literal(rest) {
            out.push(s);
            rest = after;
        }
        return out;
    }
    match value.chars().next() {
        Some('"' | '\'' | '`') => next_literal(value)
            .map(|(s, _)| vec![s])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// The next whole string literal in `text`, and what follows it. Template
/// literals with `${` are computed and skipped.
fn next_literal(text: &str) -> Option<(String, &str)> {
    let start = text.find(['"', '\'', '`'])?;
    let quote = text[start..].chars().next()?;
    let body = &text[start + 1..];
    let end = body.find(quote)?;
    let s = &body[..end];
    let after = &body[end + 1..];
    if quote == '`' && s.contains("${") || s.contains('\n') {
        return next_literal(after);
    }
    Some((s.to_string(), after))
}

/// Most alternatives one glob may expand to; braces beyond that are not
/// expanded and the pattern is not used.
const MAX_ALTERNATIVES: usize = 256;

/// A test-file glob as runners write them: `**`, `*`, `?`, `[jt]` and
/// `{a,b}`. `None` for what it does not understand (extglobs such as
/// `?(x)`, negations), which then is not used.
pub(crate) struct Glob {
    alternatives: Vec<Vec<String>>,
}

impl Glob {
    pub fn new(pattern: &str) -> Option<Glob> {
        let pattern = pattern.trim().trim_start_matches("./");
        if pattern.is_empty()
            || pattern.starts_with('!')
            || ["?(", "*(", "+(", "@(", "!("]
                .iter()
                .any(|ext| pattern.contains(ext))
        {
            return None;
        }
        let mut alternatives = Vec::new();
        expand(pattern, &mut alternatives)?;
        Some(Glob {
            alternatives: alternatives
                .into_iter()
                .map(|alt| {
                    alt.split('/')
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .collect(),
        })
    }

    /// Whether the relative path `path` (forward slashes) matches.
    pub fn matches(&self, path: &str) -> bool {
        let segments: Vec<&str> = path.split('/').collect();
        self.alternatives.iter().any(|alt| {
            let alt: Vec<&str> = alt.iter().map(String::as_str).collect();
            match_segments(&alt, &segments)
        })
    }
}

/// Expand `{a,b}` groups; `None` past [`MAX_ALTERNATIVES`].
fn expand(pattern: &str, out: &mut Vec<String>) -> Option<()> {
    let Some(open) = pattern.find('{') else {
        out.push(pattern.to_string());
        return (out.len() <= MAX_ALTERNATIVES).then_some(());
    };
    let mut depth = 0;
    let mut close = None;
    let mut commas = Vec::new();
    for (i, c) in pattern[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + i);
                    break;
                }
            }
            ',' if depth == 1 => commas.push(open + i),
            _ => {}
        }
    }
    let close = close?;
    let (head, tail) = (&pattern[..open], &pattern[close + 1..]);
    let mut bounds = vec![open];
    bounds.extend(commas);
    bounds.push(close);
    for pair in bounds.windows(2) {
        let choice = &pattern[pair[0] + 1..pair[1]];
        expand(&format!("{head}{choice}{tail}"), out)?;
    }
    Some(())
}

fn match_segments(pattern: &[&str], segments: &[&str]) -> bool {
    match pattern.split_first() {
        None => segments.is_empty(),
        Some((&"**", rest)) => {
            (0..=segments.len()).any(|skip| match_segments(rest, &segments[skip..]))
        }
        Some((first, rest)) => match segments.split_first() {
            Some((segment, others)) => match_one(first, segment) && match_segments(rest, others),
            None => false,
        },
    }
}

/// One path segment against `*`, `?` and `[...]`.
fn match_one(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    fn class(p: &[char], c: char) -> Option<(bool, usize)> {
        // `p` starts after `[`; returns whether `c` matched and the length
        // through `]`.
        let negate = matches!(p.first(), Some('!' | '^'));
        let mut i = usize::from(negate);
        let mut hit = false;
        while i < p.len() && (p[i] != ']' || i == usize::from(negate)) {
            if i + 2 < p.len() && p[i + 1] == '-' && p[i + 2] != ']' {
                hit |= p[i] <= c && c <= p[i + 2];
                i += 3;
            } else {
                hit |= p[i] == c;
                i += 1;
            }
        }
        (i < p.len()).then_some((hit != negate, i + 1))
    }
    fn go(p: &[char], n: &[char]) -> bool {
        match p.first() {
            None => n.is_empty(),
            Some('*') => (0..=n.len()).any(|k| go(&p[1..], &n[k..])),
            Some('?') => !n.is_empty() && go(&p[1..], &n[1..]),
            Some('[') => match (n.first(), class(&p[1..], *n.first().unwrap_or(&'\0'))) {
                (Some(_), Some((true, len))) => go(&p[1 + len..], &n[1..]),
                (Some(c), None) => *c == '[' && go(&p[1..], &n[1..]),
                _ => false,
            },
            Some(c) => n.first() == Some(c) && go(&p[1..], &n[1..]),
        }
    }
    go(&p, &n)
}

/// Directories of the package's files, so adapters can ask whether a
/// folder exists.
pub(crate) fn has_dir(package: &Package, dir: &Path) -> bool {
    package.files.iter().any(|f| f.starts_with(dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs_match_like_test_runners() {
        let g = Glob::new("**/*.{test,spec}.{js,ts}").unwrap();
        assert!(g.matches("src/a.test.ts"));
        assert!(g.matches("a.spec.js"));
        assert!(!g.matches("src/a.ts"));
        let g = Glob::new("./test/*.[jt]s").unwrap();
        assert!(g.matches("test/a.js"));
        assert!(!g.matches("test/deep/a.js"));
        assert!(Glob::new("**/?(*.)+(spec|test).js").is_none());
        assert!(Glob::new("!**/node_modules/**").is_none());
    }

    #[test]
    fn config_strings_reads_literals_only() {
        let text = "export default {\n  test: { setupFiles: ['./a.ts', helper, \"./b.ts\"],\n globalSetup: './g.ts', 'include': [`x/**`] }\n}";
        assert_eq!(
            config_strings(text, "setupFiles").unwrap(),
            ["./a.ts", "./b.ts"]
        );
        assert_eq!(config_strings(text, "globalSetup").unwrap(), ["./g.ts"]);
        assert_eq!(config_strings(text, "include").unwrap(), ["x/**"]);
        assert_eq!(
            config_strings("{ setupFiles: files }", "setupFiles").unwrap(),
            Vec::<String>::new()
        );
        assert!(config_strings(text, "setup").is_none());
        assert!(config_strings("{ mySetupFiles: ['a'] }", "SetupFiles").is_none());
    }

    #[test]
    fn declaration_line_finds_the_dependency() {
        let text = "{\n \"devDependencies\": {\n  \"a\": \"1\"\n },\n \"dependencies\": {\n  \"next\": \"15\"\n }\n}";
        assert_eq!(declaration_line(text, "dependencies", "next"), Some(6));
    }
}
