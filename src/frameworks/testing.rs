//! Test runners: the test files, setup files and module mocks that Vitest,
//! Jest, Playwright, Mocha and `tsd` load by configuration or convention.
//!
//! A runner is active in a package whose own `package.json` depends on it
//! or that has its own config file. Globs and setup files come from literal
//! config values (`include`, `testMatch`, `testDir`, `spec`, `setupFiles`,
//! ...), falling back to the runner's defaults. Every fact carries the
//! condition `test`.

use super::{config_strings, is_js, Adapters, Glob, Package, Repo};
use crate::facts::Certainty;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub const VERSION: u32 = 1;
const SETUP_RULE: &str = "test.setup@v1";
const MOCK_RULE: &str = "test.module_mock@v1";

const JS: &str = "{js,jsx,ts,tsx,mjs,cjs,mts,cts}";

struct Runner {
    adapter: &'static str,
    rule: &'static str,
    dependencies: &'static [&'static str],
    configs: &'static [&'static str],
    /// Keys whose literal values replace the default test globs.
    glob_keys: &'static [&'static str],
    /// Keys naming a directory the globs are relative to.
    dir_keys: &'static [&'static str],
    /// Keys whose presence means globs cannot be read (`testRegex`).
    opaque_keys: &'static [&'static str],
    defaults: fn() -> Vec<String>,
    setup_keys: &'static [&'static str],
    /// The `package.json` key that may hold the config.
    manifest_key: Option<&'static str>,
}

const RUNNERS: &[Runner] = &[
    Runner {
        adapter: "vitest",
        rule: "test.vitest@v1",
        dependencies: &["vitest"],
        configs: &[
            "vitest.config.ts",
            "vitest.config.mts",
            "vitest.config.cts",
            "vitest.config.js",
            "vitest.config.mjs",
            "vitest.config.cjs",
            "vitest.workspace.ts",
            "vitest.workspace.js",
        ],
        glob_keys: &["include"],
        dir_keys: &[],
        opaque_keys: &[],
        defaults: || {
            vec![
                format!("**/*.{{test,spec}}.{JS}"),
                format!("**/*.{{test,spec}}-d.{JS}"),
            ]
        },
        setup_keys: &["setupFiles", "globalSetup"],
        manifest_key: None,
    },
    Runner {
        adapter: "jest",
        rule: "test.jest@v1",
        dependencies: &["jest"],
        configs: &[
            "jest.config.js",
            "jest.config.ts",
            "jest.config.mjs",
            "jest.config.cjs",
            "jest.config.json",
        ],
        glob_keys: &["testMatch"],
        dir_keys: &[],
        opaque_keys: &["testRegex"],
        defaults: || {
            vec![
                format!("**/__tests__/**/*.{JS}"),
                format!("**/*.{{test,spec}}.{JS}"),
            ]
        },
        setup_keys: &[
            "setupFiles",
            "setupFilesAfterEnv",
            "globalSetup",
            "globalTeardown",
        ],
        manifest_key: Some("jest"),
    },
    Runner {
        adapter: "playwright",
        rule: "test.playwright@v1",
        dependencies: &["@playwright/test", "playwright"],
        configs: &[
            "playwright.config.ts",
            "playwright.config.js",
            "playwright.config.mts",
            "playwright.config.mjs",
            "playwright.config.cjs",
        ],
        glob_keys: &["testMatch"],
        dir_keys: &["testDir"],
        opaque_keys: &[],
        defaults: || vec![format!("**/*.{{spec,test}}.{JS}")],
        setup_keys: &["globalSetup", "globalTeardown"],
        manifest_key: None,
    },
    Runner {
        adapter: "mocha",
        rule: "test.mocha@v1",
        dependencies: &["mocha"],
        configs: &[
            ".mocharc.js",
            ".mocharc.cjs",
            ".mocharc.json",
            ".mocharc.jsonc",
            ".mocharc.yml",
            ".mocharc.yaml",
        ],
        glob_keys: &["spec"],
        dir_keys: &[],
        opaque_keys: &[],
        defaults: || vec!["test/*.{js,cjs,mjs}".to_string()],
        setup_keys: &["require", "file"],
        manifest_key: Some("mocha"),
    },
    Runner {
        adapter: "tsd",
        rule: "test.tsd@v1",
        dependencies: &["tsd"],
        configs: &[],
        glob_keys: &[],
        dir_keys: &[],
        opaque_keys: &[],
        defaults: || vec!["index.test-d.ts".to_string(), "test-d/**/*.ts".to_string()],
        setup_keys: &[],
        manifest_key: None,
    },
];

pub fn detect(repo: &Repo, package: &Package, out: &mut Adapters) {
    for runner in RUNNERS {
        detect_runner(repo, package, runner, out);
    }
    module_mocks(package, out);
}

