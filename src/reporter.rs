//! Reporter module.
//!
//! Renders `AnalysisResult` as either a human-readable CLI report or as
//! structured JSON matching the schema defined in `SPEC.md`.

use crate::analysis::AnalysisResult;

// ── Box-drawing header ────────────────────────────────────────────────────────

const HEADER_WIDTH: usize = 42;

fn print_header() {
    let title = "CodeDna Intelligence Report";
    let pad = (HEADER_WIDTH - 2 - title.len()) / 2;
    let top = "═".repeat(HEADER_WIDTH - 2);
    let blank_pad = " ".repeat(HEADER_WIDTH - 2);

    println!("╔{top}╗");
    println!("║{blank_pad}║");
    println!("║{:>pad$}{title}{:>pad$}║", "", "");
    println!("║{blank_pad}║");
    println!("╚{top}╝");
}

fn section(title: &str) {
    println!("\n  \x1b[1m{title}\x1b[0m");
    println!("  {}", "─".repeat(title.len()));
}

fn kv(label: &str, value: &str) {
    println!("  {label:<18} {value}");
}

// ── Full intelligence report ──────────────────────────────────────────────────

/// Print the full formatted intelligence report to stdout.
pub fn print_report(result: &AnalysisResult) {
    println!();
    print_header();

    // ── Project Type ─────────────────────────────────────────────────────────
    section("Project Type");
    println!("  {}", result.project_type);

    // ── Stack ─────────────────────────────────────────────────────────────────
    section("Stack");
    if result.frameworks.is_empty() {
        println!("  (no frameworks detected)");
    } else {
        println!("  {}", result.frameworks.join("  +  "));
    }

    // ── Databases ─────────────────────────────────────────────────────────────
    if !result.databases.is_empty() {
        section("Databases");
        println!("  {}", result.databases.join(",  "));
    }

    // ── Infrastructure ────────────────────────────────────────────────────────
    if !result.infrastructure.is_empty() {
        section("Infrastructure");
        println!("  {}", result.infrastructure.join("  •  "));
    }

    // ── Architecture ──────────────────────────────────────────────────────────
    section("Architecture");
    println!("  {}", result.architecture);

    // ── Languages ─────────────────────────────────────────────────────────────
    section("Languages");
    if result.languages.is_empty() {
        println!("  (no source files detected)");
    } else {
        let mut langs: Vec<(&String, &usize)> = result.languages.iter().collect();
        langs.sort_by(|a, b| b.1.cmp(a.1));

        const BAR_WIDTH: usize = 28;
        for (lang, loc) in &langs {
            let pct = if result.total_loc > 0 {
                (**loc * 100) / result.total_loc
            } else {
                0
            };
            let filled = (pct * BAR_WIDTH) / 100;
            let empty = BAR_WIDTH - filled;
            let bar = format!(
                "\x1b[32m{}\x1b[90m{}\x1b[0m",
                "█".repeat(filled),
                "░".repeat(empty)
            );
            println!(
                "  {:<16}  {}  {:>3}%   {:>8} LOC",
                lang,
                bar,
                pct,
                format_loc(**loc),
            );
        }
    }

    // ── Top Files ─────────────────────────────────────────────────────────────
    let top_n = 5;
    let top_files: Vec<_> = result.file_breakdown.iter().take(top_n).collect();
    if !top_files.is_empty() {
        section(&format!("Top {top_n} Files by LOC"));
        for (i, info) in top_files.iter().enumerate() {
            println!(
                "  {}  {:<50}  {:>8} LOC   {}",
                i + 1,
                info.file.display(),
                format_loc(info.loc),
                info.language
            );
        }
    }

    // ── Dead Code ─────────────────────────────────────────────────────────────
    if !result.dead_code.is_empty() {
        section("Dead Code");
        for file in &result.dead_code {
            println!("  \x1b[33m{}\x1b[0m", file.display());
        }
    }

    // ── Summary ───────────────────────────────────────────────────────────────
    section("Summary");
    kv("Total LOC", &format_loc(result.total_loc));
    kv("Languages", &result.languages.len().to_string());
    kv("Frameworks", &result.frameworks.len().to_string());
    kv("Databases", &result.databases.len().to_string());
    kv("Infrastructure", &result.infrastructure.len().to_string());
    kv("Files scanned", &result.file_breakdown.len().to_string());
    kv("Dead files", &result.dead_code.len().to_string());
    kv(
        "Dependency links",
        &result
            .dependency_graph
            .values()
            .map(|v| v.len())
            .sum::<usize>()
            .to_string(),
    );

    println!();
}

// ── Stack-only output ─────────────────────────────────────────────────────────

