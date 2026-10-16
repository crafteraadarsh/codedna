//! Detection rules loaded from `rules.toml`.
//!
//! The rules file is embedded at build time and parsed once on first use.
//! Contributors add languages, frameworks, databases and infrastructure by
//! editing `rules.toml`; no Rust changes are needed for a new rule.

use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

/// Raw contents of `rules.toml`, embedded into the binary.
const RULES_TOML: &str = include_str!("../rules.toml");

/// All detection rules.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    #[serde(default, rename = "language")]
    pub languages: Vec<LanguageRule>,
    #[serde(default)]
    pub npm: PackageRules,
    #[serde(default)]
    pub pypi: PackageRules,
    #[serde(default)]
    pub cargo: PackageRules,
    #[serde(default)]
    pub go: PackageRules,
    #[serde(default, rename = "file_pattern")]
    pub file_patterns: Vec<FilePatternRule>,
    #[serde(default, rename = "infra")]
    pub infrastructure: Vec<InfraRule>,
}

/// A language and the file extensions that identify it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LanguageRule {
    pub name: String,
    pub extensions: Vec<String>,
    /// Whether the language's files import other files. Data and markup
    /// languages set this to `false` so they never count as source files
    /// whose imports went unanalysed.
    #[serde(default = "yes")]
    pub imports: bool,
}

fn yes() -> bool {
    true
}

/// Package-name → label maps for one ecosystem's manifest.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageRules {
    #[serde(default)]
    pub frameworks: BTreeMap<String, String>,
    #[serde(default)]
    pub databases: BTreeMap<String, String>,
}

/// Import substrings that show a source file uses a framework.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilePatternRule {
    pub framework: String,
    pub extensions: Vec<String>,
    pub imports: Vec<String>,
}

/// File names, directories and extensions that identify a tool.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InfraRule {
    pub name: String,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub dirs: Vec<String>,
    #[serde(default)]
    pub extensions: Vec<String>,
}

impl InfraRule {
    /// Whether a file matches this rule.
    ///
    /// `path` uses `/` separators; `file_name` and `ext` come from the same
    /// path, with `ext` lowercased.
    pub fn matches(&self, path: &str, file_name: &str, ext: &str) -> bool {
        let located = (self.files.is_empty() && self.dirs.is_empty())
            || self.files.iter().any(|f| f == file_name)
            || self.dirs.iter().any(|d| {
                let rooted = format!("/{path}");
                rooted.contains(&format!("/{}/", d.trim_matches('/')))
            });
        let typed = self.extensions.is_empty() || self.extensions.iter().any(|e| e == ext);
        located && typed
    }
}

/// The parsed rules, loaded once.
pub fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| match toml::from_str(RULES_TOML) {
        Ok(rules) => rules,
        Err(e) => panic!("rules.toml is invalid: {e}"),
    })
}

/// Whether files of the named language import other files.
pub fn language_has_imports(name: &str) -> bool {
    rules()
        .languages
        .iter()
        .find(|lang| lang.name == name)
        .is_some_and(|lang| lang.imports)
}

/// Look up the language name for a lowercase file extension.
pub fn language_for_extension(ext: &str) -> Option<&'static str> {
    static INDEX: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    INDEX
        .get_or_init(|| {
            rules()
                .languages
                .iter()
                .flat_map(|lang| {
                    lang.extensions
                        .iter()
                        .map(move |ext| (ext.as_str(), lang.name.as_str()))
                })
                .collect()
        })
        .get(ext)
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn rules_file_parses() {
        let r = rules();
        assert!(!r.languages.is_empty());
        assert!(!r.npm.frameworks.is_empty());
        assert!(!r.file_patterns.is_empty());
        assert!(!r.infrastructure.is_empty());
    }

    #[test]
    fn language_extensions_are_unique_and_lowercase() {
        let mut seen = HashSet::new();
        for lang in &rules().languages {
            assert!(!lang.name.trim().is_empty(), "language with empty name");
            assert!(
                !lang.extensions.is_empty(),
                "{} has no extensions",
                lang.name
            );
            for ext in &lang.extensions {
                assert_eq!(
                    ext,
                    &ext.to_lowercase(),
                    "{}: `{ext}` must be lowercase",
                    lang.name
                );
                assert!(
                    !ext.starts_with('.'),
                    "{}: write `{}` without the dot",
                    lang.name,
                    ext.trim_start_matches('.')
                );
                assert!(
                    seen.insert(ext.clone()),
                    "extension `{ext}` is listed under two languages"
                );
            }
        }
    }

    #[test]
    fn package_labels_are_not_empty() {
        let r = rules();
        for (ecosystem, pkgs) in [
            ("npm", &r.npm),
            ("pypi", &r.pypi),
            ("cargo", &r.cargo),
            ("go", &r.go),
        ] {
            for (name, label) in pkgs.frameworks.iter().chain(pkgs.databases.iter()) {
                assert!(!name.trim().is_empty(), "{ecosystem}: empty package name");
                assert!(
                    !label.trim().is_empty(),
                    "{ecosystem}: `{name}` has an empty label"
                );
            }
        }
    }

    #[test]
    fn pypi_package_names_are_lowercase() {
        let pypi = &rules().pypi;
        for name in pypi.frameworks.keys().chain(pypi.databases.keys()) {
            assert_eq!(
                name,
                &name.to_lowercase(),
                "pypi: `{name}` must be lowercase"
            );
        }
    }

    #[test]
    fn file_patterns_are_well_formed() {
        for p in &rules().file_patterns {
            assert_eq!(
                p.framework,
                p.framework.to_lowercase(),
                "framework `{}` must be lowercase",
                p.framework
            );
            assert!(
                !p.extensions.is_empty(),
                "{} has no extensions",
                p.framework
            );
            assert!(!p.imports.is_empty(), "{} has no imports", p.framework);
            for ext in &p.extensions {
                assert_eq!(
                    ext,
                    &ext.to_lowercase(),
                    "{}: `{ext}` must be lowercase",
                    p.framework
                );
            }
        }
    }

    #[test]
    fn infra_rules_are_well_formed() {
        for rule in &rules().infrastructure {
            assert!(!rule.name.trim().is_empty(), "infra rule with empty name");
            assert!(
                !(rule.files.is_empty() && rule.dirs.is_empty() && rule.extensions.is_empty()),
                "{} would match every file",
                rule.name
            );
        }
    }

    #[test]
    fn looks_up_language_by_extension() {
        assert_eq!(language_for_extension("rs"), Some("Rust"));
        assert_eq!(language_for_extension("tsx"), Some("TypeScript"));
        assert_eq!(language_for_extension("bin"), None);
    }

    #[test]
    fn infra_rule_matching() {
        let rule = InfraRule {
            name: "Kubernetes".into(),
            files: vec![],
            dirs: vec!["k8s".into()],
            extensions: vec!["yaml".into()],
        };
        assert!(rule.matches("repo/k8s/deploy.yaml", "deploy.yaml", "yaml"));
        assert!(rule.matches("k8s/deploy.yaml", "deploy.yaml", "yaml"));
        assert!(!rule.matches("repo/k8s/README.md", "README.md", "md"));
        assert!(!rule.matches("repo/notk8s/deploy.yaml", "deploy.yaml", "yaml"));

        let by_ext = InfraRule {
            name: "Terraform".into(),
            files: vec![],
            dirs: vec![],
            extensions: vec!["tf".into()],
        };
        assert!(by_ext.matches("infra/main.tf", "main.tf", "tf"));
        assert!(!by_ext.matches("infra/main.rs", "main.rs", "rs"));
    }
}
