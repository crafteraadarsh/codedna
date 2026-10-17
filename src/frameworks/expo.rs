//! Expo Router: every module in the routes directory is a route or a
//! layout.
//!
//! Active in a package whose own `package.json` depends on `expo-router`.
//! The routes directory is the `root` option of the `expo-router` plugin in
//! `app.json` when it is a literal, else `app/`, else `src/app/`.

use super::{has_dir, is_js, Adapters, Package, Repo};
use crate::facts::Certainty;
use std::fs;
use std::path::PathBuf;

pub const VERSION: u32 = 1;
const RULE: &str = "expo.router@v1";

pub fn detect(_repo: &Repo, package: &Package, out: &mut Adapters) {
    let Some(line) = package.depends_on("expo-router") else {
        return;
    };
    let app_json = package.config(&["app.json"]).cloned();
    let configured = app_json
        .as_ref()
        .and_then(|f| fs::read_to_string(f).ok())
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|json| plugin_root(&json));
    let routes: Option<PathBuf> = match configured {
        Some(root) => Some(package.dir.join(root.trim_start_matches("./"))),
        None => [package.dir.join("app"), package.dir.join("src").join("app")]
            .into_iter()
            .find(|dir| has_dir(package, dir)),
    };
    if let Some(routes) = routes {
        for file in package
            .files
            .iter()
            .filter(|f| f.starts_with(&routes) && is_js(f))
        {
            out.entries
                .push(package.entry(file, RULE, Certainty::Inferred, line));
        }
    }
    let config = app_json.into_iter().collect();
    out.active
        .push(package.adapter("expo_router", VERSION, line, config));
}

/// `expo.plugins: [["expo-router", { "root": "./src/routes" }]]`.
fn plugin_root(json: &serde_json::Value) -> Option<String> {
    let plugins = json.pointer("/expo/plugins")?.as_array()?;
    plugins.iter().find_map(|plugin| {
        let entry = plugin.as_array()?;
        (entry.first()?.as_str()? == "expo-router")
            .then(|| entry.get(1)?.get("root")?.as_str().map(str::to_string))
            .flatten()
    })
}
