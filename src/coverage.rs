//! Coverage: how much of the repository CodeDna could actually analyze.
//!
//! An empty dead-code list means little if half the code is in a language
//! whose imports CodeDna does not follow, or if many imports never resolved.
//! This module says which source files had their imports followed, which
//! imports of the repository's own code matched no file, and which
//! languages were not analyzed at all, so "nothing found" can be told apart
//! from "could not look".

use crate::architecture::Location;
use crate::dependency_graph::{follows_imports, ImportTally};
use crate::rules;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// Most unresolved imports listed by file and line.
const MAX_UNRESOLVED: usize = 20;

/// How much of the repository the dependency analysis covered.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coverage {
    /// Files in a language that imports other files.
    pub source_files: usize,
    /// Source files whose imports were followed.
    pub analyzed_files: usize,
    /// Source files in a followed language that could not be read.
    #[serde(default)]
    pub unreadable_files: usize,
    /// Languages whose imports CodeDna does not follow, with file counts,
    /// most files first. Dead code and cycles say nothing about these files.
    #[serde(default)]
    pub not_analyzed: Vec<LanguageGap>,
    /// What happened to the imports in the analyzed files.
    #[serde(default)]
    pub imports: ImportCounts,
    /// Imports of the repository's own code that matched no file, first
    /// few by path and line.
    #[serde(default)]
    pub unresolved: Vec<UnresolvedImport>,
    /// One sentence for people, e.g. "Imports followed in 120 of 140 source
    /// files (85%); 3 of 410 internal imports unresolved; not analyzed:
    /// Go (20 files)".
    pub summary: String,
}

/// A language CodeDna does not follow imports for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageGap {
    pub language: String,
    pub files: usize,
}

/// Import counts across the analyzed files. Each distinct import in a file
/// counts once.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportCounts {
    /// Imports of the repository's own files: `resolved + unresolved`.
    pub internal: usize,
    pub resolved: usize,
    pub unresolved: usize,
    /// Imports of packages outside the repository.
    pub external: usize,
}

/// One import of the repository's own code that matched no file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvedImport {
    /// Path relative to the repository root.
    pub file: String,
    /// 1-based line number.
    pub line: usize,
    /// The import as written, e.g. `./missing` or `@app/core`.
    pub import: String,
}

impl Coverage {
    /// Whether some source code was skipped or some internal imports did not
    /// resolve, so graph-based findings may be incomplete.
    pub fn is_partial(&self) -> bool {
        self.analyzed_files < self.source_files || self.imports.unresolved > 0
    }
}

/// Measure coverage for the scanned `files`, given the per-file tallies the
/// dependency graph build recorded (keyed by normalised path).
pub fn measure(
    root: &Path,
    files: &[PathBuf],
    tallies: &HashMap<PathBuf, ImportTally>,
) -> Coverage {
    let mut coverage = Coverage::default();
    let mut gaps: BTreeMap<&'static str, usize> = BTreeMap::new();

    for file in files {
        let ext = file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let Some(language) = rules::language_for_extension(&ext) else {
            continue;
        };
        if !rules::language_has_imports(language) {
            continue;
        }
        coverage.source_files += 1;
        if !follows_imports(&ext) {
            *gaps.entry(language).or_default() += 1;
        }
    }

    let mut unresolved: Vec<(&PathBuf, usize, &str)> = Vec::new();
    for (path, tally) in tallies {
        if !tally.readable {
            coverage.unreadable_files += 1;
            continue;
        }
        coverage.analyzed_files += 1;
        coverage.imports.resolved += tally.resolved;
        coverage.imports.unresolved += tally.unresolved.len();
        coverage.imports.external += tally.external;
        unresolved.extend(
            tally
                .unresolved
                .iter()
                .map(|(line, import)| (path, *line, import.as_str())),
        );
    }
    coverage.imports.internal = coverage.imports.resolved + coverage.imports.unresolved;

    unresolved.sort();
    coverage.unresolved = unresolved
        .into_iter()
        .take(MAX_UNRESOLVED)
        .map(|(path, line, import)| {
            let at = Location::new(root, path, line, import);
            UnresolvedImport {
                file: at.file,
                line,
                import: at.text,
            }
        })
        .collect();

    let mut gaps: Vec<LanguageGap> = gaps
        .into_iter()
        .map(|(language, files)| LanguageGap {
            language: language.to_string(),
            files,
        })
        .collect();
    gaps.sort_by(|a, b| b.files.cmp(&a.files).then(a.language.cmp(&b.language)));
    coverage.not_analyzed = gaps;

    coverage.summary = summary(&coverage);
    coverage
}