fn detect_runner(repo: &Repo, package: &Package, runner: &Runner, out: &mut Adapters) {
    let dependency = package.depends_on_any(runner.dependencies);
    let mut configs: Vec<PathBuf> = runner
        .configs
        .iter()
        .filter_map(|name| package.file_in(&package.dir, name).cloned())
        .collect();
    if runner.adapter == "vitest" && dependency.is_some() && configs.is_empty() {
        // Vitest reads the `test` block of the Vite config.
        configs.extend(
            [
                "vite.config.ts",
                "vite.config.mts",
                "vite.config.js",
                "vite.config.mjs",
            ]
            .iter()
            .filter_map(|name| package.file_in(&package.dir, name).cloned()),
        );
    }
    let manifest_config = runner
        .manifest_key
        .and_then(|key| package.json.get(key))
        .filter(|v| v.is_object())
        .map(|v| v.to_string());
    if dependency.is_none() && configs.is_empty() && manifest_config.is_none() {
        return;
    }
    let line = dependency.and_then(|(_, line)| line);
    let texts: Vec<String> = configs
        .iter()
        .filter_map(|c| fs::read_to_string(c).ok())
        .chain(manifest_config)
        // Coverage settings have their own `include`.
        .map(|text| without_block(&text, "coverage"))
        .collect();
    let read = |key: &str| -> Option<Vec<String>> {
        texts
            .iter()
            .filter_map(|text| config_strings(text, key))
            .reduce(|mut a, b| {
                a.extend(b);
                a
            })
    };

    let mut seen = HashSet::new();
    let mut emit = |out: &mut Adapters, file: &Path, rule: &str| {
        if seen.insert(file.to_path_buf()) {
            out.entries.push(
                package
                    .entry(file, rule, Certainty::Inferred, line)
                    .condition("test"),
            );
        }
    };

    for key in runner.setup_keys {
        for path in read(key).unwrap_or_default() {
            let path = path.replace("<rootDir>/", "").replace("<rootDir>", ".");
            if let Some(file) = repo.resolve(&package.dir, &path) {
                emit(out, &file, SETUP_RULE);
            }
        }
    }

    let opaque = runner.opaque_keys.iter().any(|key| read(key).is_some());
    let mut globs: Vec<String> = match runner.glob_keys.iter().find_map(|key| read(key)) {
        Some(globs) => globs,
        None if opaque => Vec::new(),
        None => (runner.defaults)(),
    };
    if runner.adapter == "vitest" {
        // `vitest bench` keeps its own default, `benchmark.include`.
        globs.push(format!("**/*.{{bench,benchmark}}.{JS}"));
    }
    let globs: Vec<Glob> = globs
        .iter()
        .filter_map(|g| Glob::new(&g.replace("<rootDir>/", "")))
        .collect();
    let base = runner
        .dir_keys
        .iter()
        .find_map(|key| read(key).and_then(|v| v.into_iter().next()))
        .map(|dir| package.dir.join(dir.trim_start_matches("./")))
        .unwrap_or_else(|| package.dir.clone());
    let base = crate::dependency_graph::normalize(&base);
    for file in package.files.iter().filter(|f| is_js(f)) {
        let Ok(rel) = crate::dependency_graph::normalize(file)
            .strip_prefix(&base)
            .map(|r| super::relative(Path::new(""), r))
        else {
            continue;
        };
        if globs.iter().any(|g| g.matches(&rel)) {
            emit(out, file, runner.rule);
        }
    }

    out.active
        .push(package.adapter(runner.adapter, VERSION, line, configs));
}

/// Manual mocks of a dependency, next to the `package.json` declaring it:
/// `__mocks__/axios.ts` or `__mocks__/zustand/index.ts`. Jest and Vitest
/// swap them in for the dependency.
fn module_mocks(package: &Package, out: &mut Adapters) {
    let mocks = package.dir.join("__mocks__");
    let sections = [
        "dependencies",
        "devDependencies",
        "peerDependencies",
        "optionalDependencies",
    ];
    let deps: HashSet<&str> = sections
        .iter()
        .filter_map(|s| package.json.get(s)?.as_object())
        .flat_map(|m| m.keys().map(String::as_str))
        .collect();
    if deps.is_empty() {
        return;
    }
    for file in package
        .files
        .iter()
        .filter(|f| f.starts_with(&mocks) && is_js(f))
    {
        let rel = super::relative(&mocks, file);
        let module = match rel.rsplit_once('/') {
            // `__mocks__/zustand/index.ts`, `__mocks__/@scope/pkg.ts`.
            Some((dir, name)) => {
                let stem = name.split('.').next().unwrap_or(name);
                if deps.contains(dir) {
                    dir.to_string()
                } else {
                    format!("{dir}/{stem}")
                }
            }
            None => rel.split('.').next().unwrap_or(&rel).to_string(),
        };
        let declared = deps.contains(module.as_str())
            || deps.iter().any(|d| module.starts_with(&format!("{d}/")));
        if declared {
            let line = package.depends_on(&module).flatten();
            out.entries.push(
                package
                    .entry(file, MOCK_RULE, Certainty::Inferred, line)
                    .condition("test"),
            );
        }
    }
}

/// `text` without the object literal given to `key` (`coverage: { ... }`).
fn without_block(text: &str, key: &str) -> String {
    let mut out = text.to_string();
    let mut from = 0;
    while let Some(pos) = out[from..].find(key) {
        let start = from + pos;
        from = start + key.len();
        let rest = out[from..].trim_start_matches(['"', '\'']).trim_start();
        let Some(value) = rest.strip_prefix(':').map(str::trim_start) else {
            continue;
        };
        if !value.starts_with('{') {
            continue;
        }
        let open = out.len() - value.len();
        let mut depth = 0;
        let mut end = None;
        for (i, c) in out[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(open + i + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else { break };
        out.replace_range(open..end, "{}");
        from = open + 2;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_include_is_not_a_test_glob() {
        let text = "test: { coverage: { include: ['src/**'] }, include: ['t/**'] }";
        let cleaned = without_block(text, "coverage");
        assert_eq!(config_strings(&cleaned, "include").unwrap(), ["t/**"]);
    }
}
