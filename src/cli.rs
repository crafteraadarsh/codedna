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
        /// Target repository path
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Show detected stack (languages, frameworks, databases)
    Stack {
        /// Target repository path
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Show per-file LOC breakdown sorted by size
    Files {
        /// Target repository path
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Show files where a specific framework is used
    Framework {
        /// Framework name (e.g. react, fastapi, express)
        name: String,

        /// Target repository path
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Detect and list dead / unused code files
    Deadcode {
        /// Target repository path
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Run full intelligence analysis and print a formatted report
    Analyze {
        /// Target repository path
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Print elapsed wall-clock time after the report
        #[arg(long, default_value_t = false)]
        time: bool,
    },

    /// Output full analysis as machine-readable JSON
    Json {
        /// Target repository path
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output compact single-line JSON instead of pretty-printed
        #[arg(long, default_value_t = false)]
        compact: bool,

        /// Print elapsed wall-clock time after output
        #[arg(long, default_value_t = false)]
        time: bool,
    },

    /// Render a tree-style directory map of the repository
    Map {
        /// Target repository path
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Maximum depth to render (default: 6)
        #[arg(long, default_value_t = crate::repo_map::DEFAULT_MAX_DEPTH)]
        depth: usize,
    },
}

/// Parse CLI arguments and dispatch to the appropriate module.
pub fn run() {
    let cli = Cli::parse();

    match cli.command {
        // ── scan ─────────────────────────────────────────────────────────────
        Commands::Scan { path } => {
            let files = crate::scanner::scan_repository(&path);
            println!();
            println!(
                "  Scanned {} file{} in '{}'",
                files.len(),
                if files.len() == 1 { "" } else { "s" },
                path.display()
            );
            println!();
            for f in &files {
                println!("  {}", f.display());
            }
            println!();
        }

        // ── stack ─────────────────────────────────────────────────────────────
        Commands::Stack { path } => {
            eprintln!("Analysing '{}'…", path.display());
            let result = crate::analysis::analyze(&path);
            crate::reporter::print_stack(&result);
        }

        // ── files ─────────────────────────────────────────────────────────────
        Commands::Files { path } => {
            eprintln!("Analysing '{}'…", path.display());
            let result = crate::analysis::analyze(&path);
            crate::reporter::print_files(&result);
        }

        // ── framework ─────────────────────────────────────────────────────────
        Commands::Framework { name, path } => {
            eprintln!("Analysing '{}'…", path.display());
            let files = crate::scanner::scan_repository(&path);
            let matching = crate::framework_detector::detect_files_using_framework(&files, &name);
            crate::reporter::print_framework_usage(&name, &matching);
        }

        // ── deadcode ──────────────────────────────────────────────────────────
        Commands::Deadcode { path } => {
            eprintln!("Analysing '{}'…", path.display());
            let result = crate::analysis::analyze(&path);
            crate::reporter::print_dead_code(&result);
        }

        // ── analyze ───────────────────────────────────────────────────────────
        Commands::Analyze { path, time } => {
            eprintln!("Analysing '{}'…", path.display());
            let t0 = std::time::Instant::now();
            let result = crate::analysis::analyze(&path);
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
            let result = crate::analysis::analyze(&path);
            crate::reporter::print_json(&result, compact);
            if time {
                eprintln!("\n  ⏱  Completed in {:.2?}", t0.elapsed());
            }
        }

        // ── map ───────────────────────────────────────────────────────────────
        Commands::Map { path, depth } => {
            let tree = crate::repo_map::render_tree(&path, depth);
            println!();
            print!("{tree}");
            println!();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_analyze_with_time_flag() {
        let cli = Cli::try_parse_from(["codedna", "analyze", ".", "--time"]).unwrap();

        match cli.command {
            Commands::Analyze { path, time } => {
                assert_eq!(path, PathBuf::from("."));
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
                assert_eq!(path, PathBuf::from("."));
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
                assert_eq!(path, PathBuf::from("."));
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
                assert_eq!(path, PathBuf::from("."));
                assert_eq!(depth, crate::repo_map::DEFAULT_MAX_DEPTH);
            }
            _ => panic!("expected map command"),
        }
    }
}
