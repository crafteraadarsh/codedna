//! Monorepo detection: the workspaces declared at the repository root and
//! the packages each one contains.
//!
//! Only the declarations are read (package.json `workspaces`,
//! pnpm-workspace.yaml, lerna.json, Cargo `[workspace]`, go.work and uv
//! workspaces); the packages are listed, not analysed separately.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Packages listed per workspace at most.
const MAX_PACKAGES: usize = 500;

/// How deep a `**` pattern searches.
const MAX_GLOB_DEPTH: usize = 6;

/// One workspace declaration and the packages it resolves to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    /// The tool that owns the workspace, e.g. "pnpm" or "Cargo".
    pub tool: String,
    /// The file that declares it, relative to the root.
    pub manifest: PathBuf,
    /// Packages, sorted by path.
    pub packages: Vec<WorkspacePackage>,
}

/// A package inside a workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspacePackage {
    /// The name from its manifest, or the directory name.
    pub name: String,
    /// Its directory, relative to the root (e.g. `./packages/ui`).
    pub path: PathBuf,
}

/// Find the workspaces declared at `root`.
pub fn detect(root: &Path) -> Vec<Workspace> {
    let mut found = Vec::new();

    // JavaScript: pnpm's own file wins over package.json, then lerna.json.
    let js = pnpm(root)
        .or_else(|| package_json(root))
        .or_else(|| lerna(root));
    found.extend(js);
    found.extend(cargo(root));
    found.extend(go_work(root));
    found.extend(uv(root));

    found.retain(|w| !w.packages.is_empty());
    found
}

/// Every package across `workspaces`, once.
pub fn package_count(workspaces: &[Workspace]) -> usize {
    workspaces.iter().map(|w| w.packages.len()).sum()
}

/// One line per workspace, e.g. `pnpm (pnpm-workspace.yaml): 3 packages`.
pub fn summary(w: &Workspace) -> String {
    let n = w.packages.len();
    format!(
        "{} ({}): {} package{}",
        w.tool,
        w.manifest.display(),
        n,
        if n == 1 { "" } else { "s" }
    )
}

// ---------------------------------------------------------------------------
// Declarations
// ---------------------------------------------------------------------------

fn pnpm(root: &Path) -> Option<Workspace> {
    let text = read(&root.join("pnpm-workspace.yaml"))?;
    let patterns = yaml_list(&text, "packages");
    Some(workspace(
        root,
        "pnpm",
        "pnpm-workspace.yaml",
        &patterns,
        &["package.json"],
    ))
}

fn package_json(root: &Path) -> Option<Workspace> {
    let text = read(&root.join("package.json"))?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let decl = json.get("workspaces")?;
    let list = decl
        .as_array()
        .or_else(|| decl.get("packages")?.as_array())?;
    let patterns: Vec<String> = list
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    let tool = if root.join("yarn.lock").is_file() {
        "Yarn workspaces"
    } else if root.join("bun.lock").is_file() || root.join("bun.lockb").is_file() {
        "Bun workspaces"
    } else {
        "npm workspaces"
    };
    Some(workspace(
        root,
        tool,
        "package.json",
        &patterns,
        &["package.json"],
    ))
}

fn lerna(root: &Path) -> Option<Workspace> {
    let text = read(&root.join("lerna.json"))?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let patterns: Vec<String> = match json.get("packages").and_then(|p| p.as_array()) {
        Some(list) => list
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        None => vec!["packages/*".to_string()],
    };
    Some(workspace(
        root,
        "Lerna",
        "lerna.json",
        &patterns,
        &["package.json"],
    ))
}

fn cargo(root: &Path) -> Option<Workspace> {
    let text = read(&root.join("Cargo.toml"))?;
    let doc: toml::Table = text.parse().ok()?;
    let ws = doc.get("workspace")?.as_table()?;
    let mut patterns = toml_strings(ws.get("members"));
    patterns.extend(
        toml_strings(ws.get("exclude"))
            .into_iter()
            .map(|e| format!("!{e}")),
    );
    Some(workspace(
        root,
        "Cargo",
        "Cargo.toml",
        &patterns,
        &["Cargo.toml"],
    ))
}