/// One sentence describing `coverage`.
pub fn summary(c: &Coverage) -> String {
    if c.source_files == 0 {
        return "No source files with imports found".to_string();
    }
    let mut parts = Vec::new();
    if c.analyzed_files == c.source_files {
        parts.push(format!(
            "Imports followed in {}",
            all(c.source_files, "source file")
        ));
    } else {
        parts.push(format!(
            "Imports followed in {} of {} source files ({}%)",
            c.analyzed_files,
            c.source_files,
            percent(c.analyzed_files, c.source_files)
        ));
    }
    if c.imports.internal > 0 {
        if c.imports.unresolved == 0 {
            parts.push(format!(
                "{} resolved",
                all(c.imports.internal, "internal import")
            ));
        } else {
            parts.push(format!(
                "{} of {} internal imports unresolved",
                c.imports.unresolved, c.imports.internal
            ));
        }
    }
    if c.unreadable_files > 0 {
        parts.push(format!(
            "{} file{} unreadable",
            c.unreadable_files,
            plural(c.unreadable_files)
        ));
    }
    if !c.not_analyzed.is_empty() {
        let langs: Vec<String> = c
            .not_analyzed
            .iter()
            .map(|g| format!("{} ({} file{})", g.language, g.files, plural(g.files)))
            .collect();
        parts.push(format!("not analyzed: {}", langs.join(", ")));
    }
    parts.join("; ")
}

/// `part` as a whole percentage of `whole`, rounded down so a partial
/// result never shows as 100%.
fn percent(part: usize, whole: usize) -> usize {
    (part * 100).checked_div(whole).unwrap_or(0)
}

/// "the only file" or "all 3 files".
fn all(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("the only {noun}")
    } else {
        format!("all {n} {noun}s")
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tally(resolved: usize, unresolved: &[(usize, &str)], external: usize) -> ImportTally {
        ImportTally {
            readable: true,
            resolved,
            unresolved: unresolved
                .iter()
                .map(|(l, s)| (*l, s.to_string()))
                .collect(),
            external,
        }
    }

    #[test]
    fn counts_followed_skipped_and_unresolved() {
        let root = Path::new("/repo");
        let files: Vec<PathBuf> = [
            "src/a.ts",
            "src/b.py",
            "svc/Main.java",
            "svc/Util.java",
            "cmd/main.go",
            "README.md",
            "config.yml",
        ]
        .iter()
        .map(|f| root.join(f))
        .collect();
        let mut tallies = HashMap::new();
        tallies.insert(root.join("src/a.ts"), tally(3, &[(4, "./gone")], 2));
        tallies.insert(root.join("src/b.py"), tally(1, &[], 5));

        let c = measure(root, &files, &tallies);
        assert_eq!(c.source_files, 5, "md and yml are not source files");
        assert_eq!(c.analyzed_files, 2);
        assert_eq!(
            c.not_analyzed,
            vec![
                LanguageGap {
                    language: "Java".into(),
                    files: 2
                },
                LanguageGap {
                    language: "Go".into(),
                    files: 1
                },
            ]
        );
        assert_eq!(
            c.imports,
            ImportCounts {
                internal: 5,
                resolved: 4,
                unresolved: 1,
                external: 7
            }
        );
        assert_eq!(
            c.unresolved,
            vec![UnresolvedImport {
                file: "src/a.ts".into(),
                line: 4,
                import: "./gone".into()
            }]
        );
        assert!(c.is_partial());
        assert_eq!(
            c.summary,
            "Imports followed in 2 of 5 source files (40%); 1 of 5 internal imports \
             unresolved; not analyzed: Java (2 files), Go (1 file)"
        );
    }

    #[test]
    fn complete_coverage_says_so() {
        let root = Path::new("/repo");
        let files = vec![root.join("a.rs")];
        let mut tallies = HashMap::new();
        tallies.insert(root.join("a.rs"), tally(2, &[], 0));
        let c = measure(root, &files, &tallies);
        assert!(!c.is_partial());
        assert_eq!(
            c.summary,
            "Imports followed in the only source file; all 2 internal imports resolved"
        );
    }

    #[test]
    fn unreadable_files_are_not_analyzed() {
        let root = Path::new("/repo");
        let files = vec![root.join("a.py")];
        let mut tallies = HashMap::new();
        tallies.insert(root.join("a.py"), ImportTally::default());
        let c = measure(root, &files, &tallies);
        assert_eq!(c.analyzed_files, 0);
        assert_eq!(c.unreadable_files, 1);
        assert!(c.summary.contains("1 file unreadable"), "{}", c.summary);
    }

    #[test]
    fn no_source_files() {
        let c = measure(
            Path::new("/r"),
            &[PathBuf::from("/r/README.md")],
            &HashMap::new(),
        );
        assert_eq!(c.summary, "No source files with imports found");
        assert!(!c.is_partial());
    }

    #[test]
    fn percent_rounds_down() {
        assert_eq!(percent(199, 200), 99);
        assert_eq!(percent(0, 0), 0);
    }
}