/// Print detected tech stack (languages, frameworks, databases).
pub fn print_stack(result: &AnalysisResult) {
    println!();
    println!("  \x1b[1mStack\x1b[0m");
    println!();

    // Languages
    if !result.languages.is_empty() {
        let mut langs: Vec<(&String, &usize)> = result.languages.iter().collect();
        langs.sort_by(|a, b| b.1.cmp(a.1));
        println!("  Languages:");
        for (lang, loc) in langs {
            let pct = if result.total_loc > 0 {
                (loc * 100) / result.total_loc
            } else {
                0
            };
            println!("    {:<18} {:>7} LOC   ({}%)", lang, format_loc(*loc), pct);
        }
        println!();
    }

    // Frameworks
    if result.frameworks.is_empty() {
        println!("  Frameworks:  (none detected)");
    } else {
        println!("  Frameworks:");
        for fw in &result.frameworks {
            println!("    • {fw}");
        }
    }
    println!();

    // Databases
    if result.databases.is_empty() {
        println!("  Databases:   (none detected)");
    } else {
        println!("  Databases:");
        for db in &result.databases {
            println!("    • {db}");
        }
    }
    println!();
}

// ── File LOC breakdown ────────────────────────────────────────────────────────

/// Print per-file LOC breakdown, sorted by LOC descending.
pub fn print_files(result: &AnalysisResult) {
    println!();
    if result.file_breakdown.is_empty() {
        println!("  No source files found.");
        println!();
        return;
    }

    let col_w = longest_path_len(result) + 2;
    println!("  {:<col_w$} {:>8}   {}", "File", "LOC", "Language");
    println!("  {}", "─".repeat(col_w + 22));

    for info in &result.file_breakdown {
        println!(
            "  {:<col_w$} {:>8}   {}",
            info.file.display(),
            format_loc(info.loc),
            info.language
        );
    }

    println!();
    println!("  {:<col_w$} {:>8}", "TOTAL", format_loc(result.total_loc));
    println!();
}

// ── Dead code output ──────────────────────────────────────────────────────────

/// Print dead code list.
pub fn print_dead_code(result: &AnalysisResult) {
    println!();
    if result.dead_code.is_empty() {
        println!("  \x1b[32m✓ No dead code detected.\x1b[0m");
        println!();
        return;
    }

    println!(
        "  \x1b[1mUnused files detected\x1b[0m  ({} file{})",
        result.dead_code.len(),
        if result.dead_code.len() == 1 { "" } else { "s" }
    );
    println!();
    for file in &result.dead_code {
        println!("  \x1b[33m  {}\x1b[0m", file.display());
    }
    println!();
}

// ── Framework usage output ────────────────────────────────────────────────────

/// Print files that import the given framework (by scanning the file breakdown
/// for files whose language matches the framework's ecosystem).
/// Print a list of files that import/use the given framework.
///
/// `matching_files` is the pre-computed result from
/// `framework_detector::detect_files_using_framework`.
pub fn print_framework_usage(name: &str, matching_files: &[std::path::PathBuf]) {
    println!();

    if matching_files.is_empty() {
        println!("  No files found using framework '\x1b[1m{name}\x1b[0m'.");
        println!();
        println!("  Possible reasons:");
        println!("    • The framework is not used in this repository");
        println!("    • The framework name was not recognised (try lowercase, e.g. 'react')");
        println!("    • Imports use a non-standard pattern not yet covered");
        println!();
        return;
    }

    println!(
        "  \x1b[1m{name}\x1b[0m used in {} file{}:",
        matching_files.len(),
        if matching_files.len() == 1 { "" } else { "s" }
    );
    println!();
    for file in matching_files {
        println!("  \x1b[33m  {}\x1b[0m", file.display());
    }
    println!();
}

// ── JSON output (Prompt 12) ───────────────────────────────────────────────────

/// Serialize `AnalysisResult` to JSON and print to stdout.
///
/// When `compact` is `false` (default) output is pretty-printed with 2-space
/// indentation.  When `compact` is `true` output is a single line — useful for
/// piping directly into other tools.
///
/// The JSON schema matches `SPEC.md`:
/// ```json
/// {
///   "project_type": "...",
///   "total_loc": 15780,
///   "languages": { "TypeScript": 12450 },
///   "frameworks": ["React"],
///   "databases": ["PostgreSQL"],
///   "architecture": "Frontend → API → Database",
///   "dead_code": ["src/utils/oldHelper.ts"],
///   "dependency_graph": { "src/server.ts": ["src/api/routes.ts"] },
///   "file_breakdown": [{ "file": "src/server.ts", "loc": 340, "language": "TypeScript" }]
/// }
/// ```
pub fn print_json(result: &AnalysisResult, compact: bool) {
    let output = if compact {
        serde_json::to_string(result)
    } else {
        serde_json::to_string_pretty(result)
    };

    match output {
        Ok(json) => println!("{json}"),
        Err(e) => eprintln!("error: JSON serialisation failed: {e}"),
    }
}