fn uv(root: &Path) -> Option<Workspace> {
    let text = read(&root.join("pyproject.toml"))?;
    let doc: toml::Table = text.parse().ok()?;
    let ws = doc.get("tool")?.get("uv")?.get("workspace")?.as_table()?;
    let mut patterns = toml_strings(ws.get("members"));
    patterns.extend(
        toml_strings(ws.get("exclude"))
            .into_iter()
            .map(|e| format!("!{e}")),
    );
    Some(workspace(
        root,
        "uv",
        "pyproject.toml",
        &patterns,
        &["pyproject.toml"],
    ))
}

fn go_work(root: &Path) -> Option<Workspace> {
    let text = read(&root.join("go.work"))?;
    let mut dirs = Vec::new();
    let mut in_block = false;
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        if in_block {
            if line == ")" {
                in_block = false;
            } else if !line.is_empty() {
                dirs.push(unquote(line).to_string());
            }
        } else if let Some(rest) = line.strip_prefix("use") {
            let rest = rest.trim();
            if rest == "(" {
                in_block = true;
            } else if !rest.is_empty() && rest.len() < line.len() - 2 {
                dirs.push(unquote(rest).to_string());
            }
        }
    }
    Some(workspace(root, "Go", "go.work", &dirs, &["go.mod"]))
}

// ---------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------

/// Resolve `patterns` (globs; a leading `!` excludes) to the directories
/// under `root` that hold one of `markers`.
fn workspace(
    root: &Path,
    tool: &str,
    manifest: &str,
    patterns: &[String],
    markers: &[&str],
) -> Workspace {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut excluded: Vec<PathBuf> = Vec::new();
    for pattern in patterns {
        let (exclude, pattern) = match pattern.strip_prefix('!') {
            Some(p) => (true, p),
            None => (false, pattern.as_str()),
        };
        let segments: Vec<&str> = pattern
            .trim()
            .trim_start_matches("./")
            .trim_end_matches('/')
            .split('/')
            .filter(|s| !s.is_empty() && *s != ".")
            .collect();
        if segments.contains(&"..") {
            continue;
        }
        let mut hits = Vec::new();
        expand(root, &segments, 0, &mut hits);
        if exclude {
            excluded.extend(hits);
        } else {
            dirs.extend(hits);
        }
    }
    dirs.retain(|d| !excluded.contains(d));
    dirs.sort();
    dirs.dedup();

    let packages = dirs
        .into_iter()
        .filter(|d| d.as_path() != root)
        .filter_map(|dir| {
            let marker = markers.iter().map(|m| dir.join(m)).find(|m| m.is_file())?;
            let rel = dir.strip_prefix(root).ok()?;
            let name = manifest_name(&marker).unwrap_or_else(|| {
                rel.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
            Some(WorkspacePackage {
                name,
                path: PathBuf::from(".").join(rel),
            })
        })
        .take(MAX_PACKAGES)
        .collect();

    Workspace {
        tool: tool.to_string(),
        manifest: PathBuf::from(manifest),
        packages,
    }
}

/// Collect the directories under `dir` matching `segments`.
fn expand(dir: &Path, segments: &[&str], depth: usize, out: &mut Vec<PathBuf>) {
    let Some((first, rest)) = segments.split_first() else {
        if dir.is_dir() {
            out.push(dir.to_path_buf());
        }
        return;
    };
    if *first == "**" {
        // Zero directories, or one more and stay on `**`.
        expand(dir, rest, depth, out);
        if depth < MAX_GLOB_DEPTH {
            for child in children(dir) {
                expand(&child, segments, depth + 1, out);
            }
        }
    } else if first.contains(['*', '?']) {
        for child in children(dir) {
            let name = child.file_name().unwrap_or_default().to_string_lossy();
            if wildcard(first, &name) {
                expand(&child, rest, depth + 1, out);
            }
        }
    } else {
        expand(&dir.join(first), rest, depth + 1, out);
    }
}

/// Subdirectories, without hidden, dependency and build directories.
fn children(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            !name.starts_with('.') && !matches!(name.as_ref(), "node_modules" | "target" | "vendor")
        })
        .map(|e| e.path())
        .collect();
    out.sort();
    out
}

