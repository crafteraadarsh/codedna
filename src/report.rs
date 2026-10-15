//! Shareable reports.
//!
//! Renders an analysis as a Markdown or HTML document that can be committed,
//! attached to a pull request or sent to someone who does not have CodeDna.
//! The HTML report is a single self-contained file: no scripts, no external
//! resources, and a print stylesheet so "Print to PDF" in any browser gives a
//! clean PDF.
//!
//! Everything shown comes from the scanned repository, which CodeDna does not
//! trust, so all of it is escaped for the target format.

use crate::analysis::AnalysisResult;
use crate::architecture::Confidence;
use crate::context::{self, ContextPack};
use crate::dead_code_detector::confidence_summary;
use crate::hotspots::HotspotReport;
use crate::safe_text::sanitize;
use std::fmt::Write as FmtWrite;
use std::path::Path;

/// Depth of the directory map in a report.
pub const MAP_DEPTH: usize = 3;

/// Token budget handed to the context builder; large enough that only very
/// big directory maps are trimmed.
const REPORT_TOKENS: usize = 25_000;

/// How many dead-code candidates a report lists.
const DEAD_CODE_LIMIT: usize = 50;

/// A possibly unused file as shown in a report.
pub struct DeadFile {
    pub file: String,
    pub confidence: Confidence,
    pub reason: String,
}

/// Everything a report shows.
pub struct Report {
    pub name: String,
    pub pack: ContextPack,
    pub dead_code: Vec<DeadFile>,
    /// Count per confidence level, e.g. "2 high, 1 low".
    pub dead_summary: String,
    /// Files that change often and are large; `None` without Git history.
    pub hotspots: Option<HotspotReport>,
}

/// Build a report from an analysis and a rendered directory map.
pub fn build(name: &str, result: &AnalysisResult, tree: &str) -> Report {
    // Candidates arrive most certain first, so truncation keeps the best ones.
    let candidates = result.dead_code_candidates();
    let dead_summary = confidence_summary(&candidates);
    let mut dead_code: Vec<DeadFile> = candidates
        .into_iter()
        .map(|c| DeadFile {
            file: sanitize(&c.file.to_string_lossy()),
            confidence: c.confidence,
            reason: sanitize(&c.reason),
        })
        .collect();
    dead_code.truncate(DEAD_CODE_LIMIT);
    Report {
        name: sanitize(name),
        pack: context::build(result, tree, REPORT_TOKENS),
        dead_code,
        dead_summary,
        hotspots: None,
    }
}

/// A display name for the analysed project: the repository name for a Git
/// URL, otherwise the folder name.
pub fn project_name(input: &str, local_path: &Path) -> String {
    if crate::git_handler::is_git_url(input) {
        let url = crate::git_handler::redact_url(input);
        let last = url
            .trim_end_matches('/')
            .rsplit(['/', ':'])
            .next()
            .unwrap_or_default();
        let last = last.rsplit('@').next().unwrap_or_default();
        let last = last.strip_suffix(".git").unwrap_or(last);
        if !last.is_empty() {
            return last.to_string();
        }
    } else if let Some(name) = local_path
        .canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
    {
        return name;
    }
    "project".to_string()
}

// ---------------------------------------------------------------------------
// Markdown
// ---------------------------------------------------------------------------