// ── JSON tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::FileInfo;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn minimal_result() -> AnalysisResult {
        let mut languages = HashMap::new();
        languages.insert("TypeScript".to_string(), 12450_usize);
        languages.insert("JavaScript".to_string(), 2100_usize);

        let mut dep_graph = HashMap::new();
        dep_graph.insert(
            PathBuf::from("src/server.ts"),
            vec![PathBuf::from("src/api/routes.ts")],
        );

        AnalysisResult {
            project_type: "Full-stack web application".to_string(),
            total_loc: 14550,
            languages,
            frameworks: vec!["React".to_string(), "Express".to_string()],
            databases: vec!["PostgreSQL".to_string()],
            infrastructure: vec!["Docker".to_string(), "GitHub Actions".to_string()],
            architecture: "Frontend → API → Database".to_string(),
            dead_code: vec![PathBuf::from("src/utils/oldHelper.ts")],
            dependency_graph: dep_graph,
            file_breakdown: vec![FileInfo {
                file: PathBuf::from("src/server.ts"),
                loc: 340,
                language: "TypeScript".to_string(),
            }],
        }
    }

    #[test]
    fn json_output_is_valid_and_contains_required_keys() {
        let result = minimal_result();
        let json = serde_json::to_string_pretty(&result).expect("serialisation failed");

        let parsed: serde_json::Value =
            serde_json::from_str(&json).expect("output is not valid JSON");

        // Required top-level keys from SPEC.md
        for key in &[
            "project_type",
            "total_loc",
            "languages",
            "frameworks",
            "databases",
            "architecture",
            "dead_code",
            "dependency_graph",
            "file_breakdown",
        ] {
            assert!(
                parsed.get(key).is_some(),
                "missing required key: {key}\nJSON:\n{json}"
            );
        }
    }

    #[test]
    fn json_languages_map_has_correct_values() {
        let result = minimal_result();
        let json = serde_json::to_string(&result).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["languages"]["TypeScript"], 12450);
        assert_eq!(parsed["languages"]["JavaScript"], 2100);
    }

    #[test]
    fn json_frameworks_is_array_of_strings() {
        let result = minimal_result();
        let json = serde_json::to_string(&result).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        let fw = parsed["frameworks"]
            .as_array()
            .expect("frameworks must be array");
        assert!(fw.contains(&serde_json::json!("React")));
        assert!(fw.contains(&serde_json::json!("Express")));
    }

    #[test]
    fn json_dead_code_is_array_of_strings() {
        let result = minimal_result();
        let json = serde_json::to_string(&result).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        let dc = parsed["dead_code"]
            .as_array()
            .expect("dead_code must be array");
        assert_eq!(dc.len(), 1);
        assert_eq!(dc[0], "src/utils/oldHelper.ts");
    }

    #[test]
    fn json_dependency_graph_keys_and_values_are_strings() {
        let result = minimal_result();
        let json = serde_json::to_string(&result).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        let dg = parsed["dependency_graph"]
            .as_object()
            .expect("dependency_graph must be object");

        assert!(dg.contains_key("src/server.ts"));
        let deps = dg["src/server.ts"].as_array().expect("deps must be array");
        assert_eq!(deps[0], "src/api/routes.ts");
    }

    #[test]
    fn json_file_breakdown_entries_have_correct_shape() {
        let result = minimal_result();
        let json = serde_json::to_string(&result).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        let fb = parsed["file_breakdown"]
            .as_array()
            .expect("file_breakdown must be array");
        assert_eq!(fb.len(), 1);

        let entry = &fb[0];
        assert_eq!(entry["file"], "src/server.ts");
        assert_eq!(entry["loc"], 340);
        assert_eq!(entry["language"], "TypeScript");
    }

    #[test]
    fn compact_json_is_single_line() {
        let result = minimal_result();
        let json = serde_json::to_string(&result).unwrap();
        // Compact output must not contain newlines
        assert!(!json.contains('\n'), "compact JSON should be a single line");
    }

    #[test]
    fn pretty_json_is_multi_line() {
        let result = minimal_result();
        let json = serde_json::to_string_pretty(&result).unwrap();
        assert!(
            json.contains('\n'),
            "pretty JSON should span multiple lines"
        );
    }

    #[test]
    fn json_round_trips_losslessly() {
        let result = minimal_result();
        let json = serde_json::to_string(&result).unwrap();
        let restored: AnalysisResult = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.project_type, result.project_type);
        assert_eq!(restored.total_loc, result.total_loc);
        assert_eq!(restored.frameworks, result.frameworks);
        assert_eq!(restored.databases, result.databases);
        assert_eq!(restored.architecture, result.architecture);
        assert_eq!(restored.dead_code, result.dead_code);
        assert_eq!(
            restored.languages.get("TypeScript"),
            result.languages.get("TypeScript")
        );
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Format a LOC number with thousands separators (e.g. 12450 → "12,450").
fn format_loc(n: usize) -> String {
    let s = n.to_string();
    let mut result = String::new();
    let chars: Vec<char> = s.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if i > 0 && (chars.len() - i) % 3 == 0 {
            result.push(',');
        }
        result.push(*c);
    }
    result
}

/// Return the display length of the longest file path in the breakdown.
fn longest_path_len(result: &AnalysisResult) -> usize {
    result
        .file_breakdown
        .iter()
        .map(|f| f.file.display().to_string().len())
        .max()
        .unwrap_or(20)
        .max(4) // at least "File"
}