/// Whether `name` matches `pattern`, where `*` is any run and `?` one char.
fn wildcard(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ni));
            pi += 1;
        } else if let Some((sp, sn)) = star {
            pi = sp + 1;
            ni = sn + 1;
            star = Some((sp, sn + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '*')
}

/// The package name declared in a manifest, if any.
fn manifest_name(manifest: &Path) -> Option<String> {
    let text = read(manifest)?;
    let file = manifest.file_name()?.to_str()?;
    let name = match file {
        "package.json" => serde_json::from_str::<serde_json::Value>(&text)
            .ok()?
            .get("name")?
            .as_str()?
            .to_string(),
        "Cargo.toml" | "pyproject.toml" => {
            let doc: toml::Table = text.parse().ok()?;
            let section = if file == "Cargo.toml" {
                "package"
            } else {
                "project"
            };
            doc.get(section)?.get("name")?.as_str()?.to_string()
        }
        "go.mod" => text
            .lines()
            .find_map(|l| l.trim().strip_prefix("module "))?
            .trim()
            .to_string(),
        _ => return None,
    };
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_string())
}

// ---------------------------------------------------------------------------
// Small parsers
// ---------------------------------------------------------------------------

/// Manifests above this size are not read.
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

fn read(path: &Path) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_MANIFEST_BYTES {
        return None;
    }
    fs::read_to_string(path).ok()
}

fn toml_strings(value: Option<&toml::Value>) -> Vec<String> {
    value
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// The items of a top-level YAML list `key:` (block or `[a, b]` flow style).
fn yaml_list(text: &str, key: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_list = false;
    for raw in text.lines() {
        let line = raw.split(" #").next().unwrap_or("");
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let indented = line.starts_with([' ', '\t', '-']);
        if !indented {
            in_list = false;
            if let Some(rest) = line.strip_prefix(key).and_then(|r| r.strip_prefix(':')) {
                let rest = rest.trim();
                if let Some(flow) = rest.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
                    out.extend(
                        flow.split(',')
                            .map(|s| unquote(s.trim()).to_string())
                            .filter(|s| !s.is_empty()),
                    );
                } else {
                    in_list = rest.is_empty();
                }
            }
        } else if in_list {
            if let Some(item) = line.trim().strip_prefix('-') {
                let item = unquote(item.trim());
                if !item.is_empty() {
                    out.push(item.to_string());
                }
            }
        }
    }
    out
}