/// Render a report as Markdown.
pub fn render_markdown(report: &Report) -> String {
    let p = &report.pack;
    let mut out = String::new();
    let _ = writeln!(out, "# CodeDna report: {}\n", md_text(&report.name));

    let _ = writeln!(out, "| | |\n|---|---|");
    let _ = writeln!(out, "| Project type | {} |", md_cell(&p.project_type));
    let _ = writeln!(out, "| Architecture | {} |", md_cell(&p.architecture));
    if !p.code_layers.is_empty() {
        let _ = writeln!(out, "| Code layers | {} |", md_cell(&p.code_layers));
    }
    let _ = writeln!(out, "| Lines of code | {} |", p.total_loc);
    let _ = writeln!(out, "| Files | {} |", p.file_count);
    let _ = writeln!(out, "| Frameworks | {} |", md_list(&p.frameworks));
    let _ = writeln!(out, "| Databases | {} |", md_list(&p.databases));
    let _ = writeln!(out, "| Infrastructure | {} |", md_list(&p.infrastructure));

    if !p.languages.is_empty() {
        let _ = writeln!(out, "\n## Languages\n");
        let _ = writeln!(out, "| Language | LOC | Share |\n|---|---:|---:|");
        for l in &p.languages {
            let _ = writeln!(out, "| {} | {} | {}% |", md_cell(&l.name), l.loc, l.percent);
        }
    }

    if !p.entry_points.is_empty() {
        let _ = writeln!(out, "\n## Entry points\n");
        for f in &p.entry_points {
            let _ = writeln!(out, "- {}", md_code(f));
        }
    }

    if !p.most_imported.is_empty() {
        let _ = writeln!(out, "\n## Most imported files\n");
        let _ = writeln!(out, "| File | Imported by |\n|---|---:|");
        for f in &p.most_imported {
            let _ = writeln!(out, "| {} | {} |", md_code(&f.file), f.imported_by);
        }
    }

    if !p.largest_files.is_empty() {
        let _ = writeln!(out, "\n## Largest files\n");
        let _ = writeln!(out, "| File | LOC | Language |\n|---|---:|---|");
        for f in &p.largest_files {
            let _ = writeln!(
                out,
                "| {} | {} | {} |",
                md_code(&f.file),
                f.loc,
                md_cell(&f.language)
            );
        }
    }

    if let Some(h) = report.hotspots.as_ref().filter(|h| !h.hotspots.is_empty()) {
        let _ = writeln!(out, "\n## Hotspots\n");
        let _ = writeln!(out, "{}\n", hotspot_note(h));
        let _ = writeln!(out, "| File | Commits | LOC |\n|---|---:|---:|");
        for f in &h.hotspots {
            let _ = writeln!(out, "| {} | {} | {} |", md_code(&f.file), f.commits, f.loc);
        }
        if !h.coupling.is_empty() {
            let _ = writeln!(out, "\n### Files that change together\n");
            let _ = writeln!(out, "{}\n", COUPLING_NOTE);
            let _ = writeln!(
                out,
                "| Files | Shared commits | Together |\n|---|---:|---:|"
            );
            for c in &h.coupling {
                let _ = writeln!(
                    out,
                    "| {} + {} | {} | {}% |",
                    md_code(&c.file),
                    md_code(&c.other),
                    c.shared,
                    c.percent
                );
            }
        }
    }

    if !report.dead_code.is_empty() {
        let _ = writeln!(out, "\n## Possibly unused files\n");
        let _ = writeln!(
            out,
            "{}\n",
            dead_code_note(p.dead_code_candidates, &report.dead_summary)
        );
        for f in &report.dead_code {
            let _ = writeln!(
                out,
                "- {} ({}): {}",
                md_code(&f.file),
                f.confidence.as_str(),
                md_text(&f.reason)
            );
        }
        if p.dead_code_candidates > report.dead_code.len() {
            let _ = writeln!(
                out,
                "- …and {} more",
                p.dead_code_candidates - report.dead_code.len()
            );
        }
    }

    if !p.map.is_empty() {
        let _ = writeln!(out, "\n## Directory map\n");
        // A fence longer than any backtick run in the map cannot be closed early.
        let fence = "`".repeat(longest_backtick_run(&p.map).max(2) + 1);
        let _ = writeln!(out, "{fence}text\n{}{fence}", p.map);
        if p.map_truncated {
            let _ = writeln!(out, "\n(map trimmed)");
        }
    }

    let _ = writeln!(out, "\n---\n\n{}", footer());
    out
}

