//! Language detection module.
//!
//! Maps file extensions to programming languages (from `rules.toml`) and produces
//! an aggregated language map (language → total LOC) across a repository.

use crate::rules;
use std::collections::HashMap;
use std::fmt;
use std::path::Path;

/// A detected programming or markup language.
///
/// Languages and their extensions are defined by the `[[language]]` rules in
/// `rules.toml`; the wrapped value is the language's display name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Language(&'static str);

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// Detect the language of a file from its extension.
///
/// Returns `None` for unknown or extension-less files.
pub fn detect_language(path: &Path) -> Option<Language> {
    let ext = path.extension()?.to_str()?.to_lowercase();
    rules::language_for_extension(&ext).map(Language)
}

/// Build a map of `Language → total LOC` from a list of (path, loc) pairs.
///
/// Only files whose extension maps to a known `Language` are included.
pub fn build_language_map(files: &[(std::path::PathBuf, usize)]) -> HashMap<Language, usize> {
    let mut map: HashMap<Language, usize> = HashMap::new();

    for (path, loc) in files {
        if let Some(lang) = detect_language(path) {
            *map.entry(lang).or_insert(0) += loc;
        }
    }

    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn p(name: &str) -> PathBuf {
        PathBuf::from(name)
    }

    // --- detect_language ---

    #[test]
    fn detects_rust() {
        assert_eq!(detect_language(&p("main.rs")), Some(Language("Rust")));
    }

    #[test]
    fn detects_typescript_and_tsx() {
        assert_eq!(detect_language(&p("app.ts")), Some(Language("TypeScript")));
        assert_eq!(detect_language(&p("app.tsx")), Some(Language("TypeScript")));
    }

    #[test]
    fn detects_javascript_variants() {
        assert_eq!(
            detect_language(&p("index.js")),
            Some(Language("JavaScript"))
        );
        assert_eq!(
            detect_language(&p("index.jsx")),
            Some(Language("JavaScript"))
        );
        assert_eq!(detect_language(&p("mod.mjs")), Some(Language("JavaScript")));
    }

    #[test]
    fn detects_python() {
        assert_eq!(detect_language(&p("server.py")), Some(Language("Python")));
    }

    #[test]
    fn detects_go() {
        assert_eq!(detect_language(&p("main.go")), Some(Language("Go")));
    }

    #[test]
    fn detects_solidity() {
        assert_eq!(detect_language(&p("Token.sol")), Some(Language("Solidity")));
    }

    #[test]
    fn detects_css_variants() {
        assert_eq!(detect_language(&p("styles.css")), Some(Language("CSS")));
        assert_eq!(detect_language(&p("styles.scss")), Some(Language("CSS")));
        assert_eq!(detect_language(&p("styles.sass")), Some(Language("CSS")));
    }

    #[test]
    fn detects_html() {
        assert_eq!(detect_language(&p("index.html")), Some(Language("HTML")));
        assert_eq!(detect_language(&p("index.htm")), Some(Language("HTML")));
    }

    #[test]
    fn detects_toml() {
        assert_eq!(detect_language(&p("Cargo.toml")), Some(Language("TOML")));
    }

    #[test]
    fn detects_json() {
        assert_eq!(detect_language(&p("package.json")), Some(Language("JSON")));
    }

    #[test]
    fn detects_yaml() {
        assert_eq!(detect_language(&p("config.yml")), Some(Language("YAML")));
        assert_eq!(detect_language(&p("config.yaml")), Some(Language("YAML")));
    }

    #[test]
    fn detects_markdown() {
        assert_eq!(detect_language(&p("README.md")), Some(Language("Markdown")));
        assert_eq!(
            detect_language(&p("CONTRIBUTING.markdown")),
            Some(Language("Markdown"))
        );
    }

    #[test]
    fn detects_shell() {
        assert_eq!(detect_language(&p("install.sh")), Some(Language("Shell")));
        assert_eq!(detect_language(&p("install.bash")), Some(Language("Shell")));
        assert_eq!(detect_language(&p("rc.zsh")), Some(Language("Shell")));
    }

    #[test]
    fn returns_none_for_unknown_extension() {
        assert_eq!(detect_language(&p("binary.bin")), None);
        assert_eq!(detect_language(&p("data.dat")), None);
    }

    #[test]
    fn returns_none_for_file_with_no_extension() {
        assert_eq!(detect_language(&p("Makefile")), None);
        assert_eq!(detect_language(&p("Dockerfile")), None);
    }

    // --- build_language_map ---

    #[test]
    fn builds_aggregated_language_map() {
        let files = vec![
            (p("src/main.rs"), 100),
            (p("src/lib.rs"), 50),
            (p("src/app.ts"), 200),
            (p("src/index.js"), 80),
            (p("assets/style.css"), 30),
            (p("binary.bin"), 99), // unknown — should be excluded
        ];

        let map = build_language_map(&files);

        assert_eq!(map.get(&Language("Rust")), Some(&150));
        assert_eq!(map.get(&Language("TypeScript")), Some(&200));
        assert_eq!(map.get(&Language("JavaScript")), Some(&80));
        assert_eq!(map.get(&Language("CSS")), Some(&30));
        assert!(!map.contains_key(&Language("Python"))); // not present
    }

    // --- Display ---

    #[test]
    fn language_display_names() {
        assert_eq!(Language("Rust").to_string(), "Rust");
        assert_eq!(Language("TypeScript").to_string(), "TypeScript");
        assert_eq!(Language("JavaScript").to_string(), "JavaScript");
        assert_eq!(Language("Python").to_string(), "Python");
        assert_eq!(Language("Go").to_string(), "Go");
        assert_eq!(Language("Solidity").to_string(), "Solidity");
        assert_eq!(Language("CSS").to_string(), "CSS");
        assert_eq!(Language("HTML").to_string(), "HTML");
    }
}
