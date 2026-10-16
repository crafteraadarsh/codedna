mod analysis;
mod architecture;
mod audit;
mod cli;
mod context;
mod coverage;
mod cycles;
mod dead_code_detector;
mod dependency_graph;
mod diff;
mod framework_detector;
mod git_handler;
mod hotspots;
mod language_detector;
mod loc_counter;
mod mcp;
mod repo_map;
mod report;
mod reporter;
mod rules;
mod runbook;
mod safe_text;
mod scanner;
mod vulns;
mod workspaces;

fn main() {
    cli::run();
}
