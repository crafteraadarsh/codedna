//! Next.js: the App Router (`app/`) and Pages Router (`pages/`) files,
//! including route groups, and the special files Next loads from the
//! project root (`middleware`, `instrumentation`, `proxy`) and its config.
//!
//! Active in a package whose own `package.json` depends on `next`.

use super::{config_strings, has_dir, Adapters, Package, Repo};
use crate::facts::Certainty;
use std::fs;
use std::path::{Path, PathBuf};

pub const VERSION: u32 = 1;
const APP_RULE: &str = "next.app_router@v1";
const PAGES_RULE: &str = "next.pages_router@v1";
const SPECIAL_RULE: &str = "next.special_file@v1";
const CONFIG_RULE: &str = "next.config@v1";

const CONFIG_FILES: &[&str] = &[
    "next.config.js",
    "next.config.mjs",
    "next.config.cjs",
    "next.config.ts",
    "next.config.mts",
];

/// What Next loads from `pageExtensions` when the config does not say.
const DEFAULT_PAGE_EXTENSIONS: &[&str] = &["tsx", "ts", "jsx", "js"];

/// File stems the App Router loads from a route segment.
const APP_STEMS: &[&str] = &[
    "layout",
    "page",
    "route",
    "loading",
    "error",
    "global-error",
    "global-not-found",
    "not-found",
    "template",
    "default",
    "forbidden",
    "unauthorized",
    "sitemap",
    "robots",
    "manifest",
];

/// Metadata image routes, which may carry a number: `icon1.tsx`.
const NUMBERED_STEMS: &[&str] = &["icon", "apple-icon", "opengraph-image", "twitter-image"];

/// Files Next loads from the project root, or from `src/`.
const SPECIAL_STEMS: &[&str] = &[
    "middleware",
    "instrumentation",
    "instrumentation-client",
    "proxy",
];

pub fn detect(_repo: &Repo, package: &Package, out: &mut Adapters) {
    let Some(line) = package.depends_on("next") else {
        return;
    };
    let configs: Vec<PathBuf> = package
        .files
        .iter()
        .filter(|f| {
            f.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| CONFIG_FILES.contains(&n))
        })
        .cloned()
        .collect();
    // The package directory, and any folder below it with its own config.
    let mut roots: Vec<PathBuf> = vec![package.dir.clone()];
    for config in &configs {
        if let Some(dir) = config.parent() {
            if !roots.iter().any(|r| r == dir) {
                roots.push(dir.to_path_buf());
            }
        }
    }

    for root in &roots {
        let config = configs.iter().find(|c| c.parent() == Some(root.as_path()));
        let extensions: Vec<String> = config
            .and_then(|c| fs::read_to_string(c).ok())
            .and_then(|text| config_strings(&text, "pageExtensions"))
            .filter(|exts| !exts.is_empty())
            .unwrap_or_else(|| {
                DEFAULT_PAGE_EXTENSIONS
                    .iter()
                    .map(|e| e.to_string())
                    .collect()
            });

        // `app/` and `pages/` at the root win over `src/app` and `src/pages`.
        let (app, pages) = (root.join("app"), root.join("pages"));
        let base = if has_dir(package, &app) || has_dir(package, &pages) {
            root.clone()
        } else {
            root.join("src")
        };
        let (app, pages) = (base.join("app"), base.join("pages"));

        for file in package.files.iter() {
            if !file.starts_with(root) || nested_root(&roots, root, file) {
                continue;
            }
            let Some(stem) = page_stem(file, &extensions) else {
                continue;
            };
            let rule = if let Ok(rest) = file.strip_prefix(&app) {
                let private = rest
                    .parent()
                    .is_some_and(|dir| dir.iter().any(|c| c.to_string_lossy().starts_with('_')));
                (!private && is_app_stem(&stem)).then_some(APP_RULE)
            } else if file.starts_with(&pages) {
                Some(PAGES_RULE)
            } else {
                let dir = file.parent().unwrap_or(Path::new(""));
                ((dir == root.as_path() || dir == root.join("src"))
                    && SPECIAL_STEMS.contains(&stem.as_str()))
                .then_some(SPECIAL_RULE)
            };
            if let Some(rule) = rule {
                out.entries
                    .push(package.entry(file, rule, Certainty::Inferred, line));
            }
        }
        if let Some(config) = config {
            out.entries
                .push(package.entry(config, CONFIG_RULE, Certainty::Inferred, line));
        }
        out.next_roots.push(root.clone());
    }
    out.active
        .push(package.adapter("next", VERSION, line, configs.clone()));
}

/// Whether `file` belongs to a root nested inside `root`.
fn nested_root(roots: &[PathBuf], root: &Path, file: &Path) -> bool {
    roots
        .iter()
        .any(|r| r != root && r.starts_with(root) && file.starts_with(r))
}

/// The file's stem when its extension is one of the page extensions:
/// `page` for `page.tsx`, also with compound extensions such as
/// `page.tsx` from `"pageExtensions": ["page.tsx"]`.
fn page_stem(file: &Path, extensions: &[String]) -> Option<String> {
    let name = file.file_name()?.to_str()?;
    extensions.iter().find_map(|ext| {
        let ext = ext.trim_start_matches('.');
        name.strip_suffix(ext)?
            .strip_suffix('.')
            .filter(|stem| !stem.is_empty())
            .map(str::to_string)
    })
}

fn is_app_stem(stem: &str) -> bool {
    APP_STEMS.contains(&stem)
        || NUMBERED_STEMS.iter().any(|base| {
            stem.strip_prefix(base)
                .is_some_and(|n| n.chars().all(|c| c.is_ascii_digit()))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_stems_follow_page_extensions() {
        let exts = vec!["page.tsx".to_string(), "tsx".to_string()];
        assert_eq!(
            page_stem(Path::new("pages/a.page.tsx"), &exts).as_deref(),
            Some("a")
        );
        assert_eq!(page_stem(Path::new("pages/a.jsx"), &exts), None);
        assert!(is_app_stem("icon2"));
        assert!(!is_app_stem("iconic"));
    }
}
