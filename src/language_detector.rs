//! Language detection module.
//!
//! Maps file extensions to programming languages and produces
//! an aggregated language map (language → total LOC) across a repository.

use std::collections::HashMap;
use std::fmt;
use std::path::Path;

/// Supported programming/markup languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Go,
    Solidity,
    Css,
    Html,
    Toml,
    Json,
    Yaml,
    Markdown,
    Shell,
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Language::Rust => "Rust",
            Language::TypeScript => "TypeScript",
            Language::JavaScript => "JavaScript",
            Language::Python => "Python",
            Language::Go => "Go",
            Language::Solidity => "Solidity",
            Language::Css => "CSS",
            Language::Html => "HTML",
            Language::Toml => "TOML",
            Language::Json => "JSON",
            Language::Yaml => "YAML",
            Language::Markdown => "Markdown",
            Language::Shell => "Shell",
        };
        write!(f, "{name}")
    }
}

/// Detect the language of a file from its extension.
///
/// Returns `None` for unknown or extension-less files.
pub fn detect_language(path: &Path) -> Option<Language> {
    let ext = path.extension()?.to_str()?.to_lowercase();

    let lang = match ext.as_str() {
        "rs" => Language::Rust,
        "ts" | "tsx" => Language::TypeScript,
        "js" | "jsx" | "mjs" => Language::JavaScript,
        "py" => Language::Python,
        "go" => Language::Go,
        "sol" => Language::Solidity,
        "css" | "scss" | "sass" => Language::Css,
        "html" | "htm" => Language::Html,
        "toml" => Language::Toml,
        "json" => Language::Json,
        "yml" | "yaml" => Language::Yaml,
        "md" | "markdown" => Language::Markdown,
        "sh" | "bash" | "zsh" => Language::Shell,
        _ => return None,
    };

    Some(lang)
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
        assert_eq!(detect_language(&p("main.rs")), Some(Language::Rust));
    }

    #[test]
    fn detects_typescript_and_tsx() {
        assert_eq!(detect_language(&p("app.ts")), Some(Language::TypeScript));
        assert_eq!(detect_language(&p("app.tsx")), Some(Language::TypeScript));
    }

    #[test]
    fn detects_javascript_variants() {
        assert_eq!(detect_language(&p("index.js")), Some(Language::JavaScript));
        assert_eq!(detect_language(&p("index.jsx")), Some(Language::JavaScript));
        assert_eq!(detect_language(&p("mod.mjs")), Some(Language::JavaScript));
    }

    #[test]
    fn detects_python() {
        assert_eq!(detect_language(&p("server.py")), Some(Language::Python));
    }

    #[test]
    fn detects_go() {
        assert_eq!(detect_language(&p("main.go")), Some(Language::Go));
    }

    #[test]
    fn detects_solidity() {
        assert_eq!(detect_language(&p("Token.sol")), Some(Language::Solidity));
    }

    #[test]
    fn detects_css_variants() {
        assert_eq!(detect_language(&p("styles.css")), Some(Language::Css));
        assert_eq!(detect_language(&p("styles.scss")), Some(Language::Css));
        assert_eq!(detect_language(&p("styles.sass")), Some(Language::Css));
    }

    #[test]
    fn detects_html() {
        assert_eq!(detect_language(&p("index.html")), Some(Language::Html));
        assert_eq!(detect_language(&p("index.htm")), Some(Language::Html));
    }

    #[test]
    fn detects_toml() {
        assert_eq!(detect_language(&p("Cargo.toml")), Some(Language::Toml));
    }

    #[test]
    fn detects_json() {
        assert_eq!(detect_language(&p("package.json")), Some(Language::Json));
    }

    #[test]
    fn detects_yaml() {
        assert_eq!(detect_language(&p("config.yml")), Some(Language::Yaml));
        assert_eq!(detect_language(&p("config.yaml")), Some(Language::Yaml));
    }

    #[test]
    fn detects_markdown() {
        assert_eq!(detect_language(&p("README.md")), Some(Language::Markdown));
        assert_eq!(
            detect_language(&p("CONTRIBUTING.markdown")),
            Some(Language::Markdown)
        );
    }

    #[test]
    fn detects_shell() {
        assert_eq!(detect_language(&p("install.sh")), Some(Language::Shell));
        assert_eq!(detect_language(&p("install.bash")), Some(Language::Shell));
        assert_eq!(detect_language(&p("rc.zsh")), Some(Language::Shell));
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

        assert_eq!(map.get(&Language::Rust), Some(&150));
        assert_eq!(map.get(&Language::TypeScript), Some(&200));
        assert_eq!(map.get(&Language::JavaScript), Some(&80));
        assert_eq!(map.get(&Language::Css), Some(&30));
        assert!(!map.contains_key(&Language::Python)); // not present
    }

    // --- Display ---

    #[test]
    fn language_display_names() {
        assert_eq!(Language::Rust.to_string(), "Rust");
        assert_eq!(Language::TypeScript.to_string(), "TypeScript");
        assert_eq!(Language::JavaScript.to_string(), "JavaScript");
        assert_eq!(Language::Python.to_string(), "Python");
        assert_eq!(Language::Go.to_string(), "Go");
        assert_eq!(Language::Solidity.to_string(), "Solidity");
        assert_eq!(Language::Css.to_string(), "CSS");
        assert_eq!(Language::Html.to_string(), "HTML");
    }
}
