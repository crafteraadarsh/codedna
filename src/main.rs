mod analysis;
mod cli;
mod context;
mod dead_code_detector;
mod dependency_graph;
mod framework_detector;
mod git_handler;
mod language_detector;
mod loc_counter;
mod mcp;
mod repo_map;
mod report;
mod reporter;
mod rules;
mod safe_text;
mod scanner;

fn main() {
    cli::run();
}