fn md_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(
            c,
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '#' | '|' | '!'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn md_cell(s: &str) -> String {
    md_text(s)
}

fn md_list(items: &[String]) -> String {
    if items.is_empty() {
        "none detected".to_string()
    } else {
        items
            .iter()
            .map(|s| md_cell(s.as_str()))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn md_code(s: &str) -> String {
    // Inline code cannot hold a backtick or a table pipe safely.
    format!("`{}`", s.replace('`', "'").replace('|', "¦"))
}

fn longest_backtick_run(s: &str) -> usize {
    let mut best = 0;
    let mut run = 0;
    for c in s.chars() {
        if c == '`' {
            run += 1;
            best = best.max(run);
        } else {
            run = 0;
        }
    }
    best
}

// ---------------------------------------------------------------------------
// HTML
// ---------------------------------------------------------------------------

const STYLE: &str = "\
:root{--fg:#1f2328;--muted:#59636e;--line:#d1d9e0;--bar:#2f81f7;--bg:#fff}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--fg);font:15px/1.5 -apple-system,BlinkMacSystemFont,'Segoe UI',Helvetica,Arial,sans-serif}
main{max-width:960px;margin:0 auto;padding:32px 24px}
h1{font-size:28px;margin:0 0 4px}
h2{font-size:19px;margin:32px 0 12px;padding-bottom:6px;border-bottom:1px solid var(--line)}
.sub{color:var(--muted);margin:0 0 24px}
.cards{display:grid;grid-template-columns:repeat(auto-fit,minmax(180px,1fr));gap:12px}
.card{border:1px solid var(--line);border-radius:8px;padding:12px 14px}
.card .k{color:var(--muted);font-size:13px}
.card .v{font-size:18px;font-weight:600;word-break:break-word}
table{width:100%;border-collapse:collapse}
th,td{text-align:left;padding:6px 8px;border-bottom:1px solid var(--line);vertical-align:top}
th{font-size:13px;color:var(--muted);font-weight:600}
td.n,th.n{text-align:right;white-space:nowrap}
code,pre{font:13px/1.45 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}
td code{word-break:break-all}
pre{background:#f6f8fa;border:1px solid var(--line);border-radius:8px;padding:12px;overflow-x:auto}
.bar{background:#eef1f4;border-radius:4px;height:10px;min-width:80px}
.bar span{display:block;height:10px;border-radius:4px;background:var(--bar)}
.tags span{display:inline-block;border:1px solid var(--line);border-radius:999px;padding:2px 10px;margin:0 6px 6px 0;font-size:13px}
.note{color:var(--muted);font-size:14px}
ul.files{padding-left:20px;columns:2;column-gap:32px}
ul.dead{padding-left:0;list-style:none}ul.dead li{margin:6px 0}
.conf{display:inline-block;min-width:56px;font-size:12px;font-weight:600;text-transform:uppercase}
.conf.high{color:#c0392b}.conf.medium{color:#b7791f}.conf.low{color:#888}
footer{margin-top:40px;color:var(--muted);font-size:13px}
@media print{main{padding:0;max-width:none}h2{break-after:avoid}tr,.card,pre{break-inside:avoid}pre{white-space:pre-wrap}}
@page{margin:16mm}
";

/// Render a report as a self-contained HTML document.
pub fn render_html(report: &Report) -> String {
    let p = &report.pack;
    let mut out = String::new();
    let title = format!("CodeDna report: {}", report.name);
    let _ = write!(
        out,
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'\">\n\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n\
         <meta name=\"generator\" content=\"CodeDna {}\">\n\
         <title>{}</title>\n<style>{STYLE}</style>\n</head>\n<body>\n<main>\n",
        env!("CARGO_PKG_VERSION"),
        esc(&title)
    );

    let _ = writeln!(out, "<h1>{}</h1>", esc(&report.name));
    let _ = writeln!(
        out,
        "<p class=\"sub\">{} · {}</p>",
        esc(&p.project_type),
        esc(&p.architecture)
    );

    let _ = writeln!(out, "<div class=\"cards\">");
    card(&mut out, "Lines of code", &p.total_loc.to_string());
    card(&mut out, "Files", &p.file_count.to_string());
    card(&mut out, "Languages", &p.languages.len().to_string());
    card(
        &mut out,
        "Possibly unused files",
        &p.dead_code_candidates.to_string(),
    );
    let _ = writeln!(out, "</div>");

    let _ = writeln!(out, "<h2>Stack</h2>");
    let _ = writeln!(out, "<table>");
    for (label, items) in [
        ("Frameworks", &p.frameworks),
        ("Databases", &p.databases),
        ("Infrastructure", &p.infrastructure),
    ] {
        let _ = write!(out, "<tr><th>{label}</th><td class=\"tags\">");
        if items.is_empty() {
            let _ = write!(out, "<span class=\"note\">none detected</span>");
        }
        for item in items {
            let _ = write!(out, "<span>{}</span>", esc(item));
        }
        let _ = writeln!(out, "</td></tr>");
    }
    let _ = writeln!(out, "</table>");

    if !p.languages.is_empty() {
        let _ = writeln!(out, "<h2>Languages</h2>");
        let _ = writeln!(
            out,
            "<table><tr><th>Language</th><th class=\"n\">LOC</th><th class=\"n\">Share</th><th></th></tr>"
        );
        for l in &p.languages {
            let _ = writeln!(
                out,
                "<tr><td>{}</td><td class=\"n\">{}</td><td class=\"n\">{}%</td>\
                 <td style=\"width:40%\"><div class=\"bar\"><span style=\"width:{}%\"></span></div></td></tr>",
                esc(&l.name),
                l.loc,
                l.percent,
                l.percent.min(100)
            );
        }
        let _ = writeln!(out, "</table>");
    }

    if !p.entry_points.is_empty() {
        let _ = writeln!(out, "<h2>Entry points</h2>");
        file_list(&mut out, &p.entry_points);
    }

    if !p.most_imported.is_empty() {
        let _ = writeln!(out, "<h2>Most imported files</h2>");
        let _ = writeln!(
            out,
            "<table><tr><th>File</th><th class=\"n\">Imported by</th></tr>"
        );
        for f in &p.most_imported {
            let _ = writeln!(
                out,
                "<tr><td><code>{}</code></td><td class=\"n\">{}</td></tr>",
                esc(&f.file),
                f.imported_by
            );
        }
        let _ = writeln!(out, "</table>");
    }

    if !p.largest_files.is_empty() {
        let _ = writeln!(out, "<h2>Largest files</h2>");
        let _ = writeln!(
            out,
            "<table><tr><th>File</th><th class=\"n\">LOC</th><th>Language</th></tr>"
        );
        for f in &p.largest_files {
            let _ = writeln!(
                out,
                "<tr><td><code>{}</code></td><td class=\"n\">{}</td><td>{}</td></tr>",
                esc(&f.file),
                f.loc,
                esc(&f.language)
            );
        }
        let _ = writeln!(out, "</table>");
    }

    if let Some(h) = report.hotspots.as_ref().filter(|h| !h.hotspots.is_empty()) {
        let _ = writeln!(out, "<h2>Hotspots</h2>");
        let _ = writeln!(out, "<p class=\"note\">{}</p>", esc(&hotspot_note(h)));
        let _ = writeln!(
            out,
            "<table><tr><th>File</th><th class=\"n\">Commits</th><th class=\"n\">LOC</th></tr>"
        );
        for f in &h.hotspots {
            let _ = writeln!(
                out,
                "<tr><td><code>{}</code></td><td class=\"n\">{}</td><td class=\"n\">{}</td></tr>",
                esc(&f.file),
                f.commits,
                f.loc
            );
        }
        let _ = writeln!(out, "</table>");
        if !h.coupling.is_empty() {
            let _ = writeln!(out, "<h3>Files that change together</h3>");
            let _ = writeln!(out, "<p class=\"note\">{}</p>", esc(COUPLING_NOTE));
            let _ = writeln!(
                out,
                "<table><tr><th>Files</th><th class=\"n\">Shared commits</th><th class=\"n\">Together</th></tr>"
            );
            for c in &h.coupling {
                let _ = writeln!(
                    out,
                    "<tr><td><code>{}</code> + <code>{}</code></td><td class=\"n\">{}</td><td class=\"n\">{}%</td></tr>",
                    esc(&c.file),
                    esc(&c.other),
                    c.shared,
                    c.percent
                );
            }
            let _ = writeln!(out, "</table>");
        }
    }

    if !report.dead_code.is_empty() {
        let _ = writeln!(out, "<h2>Possibly unused files</h2>");
        let _ = writeln!(
            out,
            "<p class=\"note\">{}</p>",
            esc(&dead_code_note(
                p.dead_code_candidates,
                &report.dead_summary
            ))
        );
        let _ = writeln!(out, "<ul class=\"dead\">");
        for f in &report.dead_code {
            let _ = writeln!(
                out,
                "<li><span class=\"conf {level}\">{level}</span> <code>{}</code><br><span class=\"note\">{}</span></li>",
                esc(&f.file),
                esc(&f.reason),
                level = f.confidence.as_str(),
            );
        }
        let _ = writeln!(out, "</ul>");
        if p.dead_code_candidates > report.dead_code.len() {
            let _ = writeln!(
                out,
                "<p class=\"note\">…and {} more.</p>",
                p.dead_code_candidates - report.dead_code.len()
            );
        }
    }

    if !p.map.is_empty() {
        let _ = writeln!(out, "<h2>Directory map</h2>");
        let _ = writeln!(out, "<pre>{}</pre>", esc(&p.map));
        if p.map_truncated {
            let _ = writeln!(out, "<p class=\"note\">Map trimmed.</p>");
        }
    }

    let _ = writeln!(
        out,
        "<footer>{}</footer>\n</main>\n</body>\n</html>",
        esc(&footer())
    );
    out
}

fn card(out: &mut String, key: &str, value: &str) {
    let _ = writeln!(
        out,
        "<div class=\"card\"><div class=\"k\">{}</div><div class=\"v\">{}</div></div>",
        esc(key),
        esc(value)
    );
}

fn file_list(out: &mut String, files: &[String]) {
    let _ = writeln!(out, "<ul class=\"files\">");
    for f in files {
        let _ = writeln!(out, "<li><code>{}</code></li>", esc(f));
    }
    let _ = writeln!(out, "</ul>");
}

/// Escape text for HTML element content and quoted attribute values.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Shared text
// ---------------------------------------------------------------------------

fn dead_code_note(count: usize, summary: &str) -> String {
    format!(
        "{count} file{} not reachable from a detected entry point ({summary}). High: no other \
         file mentions the name. Medium: the name appears elsewhere or modules load dynamically. \
         Low: probably loaded by convention or a dynamic import. Check before deleting anything.",
        if count == 1 { " is" } else { "s are" }
    )
}

const COUPLING_NOTE: &str = "Pairs of files usually changed in the same commit. \"Together\" is the share of the less active file's commits that also changed the other. A change to one often needs a change to both.";

fn hotspot_note(h: &HotspotReport) -> String {
    format!(
        "Large files that changed often in the last {} days ({} commit{}), ranked by commits × lines of code. \
         Bugs and refactoring effort usually gather here.",
        h.days,
        h.commits_scanned,
        if h.commits_scanned == 1 { "" } else { "s" }
    )
}

fn footer() -> String {
    format!(
        "Generated by CodeDna {} (https://github.com/crafteraadarsh/codedna). \
         Framework, architecture and unused-file results are heuristics.",
        env!("CARGO_PKG_VERSION")
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::FileInfo;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn sample(file: &str) -> AnalysisResult {
        let mut languages = HashMap::new();
        languages.insert("Rust".to_string(), 120);
        AnalysisResult {
            project_type: "CLI tool".to_string(),
            total_loc: 120,
            comment_lines: 0,
            blank_lines: 0,
            architecture_details: Default::default(),
            dead_code_details: Vec::new(),
            languages,
            frameworks: vec!["<b>Axum</b>".to_string()],
            databases: vec![],
            infrastructure: vec!["Docker".to_string()],
            architecture: "Monolith".to_string(),
            dead_code: vec![PathBuf::from(file)],
            dependency_graph: HashMap::new(),
            file_breakdown: vec![
                FileInfo {
                    file: PathBuf::from("./src/main.rs"),
                    loc: 100,
                    language: "Rust".to_string(),
                },
                FileInfo {
                    file: PathBuf::from(file),
                    loc: 20,
                    language: "Rust".to_string(),
                },
            ],
        }
    }

    const HOSTILE: &str = "./src/<script>alert(1)</script>|`x`.rs";

    #[test]
    fn html_escapes_repository_text() {
        let report = build("demo", &sample(HOSTILE), "root\n└── src\n");
        let html = render_html(&report);
        assert!(!html.contains("<script>"));
        assert!(!html.contains("<b>Axum"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("&lt;b&gt;Axum"));
        assert!(html.contains("Content-Security-Policy"));
        assert!(!html.contains("http-equiv=\"refresh\""));
    }

    #[test]
    fn html_has_no_external_resources() {
        let html = render_html(&build("demo", &sample("./src/a.rs"), "root\n"));
        for banned in ["<script", "<link", "<img", "<iframe", "src=", "@import"] {
            assert!(!html.contains(banned), "found {banned}");
        }
    }

    #[test]
    fn markdown_has_sections_and_escapes() {
        let report = build("demo_[x]", &sample(HOSTILE), "root\n└── src\n");
        let md = render_markdown(&report);
        assert!(md.starts_with("# CodeDna report: demo\\_\\[x\\]"));
        assert!(md.contains("## Languages"));
        assert!(md.contains("## Largest files"));
        assert!(md.contains("## Possibly unused files"));
        assert!(md.contains("## Directory map"));
        assert!(md.contains("\\<b\\>Axum"));
        // The hostile file name cannot break out of its code span or table.
        assert!(md.contains("`./src/<script>alert(1)</script>¦'x'.rs`"));
    }

    #[test]
    fn markdown_fence_outlasts_backticks_in_map() {
        let report = build("demo", &sample("./a.rs"), "root\n└── ```evil\n");
        let md = render_markdown(&report);
        assert!(md.contains("````text"));
    }

    #[test]
    fn shows_escaped_hotspots() {
        let mut report = build("demo", &sample("./a.rs"), "root\n");
        report.hotspots = Some(HotspotReport {
            days: 90,
            commits_scanned: 7,
            truncated: false,
            hotspots: vec![crate::hotspots::Hotspot {
                file: HOSTILE.to_string(),
                commits: 7,
                loc: 50,
                score: 350,
            }],
            coupling: vec![crate::hotspots::Coupling {
                file: HOSTILE.to_string(),
                other: "b.rs".to_string(),
                shared: 3,
                percent: 75,
            }],
        });
        let html = render_html(&report);
        assert!(html.contains("<h2>Hotspots</h2>"));
        assert!(!html.contains("<img"));
        let md = render_markdown(&report);
        assert!(md.contains("## Hotspots"));
        assert!(md.contains("last 90 days (7 commits)"));
        assert!(md.contains("### Files that change together"));
        assert!(html.contains("<h3>Files that change together</h3>"));
    }

    #[test]
    fn names_projects() {
        let here = Path::new(".");
        assert_eq!(
            project_name("https://github.com/user/repo.git", here),
            "repo"
        );
        assert_eq!(project_name("https://github.com/user/repo/", here), "repo");
        assert_eq!(project_name("git@github.com:user/repo.git", here), "repo");
        let local = project_name(".", here);
        assert!(!local.is_empty() && local != "project");
    }

    #[test]
    fn lists_at_most_fifty_unused_files() {
        let mut result = sample("./a.rs");
        result.dead_code = (0..80)
            .map(|i| PathBuf::from(format!("./f{i}.rs")))
            .collect();
        let report = build("demo", &result, "root\n");
        assert_eq!(report.dead_code.len(), DEAD_CODE_LIMIT);
        assert!(render_markdown(&report).contains("…and 30 more"));
    }
}
