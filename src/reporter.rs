//! Reporter module.
//!
//! Renders `AnalysisResult` as either a human-readable CLI report or as
//! structured JSON matching the schema defined in `SPEC.md`.

use crate::analysis::AnalysisResult;
use crate::architecture::Confidence;
use crate::safe_text::sanitize;
use std::borrow::Cow;
use std::io::IsTerminal;
use std::sync::OnceLock;

// ── Colour ────────────────────────────────────────────────────────────────────

// Every line goes through `paint`, which drops the colour codes when colour
// is off. This shadows the standard `println!` for this file only.
macro_rules! println {
    () => {
        std::println!()
    };
    ($($arg:tt)*) => {
        std::println!("{}", paint(&format!($($arg)*)))
    };
}

/// Whether to colour the output: off when `NO_COLOR` is set (no-color.org)
/// or stdout is not a terminal, unless `CLICOLOR_FORCE` or `FORCE_COLOR`
/// asks for it.
fn colour_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        let set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty() && v != "0");
        if std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) {
            return false;
        }
        if set("CLICOLOR_FORCE") || set("FORCE_COLOR") {
            return true;
        }
        std::io::stdout().is_terminal()
    })
}

fn paint(line: &str) -> Cow<'_, str> {
    if colour_enabled() {
        Cow::Borrowed(line)
    } else {
        Cow::Owned(strip_colour(line))
    }
}

