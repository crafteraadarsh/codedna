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

    /// Run an MCP server on stdio so AI agents can call CodeDna as tools
    Mcp {
        /// Folder the server may read (repeatable; default: current folder)
        #[arg(long = "root", value_name = "DIR")]
        roots: Vec<PathBuf>,
    },
}

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
    match crate::git_handler::resolve_input(input) {
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

/// Parse CLI arguments and dispatch to the appropriate module.
pub fn run() {
    let cli = Cli::parse();

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
        Commands::Deadcode { path } => {
            eprintln!("Analysing '{}'…", shown(&path));
            let (local_path, _guard) = resolve(&path);
            let result = crate::analysis::analyze(&local_path);
            crate::reporter::print_dead_code(&result);
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
            let report = crate::report::build(&name, &result, &tree);
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

        // ── mcp ───────────────────────────────────────────────────────────────
        Commands::Mcp { roots } => {
            if let Err(e) = crate::mcp::serve(&roots) {
                eprintln!("error: {}", crate::safe_text::sanitize(&e.to_string()));
                std::process::exit(1);
            }
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