fn unquote(s: &str) -> &str {
    s.trim_matches(|c| c == '"' || c == '\'')
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn paths(w: &Workspace) -> Vec<String> {
        w.packages
            .iter()
            .map(|p| p.path.to_string_lossy().replace('\\', "/"))
            .collect()
    }

    #[test]
    fn single_package_repo_has_no_workspace() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "package.json", r#"{"name":"app"}"#);
        write(dir.path(), "Cargo.toml", "[package]\nname = \"x\"\n");
        assert!(detect(dir.path()).is_empty());
    }

    #[test]
    fn npm_workspaces_with_globs_and_names() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "package.json",
            r#"{"workspaces":["packages/*","apps/web"]}"#,
        );
        write(root, "packages/ui/package.json", r#"{"name":"@acme/ui"}"#);
        write(root, "packages/utils/package.json", "{}");
        write(root, "packages/notes/README.md", "no manifest");
        write(root, "apps/web/package.json", r#"{"name":"web"}"#);
        write(root, "apps/docs/package.json", r#"{"name":"docs"}"#);

        let ws = detect(root);
        assert_eq!(ws.len(), 1);
        assert_eq!(ws[0].tool, "npm workspaces");
        assert_eq!(
            paths(&ws[0]),
            ["./apps/web", "./packages/ui", "./packages/utils"]
        );
        let names: Vec<&str> = ws[0].packages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["web", "@acme/ui", "utils"]);
    }

    #[test]
    fn yarn_object_form_and_negation() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "package.json",
            r#"{"workspaces":{"packages":["packages/*","!packages/legacy"]}}"#,
        );
        write(root, "yarn.lock", "");
        write(root, "packages/a/package.json", "{}");
        write(root, "packages/legacy/package.json", "{}");

        let ws = detect(root);
        assert_eq!(ws[0].tool, "Yarn workspaces");
        assert_eq!(paths(&ws[0]), ["./packages/a"]);
    }

    #[test]
    fn pnpm_yaml_wins_and_double_star_skips_node_modules() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "pnpm-workspace.yaml",
            "# workspace\npackages:\n  - 'libs/**'\n  - \"apps/*\" # apps\ncatalog:\n  - nope/*\n",
        );
        write(root, "package.json", r#"{"workspaces":["ignored/*"]}"#);
        write(root, "libs/a/package.json", "{}");
        write(root, "libs/group/b/package.json", "{}");
        write(root, "libs/a/node_modules/dep/package.json", "{}");
        write(root, "apps/site/package.json", "{}");
        write(root, "nope/x/package.json", "{}");

        let ws = detect(root);
        assert_eq!(ws.len(), 1);
        assert_eq!(ws[0].tool, "pnpm");
        assert_eq!(paths(&ws[0]), ["./apps/site", "./libs/a", "./libs/group/b"]);
    }

    #[test]
    fn cargo_members_and_exclude() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "Cargo.toml",
            "[workspace]\nmembers = [\"crates/*\", \"cli\"]\nexclude = [\"crates/old\"]\n",
        );
        write(
            root,
            "crates/core/Cargo.toml",
            "[package]\nname = \"acme-core\"\n",
        );
        write(root, "crates/old/Cargo.toml", "[package]\nname = \"old\"\n");
        write(root, "cli/Cargo.toml", "[package]\nname = \"acme\"\n");

        let ws = detect(root);
        assert_eq!(ws[0].tool, "Cargo");
        assert_eq!(paths(&ws[0]), ["./cli", "./crates/core"]);
        assert_eq!(ws[0].packages[1].name, "acme-core");
    }

    #[test]
    fn go_work_block_and_single_line() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "go.work",
            "go 1.22\n\nuse (\n\t./api // service\n\t./tools\n)\nuse ./cmd\n",
        );
        write(root, "api/go.mod", "module example.com/api\n");
        write(root, "tools/go.mod", "module example.com/tools\n");
        write(root, "cmd/go.mod", "module example.com/cmd\n");

        let ws = detect(root);
        assert_eq!(ws[0].tool, "Go");
        assert_eq!(paths(&ws[0]), ["./api", "./cmd", "./tools"]);
        assert_eq!(ws[0].packages[0].name, "example.com/api");
    }

    #[test]
    fn uv_workspace_and_mixed_repo() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "pyproject.toml",
            "[project]\nname = \"root\"\n[tool.uv.workspace]\nmembers = [\"packages/*\"]\n",
        );
        write(
            root,
            "packages/lib/pyproject.toml",
            "[project]\nname = \"lib\"\n",
        );
        write(root, "lerna.json", "{}");
        write(root, "packages/js/package.json", r#"{"name":"js"}"#);

        let ws = detect(root);
        let tools: Vec<&str> = ws.iter().map(|w| w.tool.as_str()).collect();
        assert_eq!(tools, ["Lerna", "uv"]);
        assert_eq!(ws[1].packages[0].name, "lib");
        assert_eq!(package_count(&ws), 2);
        assert_eq!(summary(&ws[1]), "uv (pyproject.toml): 1 package");
    }

    #[test]
    fn ignores_parent_paths() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        write(&root, "package.json", r#"{"workspaces":["../*", "*"]}"#);
        write(dir.path(), "outside/package.json", "{}");
        write(&root, "pkg/package.json", "{}");
        let ws = detect(&root);
        assert_eq!(paths(&ws[0]), ["./pkg"]);
    }

    #[test]
    fn wildcard_matching() {
        assert!(wildcard("*", "abc"));
        assert!(wildcard("plugin-*", "plugin-x"));
        assert!(!wildcard("plugin-*", "core"));
        assert!(wildcard("a?c", "abc"));
        assert!(wildcard("*-*-z", "a-b-c-z"));
    }

    #[test]
    fn yaml_flow_list() {
        assert_eq!(
            yaml_list("packages: ['a/*', b]\n", "packages"),
            ["a/*", "b"]
        );
    }
}