/// Removes SGR colour codes such as `\x1b[1m` or `\x1b[32m`.
fn strip_colour(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(i) = rest.find("\x1b[") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        match after.find(|c: char| !(c.is_ascii_digit() || c == ';')) {
            Some(j) if after[j..].starts_with('m') => rest = &after[j + 1..],
            _ => {
                out.push_str(&rest[i..i + 2]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

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

    // ── Summary ───────────────────────────────────────────────────────────────
    section("Summary");
    kv("Total LOC", &format_loc(result.total_loc));
    if result.comment_lines > 0 {
        kv("Comment lines", &format_loc(result.comment_lines));
    }
    kv("Languages", &result.languages.len().to_string());
    kv("Frameworks", &result.frameworks.len().to_string());
    kv("Databases", &result.databases.len().to_string());
    kv("Infrastructure", &result.infrastructure.len().to_string());
    kv("Files scanned", &result.file_breakdown.len().to_string());
    if !result.workspaces.is_empty() {
        kv(
            "Workspace packages",
            &crate::workspaces::package_count(&result.workspaces).to_string(),
        );
    }
    let dead = result.dead_code_candidates();
    if dead.is_empty() {
        kv("Dead files", "0");
    } else {
        kv(
            "Dead files",
            &format!(
                "{}  ({})",
                dead.len(),
                crate::dead_code_detector::confidence_summary(&dead)
            ),
        );
    }
    kv(
        "Circular imports",
        &result.circular_imports.len().to_string(),
    );
    kv(
        "Dependency links",
        &result
            .dependency_graph
            .values()
            .map(|v| v.len())
            .sum::<usize>()
            .to_string(),
    );
    kv("Coverage", &safe(&result.coverage.summary));

    // ── Project Type ─────────────────────────────────────────────────────────
    section("Project Type");
    println!("  {}", result.project_type);

    // ── Workspaces ───────────────────────────────────────────────────────────
    if !result.workspaces.is_empty() {
        section("Workspaces");
        print_workspaces(&result.workspaces);
    }

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
    let details = &result.architecture_details;
    match details.layer_chain() {
        Some(chain) => println!(
            "  Code layers: {chain}  (confidence: {})",
            details.confidence.as_str()
        ),
        None => println!(
            "  Code layers: none found  (confidence: {})",
            details.confidence.as_str()
        ),
    }
    for line in &details.evidence {
        println!("    \x1b[2m· {line}\x1b[0m");
    }
    for layer in &details.layers {
        if let Some(example) = layer.examples.first() {
            println!(
                "    \x1b[2m· e.g. {}: {}\x1b[0m",
                layer.name,
                example.describe()
            );
        }
    }

    // ── Languages ─────────────────────────────────────────────────────────────
    section("Languages");
    if result.languages.is_empty() {
        println!("  (no source files detected)");
    } else {
        let mut langs: Vec<(&String, &usize)> = result.languages.iter().collect();
        langs.sort_by(|a, b| b.1.cmp(a.1));

        const BAR_WIDTH: usize = 28;
        for (lang, loc) in &langs {
            let pct = (**loc * 100).checked_div(result.total_loc).unwrap_or(0);
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

    // ── Coverage ──────────────────────────────────────────────────────────────
    if !result.coverage.unresolved.is_empty() {
        section("Unresolved Imports");
        print_unresolved(&result.coverage, 5);
    }

    // ── Dead Code ─────────────────────────────────────────────────────────────
    if !result.dead_code.is_empty() {
        section("Dead Code");
        print_candidates(&result.dead_code_candidates());
    }

    // ── Circular Imports ──────────────────────────────────────────────────────
    if !result.circular_imports.is_empty() {
        section("Circular Imports");
        print_cycle_list(&result.circular_imports);
    }

    println!();
}

/// Each workspace with its packages, the first few per workspace.
fn print_workspaces(workspaces: &[crate::workspaces::Workspace]) {
    const SHOWN: usize = 15;
    for w in workspaces {
        println!("  {}", crate::workspaces::summary(w));
        for p in w.packages.iter().take(SHOWN) {
            println!(
                "    {:<24}  {}  \x1b[2m{}\x1b[0m",
                safe(&p.name),
                safe(&crate::workspaces::package_line(p)),
                p.path.display()
            );
        }
        if w.packages.len() > SHOWN {
            println!("    \x1b[2m… and {} more\x1b[0m", w.packages.len() - SHOWN);
        }
    }
}

// ── Packages ─────────────────────────────────────────────────────────────────

/// Every workspace package with its own numbers and the packages it uses.
pub fn print_packages(result: &AnalysisResult) {
    println!();
    if result.workspaces.is_empty() {
        println!("  No workspaces are declared at the root, so this is one package.");
        println!(
            "  \x1b[2mWorkspaces are read from package.json, pnpm-workspace.yaml, lerna.json,\x1b[0m"
        );
        println!("  \x1b[2mCargo.toml, go.work and uv (pyproject.toml).\x1b[0m");
        println!();
        return;
    }
    for w in &result.workspaces {
        println!("  \x1b[1m{}\x1b[0m", crate::workspaces::summary(w));
        println!();
        for p in &w.packages {
            println!(
                "  \x1b[1m{}\x1b[0m  \x1b[2m{}\x1b[0m",
                safe(&p.name),
                p.path.display()
            );
            let mut facts = vec![
                format!("{} LOC", format_loc(p.loc)),
                format!("{} file{}", p.files, if p.files == 1 { "" } else { "s" }),
            ];
            if !p.languages.is_empty() {
                facts.push(p.languages.join(", "));
            }
            println!("    {}", facts.join("  ·  "));
            let stack: Vec<String> = p.frameworks.iter().chain(&p.databases).cloned().collect();
            if !stack.is_empty() {
                println!("    Stack:    {}", safe(&stack.join(", ")));
            }
            if !p.depends_on.is_empty() {
                println!("    Uses:     {}", safe(&name_list(&p.depends_on)));
            }
            if !p.used_by.is_empty() {
                println!("    Used by:  {}", safe(&name_list(&p.used_by)));
            }
            if p.dead_files > 0 {
                println!("    Dead files: {}", p.dead_files);
            }
            println!();
        }
    }
    println!(
        "  \x1b[2mRun `codedna analyze <package path>` for the full report on one package.\x1b[0m"
    );
    println!();
}

/// Manifest text as it is safe to print.
/// Joins package names, shortening long lists for the terminal.
fn name_list(names: &[String]) -> String {
    const SHOWN: usize = 12;
    if names.len() <= SHOWN {
        return names.join(", ");
    }
    format!(
        "{} and {} more",
        names[..SHOWN].join(", "),
        names.len() - SHOWN
    )
}

fn safe(s: &str) -> String {
    crate::safe_text::sanitize(s)
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
            let pct = (loc * 100).checked_div(result.total_loc).unwrap_or(0);
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
    println!("  {:<col_w$} {:>8}   Language", "File", "LOC");
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
        print_clean(&result.coverage, "dead code");
        return;
    }

    let candidates = result.dead_code_candidates();
    println!(
        "  \x1b[1mPossibly unused files\x1b[0m  ({} file{}: {})",
        candidates.len(),
        if candidates.len() == 1 { "" } else { "s" },
        crate::dead_code_detector::confidence_summary(&candidates)
    );
    println!(
        "  \x1b[2mNot reachable from a detected entry point. Check before deleting: high means\x1b[0m"
    );
    println!(
        "  \x1b[2mno other file mentions it, low means it is probably loaded another way.\x1b[0m"
    );
    println!();
    print_candidates(&candidates);
    print_coverage_note(&result.coverage);
    println!();
}

// ── Circular imports output ───────────────────────────────────────────────────

/// Print groups of files that import each other.
pub fn print_cycles(result: &AnalysisResult) {
    println!();
    let cycles = &result.circular_imports;
    if cycles.is_empty() {
        print_clean(&result.coverage, "circular imports");
        return;
    }
    println!(
        "  \x1b[1mCircular imports\x1b[0m  ({} group{})",
        cycles.len(),
        if cycles.len() == 1 { "" } else { "s" }
    );
    println!(
        "  \x1b[2mEach line is the shortest import loop in a group of files that import\x1b[0m"
    );
    println!(
        "  \x1b[2meach other. Type-only imports count too, so check before refactoring.\x1b[0m"
    );
    println!();
    print_cycle_list(cycles);
    print_coverage_note(&result.coverage);
    println!();
}

/// A clean dead-code or cycle result, worded by how much was analyzed so
/// "nothing found" is not mistaken for "could not look".
fn print_clean(coverage: &crate::coverage::Coverage, what: &str) {
    if coverage.source_files > 0 && coverage.analyzed_files == 0 {
        println!(
            "  \x1b[33m! Not checked for {what}: no source file had its imports followed.\x1b[0m"
        );
        print_coverage_note(coverage);
    } else if coverage.is_partial() {
        println!("  \x1b[32m✓ No {what} found in the analyzed files.\x1b[0m");
        print_coverage_note(coverage);
    } else {
        println!("  \x1b[32m✓ No {what} detected.\x1b[0m");
    }
    println!();
}

/// A dim coverage line when some code was skipped or imports did not resolve.
fn print_coverage_note(coverage: &crate::coverage::Coverage) {
    if coverage.is_partial() {
        println!("  \x1b[2mCoverage: {}.\x1b[0m", safe(&coverage.summary));
    }
}

/// The first `limit` unresolved imports as `file:line  import`.
fn print_unresolved(coverage: &crate::coverage::Coverage, limit: usize) {
    for u in coverage.unresolved.iter().take(limit) {
        println!(
            "  {}:{}  \x1b[2m{}\x1b[0m",
            safe(&u.file),
            u.line,
            safe(&u.import)
        );
    }
    let more = coverage
        .imports
        .unresolved
        .saturating_sub(limit.min(coverage.unresolved.len()));
    if more > 0 {
        println!("  \x1b[2m… and {more} more\x1b[0m");
    }
}

/// One line per cycle: `a → b → a`.
fn print_cycle_list(cycles: &[Vec<std::path::PathBuf>]) {
    for cycle in cycles {
        println!("  {}", sanitize(&cycle_line(cycle)));
    }
}

/// A cycle as `a → b → a`.
pub fn cycle_line(cycle: &[std::path::PathBuf]) -> String {
    cycle
        .iter()
        .chain(cycle.first())
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(" → ")
}

/// One line per candidate with its confidence, then the reason, dimmed.
fn print_candidates(candidates: &[crate::dead_code_detector::DeadCodeCandidate]) {
    for c in candidates {
        let colour = match c.confidence {
            Confidence::High => "31",
            Confidence::Medium => "33",
            Confidence::Low => "2",
        };
        println!(
            "  \x1b[{colour}m{:<6}\x1b[0m  {}",
            c.confidence.as_str(),
            sanitize(&c.file.to_string_lossy())
        );
        println!("          \x1b[2m{}\x1b[0m", sanitize(&c.reason));
        for at in &c.evidence {
            println!(
                "          \x1b[2m  {}:{}  {}\x1b[0m",
                at.file, at.line, at.text
            );
        }
    }
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

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Format a LOC number with thousands separators (e.g. 12450 → "12,450").
fn format_loc(n: usize) -> String {
    let s = n.to_string();
    let mut result = String::new();
    let chars: Vec<char> = s.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if i > 0 && (chars.len() - i).is_multiple_of(3) {
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

// ── JSON tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::FileInfo;
    use std::collections::HashMap;
    use std::path::PathBuf;

    #[test]
    fn strip_colour_removes_only_colour_codes() {
        assert_eq!(strip_colour("\x1b[1mStack\x1b[0m"), "Stack");
        assert_eq!(
            strip_colour("\x1b[32m✓\x1b[90m dim\x1b[0m end"),
            "✓ dim end"
        );
        assert_eq!(strip_colour("plain [1m text"), "plain [1m text");
        assert_eq!(strip_colour("\x1b[2J"), "\x1b[2J");
    }

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
            coverage: Default::default(),
            project_type: "Full-stack web application".to_string(),
            total_loc: 14550,
            comment_lines: 0,
            blank_lines: 0,
            architecture_details: Default::default(),
            dead_code_details: Vec::new(),
            circular_imports: Vec::new(),
            workspaces: Vec::new(),
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
