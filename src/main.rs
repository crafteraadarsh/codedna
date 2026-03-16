mod analysis;
mod cli;
mod dead_code_detector;
mod dependency_graph;
mod framework_detector;
mod language_detector;
mod loc_counter;
mod repo_map;
mod reporter;
mod scanner;

fn main() {
    cli::run();
}
