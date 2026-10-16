use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// CodeDna — Rust-powered codebase intelligence engine.
#[derive(Debug, Parser)]
#[command(
    name = "codedna",
    version,
    about = "Analyze a repository and reveal its codebase DNA",
    long_about = None
)]
pub struct Cli {
    /// Read every file afresh instead of reusing results cached from an
    /// earlier run (also: CODEDNA_NO_CACHE=1)
    #[arg(long, global = true)]
    pub no_cache: bool,

    #[command(subcommand)]
    pub command: Commands,
}

/// Supported CodeDna commands.
#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Recursively scan repository files (respecting ignore rules)
    Scan {
        /// Target repository path or Git URL
        #[arg(default_value = ".")]
        path: String,
    },

    /// Show detected stack (languages, frameworks, databases)
    Stack {
        /// Target repository path or Git URL
        #[arg(default_value = ".")]
        path: String,
    },

    /// Show per-file LOC breakdown sorted by size
    Files {
        /// Target repository path or Git URL
        #[arg(default_value = ".")]
        path: String,
    },

    /// Show files where a specific framework is used
    Framework {
        /// Framework name (e.g. react, fastapi, express)
        name: String,

        /// Target repository path or Git URL
        #[arg(default_value = ".")]
        path: String,
    },

    /// Detect and list dead / unused code files
    Deadcode {
        /// Target repository path or Git URL
        #[arg(default_value = ".")]
        path: String,

        /// Hide the files this baseline accepts
        #[arg(long, value_name = "FILE", conflicts_with = "write_baseline")]
        baseline: Option<PathBuf>,

        /// Accept every current candidate: write them to this baseline
        #[arg(long, value_name = "FILE")]
        write_baseline: Option<PathBuf>,
    },

    /// List groups of files that import each other (circular imports)
    Cycles {
        /// Target repository path or Git URL
        #[arg(default_value = ".")]
        path: String,

        /// Output JSON: a list of cycles, each a list of files
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// List monorepo packages, each with its size, stack and the packages it uses
    Packages {
        /// Target repository path or Git URL
        #[arg(default_value = ".")]
        path: String,

        /// Output JSON: the workspaces and their packages
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// Run full intelligence analysis and print a formatted report
    Analyze {
        /// Target repository path or Git URL
        #[arg(default_value = ".")]
        path: String,

        /// Print elapsed wall-clock time after the report
        #[arg(long, default_value_t = false)]
        time: bool,
    },

    /// Output full analysis as machine-readable JSON
    Json {
        /// Target repository path or Git URL
        #[arg(default_value = ".")]
        path: String,

        /// Output compact single-line JSON instead of pretty-printed
        #[arg(long, default_value_t = false)]
        compact: bool,

        /// Print elapsed wall-clock time after output
        #[arg(long, default_value_t = false)]
        time: bool,
    },

    /// Render a tree-style directory map of the repository
    Map {
        /// Target repository path or Git URL
        #[arg(default_value = ".")]
        path: String,

        /// Maximum depth to render (default: 6)
        #[arg(long, default_value_t = crate::repo_map::DEFAULT_MAX_DEPTH)]
        depth: usize,
    },

    /// Print a compact briefing on the codebase for AI coding agents
    Context {
        /// Target repository path or Git URL
        #[arg(default_value = ".")]
        path: String,

        /// Approximate size limit in tokens
        #[arg(long, default_value_t = crate::context::DEFAULT_MAX_TOKENS)]
        max_tokens: usize,

        /// Output format
        #[arg(long, value_enum, default_value_t = ContextFormat::Md)]
        format: ContextFormat,
    },

    /// Write a shareable report (Markdown, or HTML you can print to PDF)
    Report {
        /// Target repository path or Git URL
        #[arg(default_value = ".")]
        path: String,

        /// Output format
        #[arg(long, value_enum, default_value_t = ReportFormat::Md)]
        format: ReportFormat,

        /// Write to this file instead of standard output
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,
    },

    /// Rank files that change often and are large (git churn × size)
    Hotspots {
        /// Target repository path or Git URL
        #[arg(default_value = ".")]
        path: String,

        /// Count commits from the last N days
        #[arg(long, default_value_t = crate::hotspots::DEFAULT_DAYS)]
        days: u64,

        /// Number of files to show
        #[arg(long, default_value_t = crate::hotspots::DEFAULT_LIMIT)]
        limit: usize,

        /// Output JSON instead of a table
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// Scan for supply-chain red flags: secrets, install scripts, obfuscation
    Audit {
        /// Target repository path or Git URL
        #[arg(default_value = ".")]
        path: String,

        /// Output JSON instead of a list
        #[arg(long, default_value_t = false)]
        json: bool,

        /// Exit with status 1 if any finding is at or above this severity
        #[arg(long, value_name = "LEVEL", value_parser = ["low", "medium", "high"])]
        fail_on: Option<String>,

        /// Also check lockfile versions for known vulnerabilities (sends
        /// package names and versions to osv.dev)
        #[arg(long, default_value_t = false)]
        deps: bool,

        /// Hide the findings this baseline accepts; --fail-on then applies
        /// only to new ones
        #[arg(long, value_name = "FILE", conflicts_with = "write_baseline")]
        baseline: Option<PathBuf>,

        /// Accept every current finding: write them to this baseline
        #[arg(long, value_name = "FILE")]
        write_baseline: Option<PathBuf>,
    },

    /// How to run a project: commands, environment variable names and API routes
    Runbook {
        /// Target repository path or Git URL
        #[arg(default_value = ".")]
        path: String,

        /// Output JSON instead of a list
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// What the deployment configuration says will run: Kubernetes, Helm,
    /// Compose, serverless functions and their triggers, linked to code
    Topology {
        /// Target repository path or Git URL
        #[arg(default_value = ".")]
        path: String,

        /// Output JSON instead of a list
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// Compare two versions of a project: stack, languages, size and files
    Diff {
        /// Earlier version: a snapshot saved with `codedna json`, a path or a Git URL
        before: String,

        /// Later version: a snapshot, a path or a Git URL
        #[arg(default_value = ".")]
        after: String,

        /// Output JSON instead of a summary
        #[arg(long, default_value_t = false, conflicts_with = "markdown")]
        json: bool,

        /// Output Markdown, for a pull request comment or CI job summary
        #[arg(long, default_value_t = false)]
        markdown: bool,
    },

    /// Show what a change affects: files that import it, layers, API routes and tests
    Impact {
        /// Folder inside a Git repository
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Compare with this commit or branch; for a branch, from where your work left it
        #[arg(long, default_value = crate::impact::DEFAULT_BASE)]
        base: String,

        /// Output JSON instead of a summary
        #[arg(long, default_value_t = false, conflicts_with = "markdown")]
        json: bool,

        /// Output Markdown, for a pull request comment or CI job summary
        #[arg(long, default_value_t = false)]
        markdown: bool,
    },

    /// Run an MCP server on stdio so AI agents can call CodeDna as tools
    Mcp {
        /// Folder the server may read (repeatable; default: current folder)
        #[arg(long = "root", value_name = "DIR")]
        roots: Vec<PathBuf>,
    },

    /// Print the JSON Schema of the `codedna json` report
    Schema,
}

/// How many hotspots a report lists.
const REPORT_HOTSPOTS: usize = 10;

/// Output formats for `codedna context`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ContextFormat {
    /// Markdown
    Md,
    /// JSON
    Json,
}

/// Output formats for `codedna report`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ReportFormat {
    /// Markdown
    Md,
    /// Self-contained HTML (print it to PDF from a browser)
    Html,
}

// ── Helper: resolve input (path or URL) ───────────────────────────────────────

/// Resolve a CLI `path` argument through the git handler.
///
/// If the input is a Git URL, this clones the repo and returns the local path.
/// The returned `_guard` **must** be kept alive for the duration of the command
/// — dropping it deletes the cloned repository.
fn resolve(input: &str) -> (PathBuf, Option<tempfile::TempDir>) {
    resolve_with_depth(input, 1)
}

/// Like [`resolve`], keeping the newest `depth` commits of a cloned repository.
fn resolve_with_depth(input: &str, depth: i32) -> (PathBuf, Option<tempfile::TempDir>) {
    match crate::git_handler::resolve_input_with_depth(input, depth) {
        Ok(pair) => pair,
        Err(msg) => {
            eprintln!("error: {msg}");
            std::process::exit(1);
        }
    }
}

/// The CLI input as it is safe to print: credentials removed, control
/// characters replaced.
fn shown(input: &str) -> String {
    crate::safe_text::sanitize(&crate::git_handler::redact_url(input))
}

/// The value, or exit with status 2 so a gate never mistakes a broken
/// baseline for findings.
fn load_or_exit<T>(result: Result<T, String>) -> T {
    result.unwrap_or_else(|msg| {
        eprintln!("error: {msg}");
        std::process::exit(2);
    })
}

/// Load a saved snapshot, or analyse a folder or repository.
fn load_side(input: &str) -> crate::analysis::AnalysisResult {
    if crate::diff::is_snapshot(input) {
        match crate::diff::load_snapshot(std::path::Path::new(input)) {
            Ok(result) => result,
            Err(msg) => {
                eprintln!("error: {msg}");
                std::process::exit(1);
            }
        }
    } else {
        eprintln!("Analyzing {}…", shown(input));
        let (local_path, _guard) = resolve(input);
        crate::analysis::analyze(&local_path)
    }
}

/// Parse CLI arguments and dispatch to the appropriate module.
pub fn run() {
    let cli = Cli::parse();
    if !cli.no_cache {
        crate::cache::enable();
    }

    match cli.command {
        // ── scan ─────────────────────────────────────────────────────────────
        Commands::Scan { path } => {
            let (local_path, _guard) = resolve(&path);
            let files = crate::scanner::scan_repository(&local_path);
            println!();
            println!(
                "  Scanned {} file{} in '{}'",
                files.len(),
                if files.len() == 1 { "" } else { "s" },
                shown(&path)
            );
            println!();
            for f in &files {
                println!("  {}", f.display());
            }
            println!();
        }

        // ── stack ─────────────────────────────────────────────────────────────
        Commands::Stack { path } => {
            eprintln!("Analysing '{}'…", shown(&path));
            let (local_path, _guard) = resolve(&path);
            let result = crate::analysis::analyze(&local_path);
            crate::reporter::print_stack(&result);
        }

        // ── files ─────────────────────────────────────────────────────────────
        Commands::Files { path } => {
            eprintln!("Analysing '{}'…", shown(&path));
            let (local_path, _guard) = resolve(&path);
            let result = crate::analysis::analyze(&local_path);
            crate::reporter::print_files(&result);
        }

        // ── framework ─────────────────────────────────────────────────────────
        Commands::Framework { name, path } => {
            eprintln!("Analysing '{}'…", shown(&path));
            let (local_path, _guard) = resolve(&path);
            let files = crate::scanner::scan_repository(&local_path);
            let matching = crate::framework_detector::detect_files_using_framework(&files, &name);
            crate::reporter::print_framework_usage(&name, &matching);
        }

        // ── deadcode ──────────────────────────────────────────────────────────
        Commands::Deadcode {
            path,
            baseline,
            write_baseline,
        } => {
            eprintln!("Analysing '{}'…", shown(&path));
            let (local_path, _guard) = resolve(&path);
            let mut result = crate::analysis::analyze(&local_path);
            if let Some(file) = write_baseline {
                let mut accepted = load_or_exit(crate::baseline::Baseline::load_or_default(&file));
                let files: Vec<String> = result
                    .dead_code
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                accepted.set_dead_code(files.iter().map(String::as_str));
                load_or_exit(accepted.save(&file));
                eprintln!(
                    "Wrote {} dead-code candidate{} to {}",
                    files.len(),
                    if files.len() == 1 { "" } else { "s" },
                    shown(&file.to_string_lossy())
                );
                return;
            }
            let mut applied = None;
            if let Some(file) = baseline {
                let accepted = load_or_exit(crate::baseline::Baseline::load(&file));
                let files: Vec<String> = result
                    .dead_code
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                let stale = accepted.stale_dead(files.iter().map(String::as_str));
                let before = result.dead_code.len();
                result
                    .dead_code
                    .retain(|p| !accepted.accepts_dead(&p.to_string_lossy()));
                result
                    .dead_code_details
                    .retain(|c| !accepted.accepts_dead(&c.file.to_string_lossy()));
                applied = Some(crate::baseline::Applied {
                    hidden: before - result.dead_code.len(),
                    stale,
                });
            }
            crate::reporter::print_dead_code(&result);
            if let Some(applied) = applied {
                println!("{}", crate::baseline::summary(applied, "candidate"));
            }
        }

        // ── cycles ────────────────────────────────────────────────────────────
        Commands::Cycles { path, json } => {
            eprintln!("Analysing '{}'…", shown(&path));
            let (local_path, _guard) = resolve(&path);
            let result = crate::analysis::analyze(&local_path);
            if json {
                match serde_json::to_string_pretty(&result.circular_imports) {
                    Ok(out) => println!("{out}"),
                    Err(e) => eprintln!("error: JSON serialisation failed: {e}"),
                }
            } else {
                crate::reporter::print_cycles(&result);
            }
        }

        // ── packages ──────────────────────────────────────────────────────────
        Commands::Packages { path, json } => {
            eprintln!("Analysing '{}'…", shown(&path));
            let (local_path, _guard) = resolve(&path);
            let result = crate::analysis::analyze(&local_path);
            if json {
                match serde_json::to_string_pretty(&result.workspaces) {
                    Ok(out) => println!("{out}"),
                    Err(e) => eprintln!("error: JSON serialisation failed: {e}"),
                }
            } else {
                crate::reporter::print_packages(&result);
            }
        }

        // ── analyze ───────────────────────────────────────────────────────────
        Commands::Analyze { path, time } => {
            eprintln!("Analysing '{}'…", shown(&path));
            let t0 = std::time::Instant::now();
            let (local_path, _guard) = resolve(&path);
            let result = crate::analysis::analyze(&local_path);
            crate::reporter::print_report(&result);
            if time {
                eprintln!("\n  ⏱  Completed in {:.2?}", t0.elapsed());
            }
        }

        // ── json ──────────────────────────────────────────────────────────────
        Commands::Json {
            path,
            compact,
            time,
        } => {
            let t0 = std::time::Instant::now();
            let (local_path, _guard) = resolve(&path);
            let result = crate::analysis::analyze(&local_path);
            crate::reporter::print_json(&result, compact);
            if time {
                eprintln!("\n  ⏱  Completed in {:.2?}", t0.elapsed());
            }
        }

        // ── map ───────────────────────────────────────────────────────────────
        Commands::Map { path, depth } => {
            let (local_path, _guard) = resolve(&path);
            let tree = crate::repo_map::render_tree(&local_path, depth);
            println!();
            print!("{tree}");
            println!();
        }

        // ── context ───────────────────────────────────────────────────────────
        Commands::Context {
            path,
            max_tokens,
            format,
        } => {
            let (local_path, _guard) = resolve(&path);
            let result = crate::analysis::analyze(&local_path);
            let tree = crate::repo_map::render_tree(&local_path, crate::context::MAP_DEPTH);
            let pack = crate::context::build(&result, &tree, max_tokens);
            match format {
                ContextFormat::Md => print!("{}", crate::context::render_markdown(&pack)),
                ContextFormat::Json => match serde_json::to_string_pretty(&pack) {
                    Ok(json) => println!("{json}"),
                    Err(e) => eprintln!("error: JSON serialisation failed: {e}"),
                },
            }
        }

        // ── report ────────────────────────────────────────────────────────────
        Commands::Report {
            path,
            format,
            output,
        } => {
            eprintln!("Analysing '{}'…", shown(&path));
            let (local_path, _guard) = resolve(&path);
            let result = crate::analysis::analyze(&local_path);
            let tree = crate::repo_map::render_tree(&local_path, crate::report::MAP_DEPTH);
            let name = crate::report::project_name(&path, &local_path);
            let mut report = crate::report::build(&name, &result, &tree);
            report.hotspots = crate::hotspots::analyze(
                &local_path,
                &result,
                crate::hotspots::DEFAULT_DAYS,
                REPORT_HOTSPOTS,
            )
            .ok();
            let text = match format {
                ReportFormat::Md => crate::report::render_markdown(&report),
                ReportFormat::Html => crate::report::render_html(&report),
            };
            match output {
                None => print!("{text}"),
                Some(file) => {
                    if let Err(e) = std::fs::write(&file, text) {
                        eprintln!(
                            "error: could not write {}: {e}",
                            crate::safe_text::sanitize(&file.to_string_lossy())
                        );
                        std::process::exit(1);
                    }
                    eprintln!(
                        "Wrote {}",
                        crate::safe_text::sanitize(&file.to_string_lossy())
                    );
                }
            }
        }

        // ── hotspots ──────────────────────────────────────────────────────────
        Commands::Hotspots {
            path,
            days,
            limit,
            json,
        } => {
            eprintln!("Analysing {}…", shown(&path));
            let (local_path, _guard) = resolve_with_depth(&path, crate::hotspots::CLONE_DEPTH);
            let result = crate::analysis::analyze(&local_path);
            match crate::hotspots::analyze(&local_path, &result, days, limit) {
                Ok(report) if json => match serde_json::to_string_pretty(&report) {
                    Ok(text) => println!("{text}"),
                    Err(e) => eprintln!("error: JSON serialisation failed: {e}"),
                },
                Ok(report) => print!("{}", crate::hotspots::render_text(&report)),
                Err(msg) => {
                    eprintln!("error: {msg}");
                    std::process::exit(1);
                }
            }
        }

        // ── audit ─────────────────────────────────────────────────────────────
        Commands::Audit {
            path,
            json,
            fail_on,
            deps,
            baseline,
            write_baseline,
        } => {
            eprintln!("Auditing {}…", shown(&path));
            let (local_path, _guard) = resolve(&path);
            let osv = deps.then(crate::vulns::OsvApi::new);
            let source = osv.as_ref().map(|s| s as &dyn crate::vulns::Source);
            let scan = crate::audit::scan(&local_path, source);
            let accepted = match (&baseline, &write_baseline) {
                (Some(file), _) => Some(load_or_exit(crate::baseline::Baseline::load(file))),
                (None, Some(file)) => {
                    let mut accepted =
                        load_or_exit(crate::baseline::Baseline::load_or_default(file));
                    accepted.set_audit(&scan.findings);
                    load_or_exit(accepted.save(file));
                    eprintln!(
                        "Wrote {} finding{} to {}",
                        accepted.audit.len(),
                        if accepted.audit.len() == 1 { "" } else { "s" },
                        shown(&file.to_string_lossy())
                    );
                    Some(accepted)
                }
                (None, None) => None,
            };
            let report = scan.report(accepted.as_ref());
            if json {
                match serde_json::to_string_pretty(&report) {
                    Ok(text) => println!("{text}"),
                    Err(e) => eprintln!("error: JSON serialisation failed: {e}"),
                }
            } else {
                print!("{}", crate::audit::render_text(&report));
            }
            let threshold = fail_on
                .as_deref()
                .and_then(crate::audit::Severity::from_name);
            if threshold.is_some_and(|level| report.has_at_least(level)) {
                std::process::exit(1);
            }
            // A gate must not pass just because the database was down.
            if threshold.is_some()
                && report
                    .dependencies
                    .as_ref()
                    .is_some_and(|d| d.error.is_some())
            {
                eprintln!("error: dependencies could not be checked");
                std::process::exit(2);
            }
        }

        // ── runbook ───────────────────────────────────────────────────────────
        Commands::Runbook { path, json } => {
            eprintln!("Reading {}…", shown(&path));
            let (local_path, _guard) = resolve(&path);
            let book = crate::runbook::runbook(&local_path);
            if json {
                match serde_json::to_string_pretty(&book) {
                    Ok(text) => println!("{text}"),
                    Err(e) => eprintln!("error: JSON serialisation failed: {e}"),
                }
            } else {
                print!("{}", crate::runbook::render_text(&book));
            }
        }

        // ── topology ──────────────────────────────────────────────────────────
        Commands::Topology { path, json } => {
            eprintln!("Reading {}…", shown(&path));
            let (local_path, _guard) = resolve(&path);
            let topology = crate::topology::topology(&local_path);
            if json {
                match serde_json::to_string_pretty(&topology) {
                    Ok(text) => println!("{text}"),
                    Err(e) => eprintln!("error: JSON serialisation failed: {e}"),
                }
            } else {
                print!("{}", crate::topology::render_text(&topology));
            }
        }

        // ── diff ──────────────────────────────────────────────────────────────
        Commands::Diff {
            before,
            after,
            json,
            markdown,
        } => {
            let old = load_side(&before);
            let new = load_side(&after);
            let d = crate::diff::diff(&old, &new);
            if json {
                match serde_json::to_string_pretty(&d) {
                    Ok(text) => println!("{text}"),
                    Err(e) => eprintln!("error: JSON serialisation failed: {e}"),
                }
            } else if markdown {
                print!("{}", crate::diff::render_markdown(&d));
            } else {
                print!("{}", crate::diff::render_text(&d));
            }
        }

        // ── impact ────────────────────────────────────────────────────────────
        Commands::Impact {
            path,
            base,
            json,
            markdown,
        } => match crate::impact::analyze(&path, &base) {
            Ok(impact) if json => match serde_json::to_string_pretty(&impact) {
                Ok(text) => println!("{text}"),
                Err(e) => eprintln!("error: JSON serialisation failed: {e}"),
            },
            Ok(impact) if markdown => print!("{}", crate::impact::render_markdown(&impact)),
            Ok(impact) => print!("{}", crate::impact::render_text(&impact)),
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        },

        // ── mcp ───────────────────────────────────────────────────────────────
        Commands::Mcp { roots } => {
            if let Err(e) = crate::mcp::serve(&roots) {
                eprintln!("error: {}", crate::safe_text::sanitize(&e.to_string()));
                std::process::exit(1);
            }
        }

        // ── schema ────────────────────────────────────────────────────────────
        Commands::Schema => {
            println!("{}", crate::analysis::json_schema());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_report_with_options() {
        let cli =
            Cli::try_parse_from(["codedna", "report", ".", "--format", "html", "-o", "r.html"])
                .unwrap();
        match cli.command {
            Commands::Report {
                path,
                format,
                output,
            } => {
                assert_eq!(path, ".");
                assert_eq!(format, ReportFormat::Html);
                assert_eq!(output, Some(PathBuf::from("r.html")));
            }
            _ => panic!("expected report command"),
        }
    }

    #[test]
    fn parses_hotspots_with_defaults_and_options() {
        let cli = Cli::try_parse_from(["codedna", "hotspots"]).unwrap();
        match cli.command {
            Commands::Hotspots {
                path,
                days,
                limit,
                json,
            } => {
                assert_eq!(path, ".");
                assert_eq!(days, crate::hotspots::DEFAULT_DAYS);
                assert_eq!(limit, crate::hotspots::DEFAULT_LIMIT);
                assert!(!json);
            }
            _ => panic!("expected hotspots command"),
        }
        let cli = Cli::try_parse_from([
            "codedna", "hotspots", "x", "--days", "30", "--limit", "5", "--json",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Hotspots {
                days: 30,
                limit: 5,
                json: true,
                ..
            }
        ));
    }

    #[test]
    fn parses_audit() {
        let cli = Cli::try_parse_from(["codedna", "audit"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Audit {
                json: false,
                fail_on: None,
                deps: false,
                ..
            }
        ));
        let cli = Cli::try_parse_from([
            "codedna",
            "audit",
            "x",
            "--json",
            "--fail-on",
            "high",
            "--deps",
        ])
        .unwrap();
        match cli.command {
            Commands::Audit {
                path,
                json,
                fail_on,
                deps,
                baseline,
                write_baseline,
            } => {
                assert_eq!(path, "x");
                assert!(json);
                assert!(deps);
                assert_eq!(fail_on.as_deref(), Some("high"));
                assert_eq!(baseline, None);
                assert_eq!(write_baseline, None);
            }
            _ => panic!("expected audit command"),
        }
        assert!(Cli::try_parse_from(["codedna", "audit", "--fail-on", "severe"]).is_err());
    }

    #[test]
    fn parses_baselines() {
        let cli = Cli::try_parse_from(["codedna", "audit", "--baseline", "b.json"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Audit { baseline: Some(ref b), .. } if b == &PathBuf::from("b.json")
        ));
        let cli =
            Cli::try_parse_from(["codedna", "deadcode", "--write-baseline", "b.json"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Deadcode {
                write_baseline: Some(_),
                baseline: None,
                ..
            }
        ));
        for command in ["audit", "deadcode"] {
            let both = [
                "codedna",
                command,
                "--baseline",
                "a.json",
                "--write-baseline",
                "b.json",
            ];
            assert!(Cli::try_parse_from(both).is_err());
        }
    }

    #[test]
    fn parses_impact() {
        let cli = Cli::try_parse_from(["codedna", "impact"]).unwrap();
        match cli.command {
            Commands::Impact {
                path,
                base,
                json,
                markdown,
            } => {
                assert_eq!(path, PathBuf::from("."));
                assert_eq!(base, "HEAD");
                assert!(!json && !markdown);
            }
            _ => panic!("expected impact"),
        }
        let cli = Cli::try_parse_from([
            "codedna",
            "impact",
            "app",
            "--base",
            "origin/main",
            "--json",
        ])
        .unwrap();
        match cli.command {
            Commands::Impact {
                path, base, json, ..
            } => {
                assert_eq!(path, PathBuf::from("app"));
                assert_eq!(base, "origin/main");
                assert!(json);
            }
            _ => panic!("expected impact"),
        }
        assert!(Cli::try_parse_from(["codedna", "impact", "--json", "--markdown"]).is_err());
    }

    #[test]
    fn parses_runbook() {
        let cli = Cli::try_parse_from(["codedna", "runbook"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Runbook { ref path, json: false } if path == "."
        ));
        let cli = Cli::try_parse_from(["codedna", "runbook", "x", "--json"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Runbook { ref path, json: true } if path == "x"
        ));
    }

    #[test]
    fn parses_topology() {
        let cli = Cli::try_parse_from(["codedna", "topology", "x", "--json"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Topology { ref path, json: true } if path == "x"
        ));
    }

    #[test]
    fn parses_diff() {
        let cli = Cli::try_parse_from(["codedna", "diff", "old.json"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Diff { ref before, ref after, json: false, markdown: false } if before == "old.json" && after == "."
        ));
        let cli = Cli::try_parse_from(["codedna", "diff", "a", "b", "--json"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Diff { ref before, ref after, json: true, .. } if before == "a" && after == "b"
        ));
        let cli = Cli::try_parse_from(["codedna", "diff", "a", "--markdown"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Diff {
                markdown: true,
                json: false,
                ..
            }
        ));
        assert!(Cli::try_parse_from(["codedna", "diff", "a", "--json", "--markdown"]).is_err());
        assert!(Cli::try_parse_from(["codedna", "diff"]).is_err());
    }

    #[test]
    fn parses_analyze_with_time_flag() {
        let cli = Cli::try_parse_from(["codedna", "analyze", ".", "--time"]).unwrap();

        match cli.command {
            Commands::Analyze { path, time } => {
                assert_eq!(path, ".");
                assert!(time);
            }
            _ => panic!("expected analyze command"),
        }
    }

    #[test]
    fn parses_json_with_compact_and_time_flags() {
        let cli = Cli::try_parse_from(["codedna", "json", ".", "--compact", "--time"]).unwrap();

        match cli.command {
            Commands::Json {
                path,
                compact,
                time,
            } => {
                assert_eq!(path, ".");
                assert!(compact);
                assert!(time);
            }
            _ => panic!("expected json command"),
        }
    }

    #[test]
    fn parses_map_with_custom_depth() {
        let cli = Cli::try_parse_from(["codedna", "map", ".", "--depth", "3"]).unwrap();

        match cli.command {
            Commands::Map { path, depth } => {
                assert_eq!(path, ".");
                assert_eq!(depth, 3);
            }
            _ => panic!("expected map command"),
        }
    }

    #[test]
    fn parses_map_with_default_depth() {
        let cli = Cli::try_parse_from(["codedna", "map", "."]).unwrap();

        match cli.command {
            Commands::Map { path, depth } => {
                assert_eq!(path, ".");
                assert_eq!(depth, crate::repo_map::DEFAULT_MAX_DEPTH);
            }
            _ => panic!("expected map command"),
        }
    }

    #[test]
    fn parses_context_with_options() {
        let cli = Cli::try_parse_from([
            "codedna",
            "context",
            ".",
            "--max-tokens",
            "800",
            "--format",
            "json",
        ])
        .unwrap();

        match cli.command {
            Commands::Context {
                path,
                max_tokens,
                format,
            } => {
                assert_eq!(path, ".");
                assert_eq!(max_tokens, 800);
                assert_eq!(format, ContextFormat::Json);
            }
            _ => panic!("expected context command"),
        }
    }

    #[test]
    fn parses_mcp_with_roots() {
        let cli = Cli::try_parse_from(["codedna", "mcp", "--root", "a", "--root", "b"]).unwrap();

        match cli.command {
            Commands::Mcp { roots } => {
                assert_eq!(roots, vec![PathBuf::from("a"), PathBuf::from("b")]);
            }
            _ => panic!("expected mcp command"),
        }
    }

    #[test]
    fn parses_analyze_with_git_url() {
        let cli =
            Cli::try_parse_from(["codedna", "analyze", "https://github.com/user/repo"]).unwrap();

        match cli.command {
            Commands::Analyze { path, time } => {
                assert_eq!(path, "https://github.com/user/repo");
                assert!(!time);
            }
            _ => panic!("expected analyze command"),
        }
    }

    #[test]
    fn parses_stack_with_git_url() {
        let cli =
            Cli::try_parse_from(["codedna", "stack", "git@github.com:user/repo.git"]).unwrap();

        match cli.command {
            Commands::Stack { path } => {
                assert_eq!(path, "git@github.com:user/repo.git");
            }
            _ => panic!("expected stack command"),
        }
    }
}
