//! JavaScript and TypeScript module resolution.
//!
//! Follows the rules of Node.js ("Resolution algorithm specification" in the
//! ECMAScript modules documentation: `PACKAGE_RESOLVE`,
//! `PACKAGE_EXPORTS_RESOLVE`, `PACKAGE_IMPORTS_RESOLVE`,
//! `PACKAGE_TARGET_RESOLVE`) and of TypeScript's module resolution
//! (`paths`, `baseUrl`, extension substitution, `customConditions`), checked
//! against `tsc --traceResolution` (TypeScript 5.9) and Node.js 22 on the
//! fixtures in `tests/cli_js_resolution.rs`. It is not a complete
//! reimplementation of either; the differences are deliberate:
//!
//! - Resolution is fenced to the scanned files. There is no `node_modules`
//!   lookup: a bare specifier that names none of the repository's own
//!   packages is `external`.
//! - A package condition CodeDna cannot know (`browser`, `node`,
//!   `development`, `types` for code, ...) is not decided: every branch is
//!   kept, labelled with its condition, so a file used only by the browser
//!   build stays reachable. The conditions every consumer agrees on
//!   (`default`, `import` or `require` from the import's own syntax, the
//!   project's `customConditions`) are decided as Node does.
//! - Built output (`dist/x.js`) missing from the repository is mapped back
//!   to its source. With a `tsconfig.json` `outDir` / `rootDir` saying so,
//!   that is `direct`; guessed from folder names, it is `inferred`.
//! - Two workspace packages with the same name make an import of that name
//!   `ambiguous` instead of picking one. Packages declared by a different
//!   workspace (a folder of example monorepos, each with its own
//!   `pnpm-workspace.yaml`) do not compete: the importer's own, innermost
//!   workspace decides.

use super::{Json, Outcome, Target, MAX_CONFIG_BYTES};
use crate::dependency_graph::normalize;
use crate::facts::{Certainty, DependencyFact, DependencyKind, Resolution};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Whether a file runs as an ES module or as CommonJS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Esm,
    Cjs,
}

/// How an import is written, which picks the `import` or `require`
/// package condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Syntax {
    /// `import ... from`, `export ... from`, `import "x"`.
    Import,
    /// `import("x")`, always ESM.
    DynamicImport,
    /// `require("x")`, `import x = require("x")`.
    Require,
}

/// TypeScript's `moduleResolution`, reduced to what changes the result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Mode {
    /// `node10`, `classic` or unset: permissive, like a bundler.
    #[default]
    Loose,
    /// `node16` / `nodenext`: ES modules need full relative paths.
    Node16,
    /// `bundler`.
    Bundler,
}

/// The `${configDir}` template in config paths.
const CONFIG_DIR: &str = "${configDir}";

/// `paths` patterns and their targets, in the order written.
type PathPatterns = Vec<(String, Vec<String>)>;

/// The parts of a `tsconfig.json` / `jsconfig.json` resolution uses, after
/// `extends`.
#[derive(Debug, Clone, Default)]
struct TsConfig {
    /// Directory holding the config.
    dir: PathBuf,
    base_url: Option<PathBuf>,
    /// `paths` and the directory of the config that set them, which their
    /// targets are relative to when there is no `baseUrl`.
    paths: Option<(PathPatterns, PathBuf)>,
    module_resolution: Option<String>,
    module: Option<String>,
    custom_conditions: Option<Vec<String>>,
    out_dir: Option<PathBuf>,
    root_dir: Option<PathBuf>,
    /// Folders `include` names, cut at the first wildcard.
    include: Option<Vec<PathBuf>>,
    /// `"files": []`.
    no_files: Option<bool>,
    /// Configs named by `references` (not inherited).
    references: Vec<PathBuf>,
}

impl TsConfig {
    /// Fill every option this config does not set from `base`.
    fn inherit(&mut self, base: &TsConfig) {
        macro_rules! fill {
            ($($field:ident),*) => {$(
                if self.$field.is_none() {
                    self.$field = base.$field.clone();
                }
            )*};
        }
        fill!(
            base_url,
            paths,
            module_resolution,
            module,
            custom_conditions,
            out_dir,
            root_dir,
            include,
            no_files
        );
    }

    /// Replace `${configDir}` (TypeScript 5.5) with the directory of
    /// this config, the one in use, rather than the base that wrote it.
    fn expand_config_dir(&mut self) {
        let dir = self.dir.clone();
        let expand = |path: &mut PathBuf| {
            if let Ok(rest) = path.strip_prefix(CONFIG_DIR) {
                *path = normalize(&dir.join(rest));
            }
        };
        for path in [&mut self.base_url, &mut self.out_dir, &mut self.root_dir]
            .into_iter()
            .flatten()
        {
            expand(path);
        }
        for path in self.include.iter_mut().flatten() {
            expand(path);
        }
        let dir_text = dir.to_string_lossy().into_owned();
        for (_, targets) in self.paths.iter_mut().flat_map(|(p, _)| p.iter_mut()) {
            for target in targets {
                if let Some(rest) = target.strip_prefix(CONFIG_DIR) {
                    *target = format!("{dir_text}{rest}");
                }
            }
        }
    }

    fn mode(&self) -> Mode {
        let pick = |value: &Option<String>| match value.as_deref() {
            Some("node16" | "nodenext") => Some(Mode::Node16),
            Some("bundler") => Some(Mode::Bundler),
            Some(_) => Some(Mode::Loose),
            None => None,
        };
        pick(&self.module_resolution)
            .or(match self.module.as_deref() {
                Some("node16" | "node18" | "node20" | "nodenext") => Some(Mode::Node16),
                Some("preserve") => Some(Mode::Bundler),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// A solution config: `"files": []` and no `include`, only
    /// `references`.
    fn is_solution(&self) -> bool {
        self.no_files == Some(true) && self.include.is_none()
    }

    fn includes(&self, file: &Path) -> bool {
        if self.is_solution() {
            return false;
        }
        match &self.include {
            Some(prefixes) => prefixes.iter().any(|p| file.starts_with(p)),
            None => file.starts_with(&self.dir),
        }
    }

    /// Where `outDir` output comes from: `rootDir`, else the one folder
    /// `include` names.
    fn source_root(&self) -> Option<PathBuf> {
        if let Some(root) = &self.root_dir {
            return Some(root.clone());
        }
        match self.include.as_deref() {
            Some([one]) => Some(one.clone()),
            _ => None,
        }
    }
}

/// One `package.json`.
#[derive(Debug)]
struct Package {
    dir: PathBuf,
    format: Format,
    exports: Option<Json>,
    imports: Option<Json>,
    /// `main`, `module`, `browser` (as a string), `types`, `typings`,
    /// `source`.
    fields: Vec<(&'static str, String)>,
}

impl Package {
    fn field(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(f, _)| *f == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Fields of a package without `exports` that point at an entry file,
/// with the condition each applies under.
const ENTRY_FIELDS: &[(&str, &str)] = &[
    ("module", "module"),
    ("browser", "browser"),
    ("types", "types"),
    ("typings", "types"),
    ("source", "source"),
];

/// Folders built output usually goes to.
const BUILD_DIRS: &[&str] = &["dist", "build", "lib", "out", "esm", "cjs"];

/// Extensions appended to a path written without one, in order.
const TS_APPEND: &[&str] = &["ts", "tsx", "d.ts", "js", "jsx", "mts", "cts", "mjs", "cjs"];
const JS_APPEND: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "mts", "cts", "cjs", "d.ts"];
const TS_INDEX: &[&str] = &[
    "index.ts",
    "index.tsx",
    "index.d.ts",
    "index.js",
    "index.jsx",
    "index.mjs",
];
const JS_INDEX: &[&str] = &[
    "index.ts",
    "index.tsx",
    "index.js",
    "index.jsx",
    "index.mjs",
    "index.d.ts",
];

/// How a path was turned into a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum How {
    /// The path as written.
    Exact,
    /// A JavaScript extension swapped for its TypeScript source
    /// (`./x.js` → `x.ts`), as TypeScript does.
    Substituted,
    /// An extension added.
    Appended,
    /// A folder's `index` file.
    Index,
}

/// Resolves JavaScript and TypeScript specifiers against the scanned files.
#[derive(Debug, Default)]
pub struct Resolver {
    known: HashSet<PathBuf>,
    /// Unit tests that pass no file list resolve against the disk.
    disk: bool,
    packages: HashMap<PathBuf, Package>,
    by_name: HashMap<String, Vec<PathBuf>>,
    /// Directories of packages a workspace declares (`workspaces`,
    /// `pnpm-workspace.yaml`).
    /// Each with the workspace roots that declare it.
    members: HashMap<PathBuf, Vec<PathBuf>>,
    /// Effective configs, by config file.
    configs: HashMap<PathBuf, TsConfig>,
    /// Directory → its `tsconfig.json` (else `jsconfig.json`).
    projects: HashMap<PathBuf, PathBuf>,
    /// Directories holding a `svelte.config.*`.
    svelte: HashSet<PathBuf>,
    /// Directories holding a `docusaurus.config.*`.
    docusaurus: HashSet<PathBuf>,
    /// Directories holding a `vite.config.*`.
    vite: HashSet<PathBuf>,
    /// The known files in path order, for glob expansion.
    sorted: Vec<PathBuf>,
}

/// What an `import.meta.glob` expands to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GlobMatch {
    /// The matching files, sorted.
    Files(Vec<PathBuf>),
    /// No file matches, or the patterns are aliases.
    None,
    /// More files than [`MAX_GLOB_FILES`], or a pattern this matcher does
    /// not read (extglobs).
    Unsupported,
}

/// The most files one `import.meta.glob` may expand to.
pub const MAX_GLOB_FILES: usize = 10_000;

/// `path` under `base`, with forward slashes.
fn relative_path(base: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(base).ok()?;
    let parts: Vec<&str> = rel
        .components()
        .map(|c| c.as_os_str().to_str())
        .collect::<Option<_>>()?;
    Some(parts.join("/"))
}

/// Config files that make a directory a Vite root.
const VITE_CONFIG_EXTENSIONS: &[&str] = &["js", "mjs", "cjs", "ts", "mts", "cts"];

/// What resolution knows about the file doing the importing.
#[derive(Debug, Clone, Copy)]
pub struct Context<'r> {
    dir: &'r Path,
    package: Option<&'r Package>,
    config: Option<&'r TsConfig>,
    format: Format,
    /// A TypeScript file, which prefers `x.ts` over `x.js` for `./x.js`.
    ts: bool,
}

/// One import to resolve.
#[derive(Debug, Clone, Copy)]
pub struct Request<'s> {
    pub specifier: &'s str,
    pub syntax: Syntax,
    /// `import type`, `export type`, JSDoc: only types are needed.
    pub types_only: bool,
}

/// Package conditions in force for one request.
struct Conditions<'a> {
    /// `import` or `require`.
    mode: &'static str,
    types_only: bool,
    custom: &'a [String],
}

enum Decision {
    Yes,
    No,
    /// Depends on the consumer: keep the branch, labelled.
    Maybe,
}

impl Conditions<'_> {
    fn decide(&self, key: &str) -> Decision {
        match key {
            "default" => Decision::Yes,
            "import" | "require" if key == self.mode => Decision::Yes,
            "import" | "require" => Decision::No,
            "types" if self.types_only => Decision::Yes,
            _ if self.custom.iter().any(|c| c == key) => Decision::Yes,
            _ => Decision::Maybe,
        }
    }
}

impl Resolver {
    /// Index the packages and configs among `files`.
    pub fn from_files(files: &[PathBuf]) -> Self {
        let known: HashSet<PathBuf> = files.iter().map(|f| normalize(f)).collect();
        let mut resolver = Resolver {
            disk: cfg!(test) && known.is_empty(),
            known,
            ..Resolver::default()
        };
        let mut config_files = Vec::new();
        let mut workspace_roots = Vec::new();
        let mut paths: Vec<&PathBuf> = resolver.known.iter().collect();
        paths.sort();
        let paths: Vec<PathBuf> = paths.into_iter().cloned().collect();
        for path in &paths {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let Some(dir) = path.parent() else { continue };
            match name {
                "package.json" => {
                    if let Some(json) = read_json(path, false) {
                        if json.get("workspaces").is_some() {
                            workspace_roots.push((dir.to_path_buf(), json.clone()));
                        }
                        resolver.add_package(dir, &json);
                    }
                }
                "pnpm-workspace.yaml" => {
                    if let Some(text) = read_text(path) {
                        let globs = crate::workspaces::yaml_list(&text, "packages");
                        resolver.add_members(dir, &globs);
                    }
                }
                _ => {}
            }
            let stem = name.strip_suffix(".json");
            if stem.is_some_and(|s| s.starts_with("tsconfig") || s.starts_with("jsconfig")) {
                config_files.push(path.clone());
            }
            if let Some((stem, ext)) = name.rsplit_once('.') {
                if matches!(ext, "js" | "mjs" | "cjs" | "ts" | "mts" | "cts") {
                    match stem {
                        "svelte.config" => {
                            resolver.svelte.insert(dir.to_path_buf());
                        }
                        "docusaurus.config" => {
                            resolver.docusaurus.insert(dir.to_path_buf());
                        }
                        "vite.config" if VITE_CONFIG_EXTENSIONS.contains(&ext) => {
                            resolver.vite.insert(dir.to_path_buf());
                        }
                        _ => {}
                    }
                }
            }
        }
        for (dir, json) in workspace_roots {
            let globs = match json.get("workspaces") {
                Some(Json::Object(_)) => json
                    .get("workspaces")
                    .and_then(|w| w.get("packages"))
                    .map(|p| p.strings().into_iter().map(str::to_string).collect())
                    .unwrap_or_default(),
                Some(list) => list.strings().into_iter().map(str::to_string).collect(),
                None => Vec::new(),
            };
            resolver.add_members(&dir, &globs);
        }

        let mut loader = ConfigLoader::default();
        for path in &config_files {
            let config = loader.load(&resolver, path);
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if let Some(dir) = path.parent() {
                if name == "tsconfig.json"
                    || (name == "jsconfig.json" && !resolver.projects.contains_key(dir))
                {
                    resolver.projects.insert(dir.to_path_buf(), path.clone());
                }
            }
            for reference in config.references.clone() {
                let referenced = loader.load(&resolver, &reference);
                resolver.configs.entry(reference).or_insert(referenced);
            }
            resolver.configs.insert(path.clone(), config);
        }
        resolver.sorted = paths;
        resolver
    }

    fn add_package(&mut self, dir: &Path, json: &Json) {
        let name = json
            .get("name")
            .and_then(Json::as_str)
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_string);
        let format = match json.get("type").and_then(Json::as_str) {
            Some("module") => Format::Esm,
            _ => Format::Cjs,
        };
        let fields = ["main", "module", "browser", "types", "typings", "source"]
            .into_iter()
            .filter_map(|f| Some((f, json.get(f)?.as_str()?.to_string())))
            .collect();
        if let Some(name) = &name {
            self.by_name
                .entry(name.clone())
                .or_default()
                .push(dir.to_path_buf());
        }
        self.packages.insert(
            dir.to_path_buf(),
            Package {
                dir: dir.to_path_buf(),
                format,
                exports: json.get("exports").cloned(),
                imports: json.get("imports").cloned(),
                fields,
            },
        );
    }

    /// Mark the package folders below `root` that `globs` name as workspace
    /// members. Runs after every `package.json` is indexed only for
    /// `workspaces`; `pnpm-workspace.yaml` sorts after `package.json` in
    /// the same folder but may come before deeper packages, so membership
    /// is matched on paths, not on indexed packages.
    fn add_members(&mut self, root: &Path, globs: &[String]) {
        let globs: Vec<Vec<&str>> = globs
            .iter()
            .filter(|g| !g.starts_with('!'))
            .map(|g| {
                g.trim_start_matches("./")
                    .trim_end_matches('/')
                    .split('/')
                    .collect()
            })
            .collect();
        for path in &self.known {
            if path.file_name().is_none_or(|n| n != "package.json") {
                continue;
            }
            let Some(dir) = path.parent() else { continue };
            let Ok(rel) = dir.strip_prefix(root) else {
                continue;
            };
            let segments: Vec<String> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            if segments.iter().any(|s| s == "node_modules") {
                continue;
            }
            let segments: Vec<&str> = segments.iter().map(String::as_str).collect();
            if globs.iter().any(|g| glob_segments(g, &segments)) {
                let roots = self.members.entry(dir.to_path_buf()).or_default();
                if !roots.iter().any(|r| r == root) {
                    roots.push(root.to_path_buf());
                }
            }
        }
    }

    fn exists(&self, path: &Path) -> bool {
        self.known.contains(path) || (self.disk && fs::metadata(path).is_ok_and(|m| m.is_file()))
    }

    /// What resolution needs to know about `file`, the importer.
    pub fn context<'r>(&'r self, file: &'r Path) -> Context<'r> {
        let dir = file.parent().unwrap_or(Path::new(""));
        let package = dir.ancestors().find_map(|d| self.packages.get(d));
        let config = self.config_for(file);
        let ext = file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let format = match ext.as_str() {
            "mjs" | "mts" | "vue" | "svelte" | "astro" => Format::Esm,
            "cjs" | "cts" => Format::Cjs,
            _ => package.map_or(Format::Cjs, |p| p.format),
        };
        Context {
            dir,
            package,
            config,
            format,
            ts: matches!(ext.as_str(), "ts" | "tsx" | "mts" | "cts"),
        }
    }

    /// The config TypeScript uses for `file`: the nearest `tsconfig.json`
    /// (else `jsconfig.json`) above it, or, for a solution config, the
    /// referenced config that includes the file.
    fn config_for(&self, file: &Path) -> Option<&TsConfig> {
        let path = file
            .ancestors()
            .skip(1)
            .find_map(|d| self.projects.get(d))?;
        let config = self.configs.get(path)?;
        if config.includes(file) || config.references.is_empty() {
            return Some(config);
        }
        config
            .references
            .iter()
            .filter_map(|r| self.configs.get(r))
            .find(|r| r.includes(file))
            .or(Some(config))
    }

    /// Resolve `request` made by the file `ctx` describes.
    pub fn resolve(&self, ctx: &Context, request: Request) -> Outcome {
        let specifier = request.specifier;
        if specifier.starts_with('.') {
            return self.relative(ctx, request);
        }
        // `node:fs`, `virtual:x`, `https://...`: never a repository file.
        if specifier.split('/').next().is_some_and(|s| s.contains(':')) {
            return Outcome::external();
        }
        self.bare(ctx, request)
    }

    /// The files Vite's `import.meta.glob(patterns)` in `file` loads.
    /// `./` and `../` patterns are relative to the file, `/` patterns to
    /// the Vite root (the nearest folder with a `vite.config.*`, within the
    /// package, else the package folder). Patterns starting `!` remove
    /// files. Alias patterns (`@/x/*`) match nothing here. Like Vite,
    /// `node_modules` and dot folders are skipped unless named, and the
    /// file never matches itself.
    pub fn glob(&self, ctx: &Context, file: &Path, patterns: &[String]) -> GlobMatch {
        let file = normalize(file);
        let mut include = Vec::new();
        let mut exclude = Vec::new();
        for pattern in patterns {
            let (negated, body) = match pattern.strip_prefix('!') {
                Some(body) => (true, body),
                None => (false, pattern.as_str()),
            };
            let anchor = if body.starts_with("./") || body.starts_with("../") {
                ctx.dir.to_path_buf()
            } else if body.starts_with('/') {
                self.vite_root(ctx)
            } else if negated {
                // An aliased negation cannot be applied; matching more is
                // the safe side.
                continue;
            } else {
                // Aliases and package paths are not followed.
                return GlobMatch::Unsupported;
            };
            let body = body.trim_start_matches('/');
            // The folder part before the first wildcard is a plain path.
            let wild = body.find(['*', '?', '[', '{']).unwrap_or(body.len());
            let split = body[..wild].rfind('/').map_or(0, |i| i + 1);
            let (prefix, rest) = body.split_at(split);
            let base = normalize(&anchor.join(prefix));
            if rest.is_empty() {
                continue;
            }
            let Some(glob) = crate::frameworks::Glob::new(rest) else {
                return GlobMatch::Unsupported;
            };
            let hidden_ok = rest.starts_with('.') || rest.contains("/.");
            let modules_ok = rest.contains("node_modules");
            if negated {
                exclude.push((base, glob));
            } else {
                include.push((base, glob, hidden_ok, modules_ok));
            }
        }
        let mut found = Vec::new();
        for (base, glob, hidden_ok, modules_ok) in &include {
            let start = self.sorted.partition_point(|p| p < base);
            for path in self.sorted[start..]
                .iter()
                .take_while(|p| p.starts_with(base))
            {
                if *path == file {
                    continue;
                }
                let Some(rel) = relative_path(base, path) else {
                    continue;
                };
                let skipped = rel.split('/').any(|segment| {
                    (!hidden_ok && segment.starts_with('.'))
                        || (!modules_ok && segment == "node_modules")
                });
                if skipped || !glob.matches(&rel) {
                    continue;
                }
                let excluded = exclude.iter().any(|(base, glob)| {
                    relative_path(base, path).is_some_and(|rel| glob.matches(&rel))
                });
                if !excluded {
                    found.push(path.clone());
                    if found.len() > MAX_GLOB_FILES {
                        return GlobMatch::Unsupported;
                    }
                }
            }
        }
        found.sort();
        found.dedup();
        if found.is_empty() {
            GlobMatch::None
        } else {
            GlobMatch::Files(found)
        }
    }

    /// The folder Vite's `/` patterns start from for the file `ctx`
    /// describes.
    fn vite_root(&self, ctx: &Context) -> PathBuf {
        let dir = normalize(ctx.dir);
        let package = ctx.package.map(|p| normalize(&p.dir));
        for ancestor in dir.ancestors() {
            if self.vite.contains(ancestor) {
                return ancestor.to_path_buf();
            }
            if package.as_deref() == Some(ancestor) {
                return ancestor.to_path_buf();
            }
        }
        package.unwrap_or(dir)
    }

    /// The file a runtime load (`new Worker`, `new URL`, `spawn`) names,
    /// relative to the importer, with or without a leading `./`.
    pub fn runtime_file(&self, ctx: &Context, specifier: &str) -> Option<PathBuf> {
        let base = normalize(&ctx.dir.join(specifier));
        self.find(&base, false).map(|(path, _)| path)
    }

    fn mode(&self, ctx: &Context) -> Mode {
        ctx.config.map(TsConfig::mode).unwrap_or_default()
    }

    /// The `import` / `require` condition of a request.
    fn conditions<'c>(&self, ctx: &'c Context, request: Request) -> Conditions<'c> {
        let mode = match request.syntax {
            Syntax::Require => "require",
            Syntax::DynamicImport => "import",
            // TypeScript compiles a static import in a CommonJS file to
            // `require` under `node16` / `nodenext`.
            Syntax::Import
                if ctx.ts && ctx.format == Format::Cjs && self.mode(ctx) == Mode::Node16 =>
            {
                "require"
            }
            Syntax::Import => "import",
        };
        Conditions {
            mode,
            types_only: request.types_only,
            custom: ctx
                .config
                .and_then(|c| c.custom_conditions.as_deref())
                .unwrap_or(&[]),
        }
    }

    fn relative(&self, ctx: &Context, request: Request) -> Outcome {
        let base = normalize(&ctx.dir.join(request.specifier));
        // Node.js and TypeScript's `node16` need the full path of an ES
        // module: `./x` for `./x.js` works only through a bundler.
        let strict =
            self.mode(ctx) == Mode::Node16 && self.conditions(ctx, request).mode == "import";
        let targets = self
            .files(&base, ctx.ts, request.types_only)
            .into_iter()
            .map(|(path, condition, how)| Target {
                path,
                condition: condition.map(str::to_string),
                certainty: if strict && matches!(how, How::Appended | How::Index) {
                    Certainty::Inferred
                } else {
                    Certainty::Direct
                },
            })
            .collect();
        Outcome::found(targets)
    }

    /// Bare specifiers, in TypeScript's order: `paths`, `baseUrl`, then
    /// package lookup (`#imports`, the repository's own packages); then the
    /// aliases frameworks set without a config.
    fn bare(&self, ctx: &Context, request: Request) -> Outcome {
        let specifier = request.specifier;
        let mut local = is_local_prefix(specifier);
        if let Some(config) = ctx.config {
            match self.through_paths(ctx, config, request) {
                Some(Some(found)) => return found,
                // A `paths` pattern matched but none of its targets exist:
                // TypeScript goes on to packages, skipping `baseUrl`.
                Some(None) => local = true,
                None => {
                    if let Some(base) = &config.base_url {
                        let found =
                            self.local_targets(&normalize(&base.join(specifier)), ctx, request);
                        if found.is_found() {
                            return found;
                        }
                    }
                }
            }
        }
        if specifier.starts_with('#') {
            return self.package_imports(ctx, request);
        }
        if let Some(found) = self.workspace(ctx, request) {
            return found;
        }
        if let Some(found) = self.default_alias(ctx, request) {
            return found;
        }
        if local {
            Outcome::unresolved()
        } else {
            Outcome::external()
        }
    }

    /// `None` when no `paths` pattern matches; `Some(None)` when one does
    /// but none of its targets exist.
    fn through_paths(
        &self,
        ctx: &Context,
        config: &TsConfig,
        request: Request,
    ) -> Option<Option<Outcome>> {
        let (paths, defined_in) = config.paths.as_ref()?;
        let base = config.base_url.as_ref().unwrap_or(defined_in);
        let specifier = request.specifier;
        // An exact pattern wins; then the `*` pattern with the longest
        // prefix (TypeScript's `matchPatternOrExact`).
        let (targets, star) = match paths.iter().find(|(p, _)| p == specifier) {
            Some((_, targets)) => (targets, ""),
            None => paths
                .iter()
                .filter_map(|(pattern, targets)| {
                    let (prefix, suffix) = pattern.split_once('*')?;
                    let middle = specifier.strip_prefix(prefix)?.strip_suffix(suffix)?;
                    Some((prefix.len(), targets, middle))
                })
                .max_by_key(|(len, _, _)| *len)
                .map(|(_, targets, middle)| (targets, middle))?,
        };
        for target in targets {
            let path = normalize(&base.join(target.replace('*', star)));
            let found = self.local_targets(&path, ctx, request);
            if found.is_found() {
                return Some(Some(found));
            }
        }
        Some(None)
    }

    /// The files at `path` (a location inside the repository named by an
    /// alias), as a direct outcome.
    fn local_targets(&self, path: &Path, ctx: &Context, request: Request) -> Outcome {
        Outcome::found(
            self.files(path, ctx.ts, request.types_only)
                .into_iter()
                .map(|(path, condition, _)| direct(path, condition))
                .collect(),
        )
    }

    /// `#x` through the `imports` field of the importer's nearest
    /// `package.json` (`PACKAGE_IMPORTS_RESOLVE`).
    fn package_imports(&self, ctx: &Context, request: Request) -> Outcome {
        let Some(package) = ctx.package else {
            return Outcome::unresolved();
        };
        let Some(Json::Object(map)) = &package.imports else {
            return Outcome::unresolved();
        };
        if request.specifier == "#" || request.specifier.starts_with("#/") {
            return Outcome::unresolved();
        }
        let Some((entry, star)) = lookup(map, request.specifier) else {
            return Outcome::unresolved();
        };
        let conditions = self.conditions(ctx, request);
        let mut found = Vec::new();
        walk(entry, &conditions, &mut Vec::new(), &mut found);
        let mut targets = Vec::new();
        for (target, label) in found {
            let target = substitute(&target, star);
            if target.starts_with("./") {
                targets.extend(labelled(
                    self.package_target(ctx, package, &target, request),
                    &label,
                ));
            } else if !target.starts_with('.') && !target.starts_with('/') {
                // A bare target names a package (`"#dep": "dep"`).
                let inner = Request {
                    specifier: &target,
                    ..request
                };
                let outcome = self.workspace(ctx, inner).unwrap_or_else(Outcome::external);
                targets.extend(labelled(outcome.targets, &label));
            }
        }
        Outcome::found(dedupe(targets))
    }

    /// An import of one of the repository's own packages, by name.
    fn workspace(&self, ctx: &Context, request: Request) -> Option<Outcome> {
        let (name, subpath) = split_package(request.specifier)?;
        let dirs = self.by_name.get(name)?;
        let chosen: Vec<&PathBuf> = if dirs.len() == 1 {
            dirs.iter().collect()
        } else if let Some(own) = ctx.package.filter(|p| dirs.contains(&p.dir)) {
            // A package importing itself by name.
            vec![&own.dir]
        } else {
            // Same-named packages in separate workspaces (a repository of
            // example monorepos) do not compete: the importer's own
            // workspace, the innermost declaring it, decides.
            let own_root = ctx
                .package
                .and_then(|p| self.members.get(&p.dir))
                .and_then(|roots| roots.iter().max_by_key(|r| r.components().count()));
            let in_own: Vec<&PathBuf> = match own_root {
                Some(root) => dirs
                    .iter()
                    .filter(|d| self.members.get(*d).is_some_and(|r| r.contains(root)))
                    .collect(),
                None => Vec::new(),
            };
            let members: Vec<&PathBuf> = dirs
                .iter()
                .filter(|d| self.members.contains_key(*d))
                .collect();
            if !in_own.is_empty() {
                in_own
            } else if members.len() == 1 {
                members
            } else {
                dirs.iter().collect()
            }
        };
        if let [dir] = chosen.as_slice() {
            let package = self.packages.get(*dir)?;
            return Some(self.package(ctx, package, &subpath, request));
        }
        let mut candidates = Vec::new();
        for dir in chosen {
            if let Some(package) = self.packages.get(dir) {
                candidates.extend(self.package(ctx, package, &subpath, request).targets);
            }
        }
        let candidates = dedupe(candidates);
        Some(if candidates.is_empty() {
            Outcome::unresolved()
        } else {
            Outcome {
                resolution: Resolution::Ambiguous,
                targets: candidates,
            }
        })
    }

    /// `subpath` (`.` or `./x`) of `package`.
    fn package(
        &self,
        ctx: &Context,
        package: &Package,
        subpath: &str,
        request: Request,
    ) -> Outcome {
        if let Some(exports) = &package.exports {
            let Some((entry, star)) = exports_entry(exports, subpath) else {
                return Outcome::unresolved();
            };
            let conditions = self.conditions(ctx, request);
            let mut found = Vec::new();
            walk(entry, &conditions, &mut Vec::new(), &mut found);
            let mut targets = Vec::new();
            for (target, label) in found {
                let target = substitute(&target, star);
                targets.extend(labelled(
                    self.package_target(ctx, package, &target, request),
                    &label,
                ));
            }
            return Outcome::found(dedupe(targets));
        }

        let mut targets = Vec::new();
        if subpath == "." {
            let order: &[&str] = if request.types_only {
                &["types", "typings", "main"]
            } else {
                &["main"]
            };
            let main = order.iter().find_map(|field| {
                let found = self.package_target(ctx, package, package.field(field)?, request);
                (!found.is_empty()).then_some(found)
            });
            match main {
                Some(found) => targets.extend(found),
                None => targets.extend(self.package_index(ctx, package, request)),
            }
            for (field, condition) in ENTRY_FIELDS {
                if request.types_only && matches!(*field, "types" | "typings") {
                    continue;
                }
                if let Some(value) = package.field(field) {
                    let found = self.package_target(ctx, package, value, request);
                    targets.extend(labelled(found, &Some(condition.to_string())));
                }
            }
        } else {
            let rest = &subpath[2..];
            targets.extend(self.package_target(ctx, package, rest, request));
            if targets.is_empty() {
                let src = normalize(&package.dir.join("src").join(rest));
                targets.extend(
                    self.files(&src, true, request.types_only)
                        .into_iter()
                        .map(|(path, condition, _)| inferred(path, condition)),
                );
            }
        }
        Outcome::found(dedupe(targets))
    }

    /// A package without `exports` or `main`: its `index`, else
    /// `src/index` (a guess).
    fn package_index(&self, ctx: &Context, package: &Package, request: Request) -> Vec<Target> {
        let index = self.files(&package.dir.join("index"), ctx.ts, request.types_only);
        if !index.is_empty() {
            return index
                .into_iter()
                .map(|(path, condition, _)| direct(path, condition))
                .collect();
        }
        self.files(
            &package.dir.join("src").join("index"),
            true,
            request.types_only,
        )
        .into_iter()
        .map(|(path, condition, _)| inferred(path, condition))
        .collect()
    }

    /// The files a path in `package.json` (`./dist/index.js`) stands for:
    /// the file itself; else its source through the package's
    /// `tsconfig.json` `outDir` and `rootDir`; else the same path under
    /// `src/`, guessed.
    fn package_target(
        &self,
        ctx: &Context,
        package: &Package,
        target: &str,
        request: Request,
    ) -> Vec<Target> {
        let rel = target.strip_prefix("./").unwrap_or(target);
        if rel.split('/').any(|s| s == ".." || s == "node_modules") {
            return Vec::new();
        }
        let base = normalize(&package.dir.join(rel));
        let found = self.files(&base, ctx.ts, request.types_only);
        if !found.is_empty() {
            return found
                .into_iter()
                .map(|(path, condition, _)| direct(path, condition))
                .collect();
        }
        if let Some(source) = self.built_source(package, &base) {
            let found = self.files(&source, true, request.types_only);
            if !found.is_empty() {
                return found
                    .into_iter()
                    .map(|(path, condition, _)| direct(path, condition))
                    .collect();
            }
        }
        if let Some(source) = source_for_built(rel) {
            return self
                .files(
                    &normalize(&package.dir.join(source)),
                    true,
                    request.types_only,
                )
                .into_iter()
                .map(|(path, condition, _)| inferred(path, condition))
                .collect();
        }
        Vec::new()
    }

    /// `outDir/x.js` → `rootDir/x`, from the package's own `tsconfig.json`
    /// or a config it references.
    fn built_source(&self, package: &Package, built: &Path) -> Option<PathBuf> {
        let project = self.configs.get(self.projects.get(&package.dir)?)?;
        let configs = std::iter::once(project).chain(
            project
                .references
                .iter()
                .filter_map(|r| self.configs.get(r)),
        );
        for config in configs {
            let (Some(out), Some(root)) = (&config.out_dir, config.source_root()) else {
                continue;
            };
            if let Ok(rest) = built.strip_prefix(out) {
                return Some(strip_js_extension(&root.join(rest)));
            }
        }
        None
    }

    /// `@/x`, `~/x` (Vite, Nuxt, Create React App templates), `$lib`
    /// (SvelteKit) and `@site` (Docusaurus), set by the framework rather
    /// than a config CodeDna reads.
    fn default_alias(&self, ctx: &Context, request: Request) -> Option<Outcome> {
        let specifier = request.specifier;
        let mut tries: Vec<(PathBuf, Certainty)> = Vec::new();
        let anchors: Vec<&Path> = {
            let mut list: Vec<&Path> = Vec::new();
            if let Some(p) = ctx.package {
                list.push(&p.dir);
            }
            if let Some(c) = ctx.config {
                if !list.contains(&c.dir.as_path()) {
                    list.push(&c.dir);
                }
            }
            list
        };
        let marker = |set: &HashSet<PathBuf>| {
            ctx.dir
                .ancestors()
                .find(|d| set.contains(*d))
                .map(Path::to_path_buf)
        };
        if let Some(rest) = specifier
            .strip_prefix("@/")
            .or_else(|| specifier.strip_prefix("~/"))
        {
            for anchor in &anchors {
                tries.push((anchor.join("src").join(rest), Certainty::Inferred));
                tries.push((anchor.join(rest), Certainty::Inferred));
            }
        } else if specifier == "$lib" || specifier.starts_with("$lib/") {
            let rest = specifier
                .strip_prefix("$lib")
                .unwrap_or("")
                .trim_start_matches('/');
            match marker(&self.svelte) {
                Some(dir) => tries.push((dir.join("src/lib").join(rest), Certainty::Direct)),
                None => {
                    for anchor in &anchors {
                        tries.push((anchor.join("src/lib").join(rest), Certainty::Inferred));
                    }
                }
            }
        } else {
            let rest = specifier.strip_prefix("@site/")?;
            match marker(&self.docusaurus) {
                Some(dir) => tries.push((dir.join(rest), Certainty::Direct)),
                None => {
                    for anchor in &anchors {
                        tries.push((anchor.join(rest), Certainty::Inferred));
                    }
                }
            }
        }
        for (path, certainty) in tries {
            let found = self.files(&normalize(&path), ctx.ts, request.types_only);
            if !found.is_empty() {
                return Some(Outcome::found(
                    found
                        .into_iter()
                        .map(|(path, condition, _)| Target {
                            path,
                            condition: condition.map(str::to_string),
                            certainty,
                        })
                        .collect(),
                ));
            }
        }
        Some(Outcome::unresolved())
    }

    /// The file `base` stands for, with how it was found. A TypeScript
    /// importer prefers the TypeScript source of `./x.js`; JavaScript takes
    /// the path as written first.
    fn find(&self, base: &Path, ts: bool) -> Option<(PathBuf, How)> {
        let name = base.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let substitutes: &[&str] = match name.rsplit_once('.').map(|(_, e)| e) {
            Some("js") => &["ts", "tsx", "d.ts"],
            Some("jsx") => &["tsx", "ts", "d.ts"],
            Some("mjs") => &["mts", "d.mts"],
            Some("cjs") => &["cts", "d.cts"],
            _ => &[],
        };
        let exact = || self.exists(base).then(|| (base.to_path_buf(), How::Exact));
        if !ts {
            if let Some(found) = exact() {
                return Some(found);
            }
        }
        if !substitutes.is_empty() {
            let stem = base.with_extension("");
            for ext in substitutes {
                let candidate = append_extension(&stem, ext);
                if self.exists(&candidate) {
                    return Some((candidate, How::Substituted));
                }
            }
        }
        if ts {
            if let Some(found) = exact() {
                return Some(found);
            }
        }
        for ext in if ts { TS_APPEND } else { JS_APPEND } {
            let candidate = append_extension(base, ext);
            if self.exists(&candidate) {
                return Some((candidate, How::Appended));
            }
        }
        for index in if ts { TS_INDEX } else { JS_INDEX } {
            let candidate = base.join(index);
            if self.exists(&candidate) {
                return Some((candidate, How::Index));
            }
        }
        None
    }

    /// [`Self::find`], and when that lands on a declaration file next to
    /// its JavaScript, both: the code for running, the declaration under
    /// the `types` condition. A types-only import gets the declaration.
    fn files(
        &self,
        base: &Path,
        ts: bool,
        types_only: bool,
    ) -> Vec<(PathBuf, Option<&'static str>, How)> {
        let Some((path, how)) = self.find(base, ts) else {
            return Vec::new();
        };
        // `./x.js` with both `x.js` and `x.ts` present: TypeScript checks
        // against `x.ts`, but Node.js and bundlers load `x.js` as written.
        if !types_only && how == How::Substituted && self.exists(base) {
            return vec![
                (base.to_path_buf(), None, How::Exact),
                (path, Some("types"), how),
            ];
        }
        if !types_only {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let code: Option<(&str, &[&str])> = if let Some(stem) = name.strip_suffix(".d.ts") {
                Some((stem, &["js", "jsx"]))
            } else if let Some(stem) = name.strip_suffix(".d.mts") {
                Some((stem, &["mjs"]))
            } else {
                name.strip_suffix(".d.cts").map(|stem| (stem, &["cjs"][..]))
            };
            if let Some((stem, exts)) = code {
                let sibling = exts
                    .iter()
                    .map(|ext| path.with_file_name(format!("{stem}.{ext}")))
                    .find(|p| self.exists(p));
                if let Some(sibling) = sibling {
                    return vec![(sibling, None, how), (path, Some("types"), how)];
                }
            }
        }
        vec![(path, None, how)]
    }
}

/// Reads configs once each, following `extends`.
#[derive(Default)]
struct ConfigLoader {
    done: HashMap<PathBuf, TsConfig>,
}

impl ConfigLoader {
    /// The config at `path` as it applies to the files it covers.
    fn load(&mut self, resolver: &Resolver, path: &Path) -> TsConfig {
        let mut config = self.effective(resolver, path, &mut Vec::new());
        config.expand_config_dir();
        config
    }

    fn effective(
        &mut self,
        resolver: &Resolver,
        path: &Path,
        stack: &mut Vec<PathBuf>,
    ) -> TsConfig {
        let dir = path.parent().unwrap_or(Path::new("")).to_path_buf();
        if let Some(done) = self.done.get(path) {
            return done.clone();
        }
        // An `extends` cycle: TypeScript reports it; stop here.
        if stack.iter().any(|p| p == path) || !resolver.exists(path) {
            return TsConfig {
                dir,
                ..TsConfig::default()
            };
        }
        let Some(json) = read_json(path, true) else {
            return TsConfig {
                dir,
                ..TsConfig::default()
            };
        };
        let mut config = own_options(&json, &dir);
        stack.push(path.to_path_buf());
        // Later entries of an `extends` list override earlier ones.
        let bases: Vec<PathBuf> = json
            .get("extends")
            .map(Json::strings)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|e| extends_path(resolver, &dir, e))
            .collect();
        for base in bases.iter().rev() {
            let inherited = self.effective(resolver, base, stack);
            config.inherit(&inherited);
        }
        stack.pop();
        self.done.insert(path.to_path_buf(), config.clone());
        config
    }
}

/// The options a config sets itself, with paths made absolute.
fn own_options(json: &Json, dir: &Path) -> TsConfig {
    let mut config = TsConfig {
        dir: dir.to_path_buf(),
        ..TsConfig::default()
    };
    let path_of = |value: &Json| value.as_str().map(|s| config_path(dir, s));
    if let Some(options) = json.get("compilerOptions") {
        config.base_url = options.get("baseUrl").and_then(path_of);
        config.out_dir = options.get("outDir").and_then(path_of);
        config.root_dir = options.get("rootDir").and_then(path_of);
        let lower = |key: &str| {
            options
                .get(key)
                .and_then(Json::as_str)
                .map(str::to_ascii_lowercase)
        };
        config.module_resolution = lower("moduleResolution");
        config.module = lower("module");
        config.custom_conditions = options
            .get("customConditions")
            .map(|c| c.strings().into_iter().map(str::to_string).collect());
        if let Some(Json::Object(entries)) = options.get("paths") {
            let paths = entries
                .iter()
                .map(|(pattern, targets)| {
                    let targets = targets.strings().into_iter().map(str::to_string).collect();
                    (pattern.clone(), targets)
                })
                .collect();
            config.paths = Some((paths, dir.to_path_buf()));
        }
    }
    if let Some(Json::Array(items)) = json.get("include") {
        let prefixes = items
            .iter()
            .filter_map(Json::as_str)
            .map(|pattern| {
                let fixed: Vec<&str> = pattern
                    .split('/')
                    .take_while(|s| !s.contains(['*', '?']))
                    .collect();
                config_path(dir, &fixed.join("/"))
            })
            .collect();
        config.include = Some(prefixes);
    }
    if let Some(Json::Array(files)) = json.get("files") {
        config.no_files = Some(files.is_empty());
    }
    if let Some(Json::Array(references)) = json.get("references") {
        config.references = references
            .iter()
            .filter_map(|r| r.get("path")?.as_str())
            .map(|p| {
                let path = normalize(&dir.join(p));
                if p.ends_with(".json") {
                    path
                } else {
                    path.join("tsconfig.json")
                }
            })
            .collect();
    }
    config
}

/// A path from a config, relative to its directory. One starting with
/// `${configDir}` keeps the template until the config in use is known.
fn config_path(dir: &Path, value: &str) -> PathBuf {
    if value.starts_with(CONFIG_DIR) {
        normalize(Path::new(value))
    } else {
        normalize(&dir.join(value))
    }
}

/// The config an `extends` entry names: a path, or a file inside one of
/// the repository's own packages. Configs from installed packages
/// (`@tsconfig/node20`) are outside the repository and skipped.
fn extends_path(resolver: &Resolver, dir: &Path, entry: &str) -> Option<PathBuf> {
    let with_json = |path: PathBuf| {
        if resolver.exists(&path) {
            Some(path)
        } else {
            let json = append_extension(&path, "json");
            resolver.exists(&json).then_some(json)
        }
    };
    if entry.starts_with('.') || entry.starts_with('/') {
        return with_json(normalize(&dir.join(entry)));
    }
    let (name, subpath) = split_package(entry)?;
    let package_dir = resolver.by_name.get(name)?.first()?;
    if subpath == "." {
        return with_json(package_dir.join("tsconfig.json"));
    }
    with_json(normalize(&package_dir.join(&subpath[2..])))
}

/// Walk an `exports` / `imports` entry (`PACKAGE_TARGET_RESOLVE`), adding
/// every target string with the conditions it depends on. Returns whether
/// the entry settled the match, so the conditions after it are not tried:
/// a string target settles even when its file is missing, and `null`
/// settles with nothing (the path is blocked).
fn walk(
    entry: &Json,
    conditions: &Conditions,
    labels: &mut Vec<String>,
    out: &mut Vec<(String, Option<String>)>,
) -> bool {
    match entry {
        Json::String(target) => {
            let label = (!labels.is_empty()).then(|| labels.join("+"));
            out.push((target.clone(), label));
            true
        }
        Json::Null => true,
        // The first entry that settles wins.
        Json::Array(items) => items.iter().any(|item| walk(item, conditions, labels, out)),
        Json::Object(entries) => {
            for (key, value) in entries {
                match conditions.decide(key) {
                    Decision::Yes => {
                        if walk(value, conditions, labels, out) {
                            return true;
                        }
                    }
                    Decision::No => {}
                    Decision::Maybe => {
                        labels.push(key.clone());
                        walk(value, conditions, labels, out);
                        labels.pop();
                    }
                }
            }
            false
        }
        Json::Bool(_) | Json::Number(_) => false,
    }
}

/// The `exports` entry for `subpath` and what its `*` matched.
/// `exports` that is a string, an array or an object of conditions is
/// the entry for `.` alone; an object mixing `.` keys and conditions is
/// invalid.
fn exports_entry<'a>(exports: &'a Json, subpath: &'a str) -> Option<(&'a Json, &'a str)> {
    let Json::Object(map) = exports else {
        return (subpath == ".").then_some((exports, ""));
    };
    let dotted = map.iter().filter(|(k, _)| k.starts_with('.')).count();
    if dotted == 0 {
        return (subpath == ".").then_some((exports, ""));
    }
    if dotted != map.len() {
        return None;
    }
    lookup(map, subpath)
}

/// `key` in a subpath map: the exact key, else the `*` pattern with the
/// longest prefix, then the longest key (`PATTERN_KEY_COMPARE`).
fn lookup<'a>(map: &'a [(String, Json)], key: &'a str) -> Option<(&'a Json, &'a str)> {
    if !key.contains('*') {
        if let Some((_, entry)) = map.iter().find(|(k, _)| k == key && !k.contains('*')) {
            return Some((entry, ""));
        }
    }
    map.iter()
        .filter_map(|(pattern, entry)| {
            let (prefix, suffix) = pattern.split_once('*')?;
            if suffix.contains('*') {
                return None;
            }
            let middle = key.strip_prefix(prefix)?.strip_suffix(suffix)?;
            (!middle.is_empty()).then_some((prefix.len(), pattern.len(), entry, middle))
        })
        .max_by_key(|(prefix, len, _, _)| (*prefix, *len))
        .map(|(_, _, entry, middle)| (entry, middle))
}

/// Replace every `*` of a pattern target.
fn substitute(target: &str, star: &str) -> String {
    if star.is_empty() {
        target.to_string()
    } else {
        target.replace('*', star)
    }
}

/// `@scope/name/sub` → (`@scope/name`, `./sub`); `name` → (`name`, `.`).
fn split_package(specifier: &str) -> Option<(&str, String)> {
    let mut parts = specifier.splitn(if specifier.starts_with('@') { 3 } else { 2 }, '/');
    let first = parts.next()?;
    let name_len = if specifier.starts_with('@') {
        first.len() + 1 + parts.next()?.len()
    } else {
        first.len()
    };
    let name = &specifier[..name_len];
    if name.is_empty() || name.ends_with('/') {
        return None;
    }
    let rest = &specifier[name_len..];
    let subpath = match rest.strip_prefix('/') {
        Some(rest) if !rest.is_empty() => format!("./{rest}"),
        _ => ".".to_string(),
    };
    Some((name, subpath))
}

/// Prefixes no npm package name can start with: always the project's
/// own code.
fn is_local_prefix(specifier: &str) -> bool {
    specifier.starts_with("@/")
        || specifier.starts_with("~/")
        || specifier.starts_with('#')
        || specifier == "$lib"
        || specifier.starts_with("$lib/")
}

/// `./dist/x.js` → `src/x`: where the source of a built file usually
/// lives (`./dist/src/x.js`, built from the package root, → `src/x`).
/// `None` when the path is not under a build folder.
fn source_for_built(target: &str) -> Option<String> {
    let trimmed = target.strip_prefix("./").unwrap_or(target);
    let (first, rest) = trimmed.split_once('/')?;
    if !BUILD_DIRS.contains(&first) {
        return None;
    }
    let rest = [".d.ts", ".d.mts", ".d.cts", ".js", ".mjs", ".cjs", ".jsx"]
        .iter()
        .find_map(|ext| rest.strip_suffix(ext))
        .unwrap_or(rest);
    if rest.starts_with("src/") {
        Some(rest.to_string())
    } else {
        Some(format!("src/{rest}"))
    }
}

/// `x.d.ts`, `x.js`, `x.mjs`, ... → `x`.
fn strip_js_extension(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    for ext in [".d.ts", ".d.mts", ".d.cts", ".js", ".mjs", ".cjs", ".jsx"] {
        if let Some(stem) = text.strip_suffix(ext) {
            return PathBuf::from(stem);
        }
    }
    path.to_path_buf()
}

/// Whether path `segments` match glob `pattern` segments, where `**` is
/// any number of segments and `*` / `?` work inside one.
fn glob_segments(pattern: &[&str], segments: &[&str]) -> bool {
    match pattern.split_first() {
        None => segments.is_empty(),
        Some((&"**", rest)) => {
            (0..=segments.len()).any(|skip| glob_segments(rest, &segments[skip..]))
        }
        Some((first, rest)) => match segments.split_first() {
            Some((segment, others)) => {
                crate::workspaces::wildcard(first, segment) && glob_segments(rest, others)
            }
            None => false,
        },
    }
}

fn direct(path: PathBuf, condition: Option<&str>) -> Target {
    Target {
        path,
        condition: condition.map(str::to_string),
        certainty: Certainty::Direct,
    }
}

fn inferred(path: PathBuf, condition: Option<&str>) -> Target {
    Target {
        path,
        condition: condition.map(str::to_string),
        certainty: Certainty::Inferred,
    }
}

/// Put `targets` under the package condition `label` as well.
fn labelled(targets: Vec<Target>, label: &Option<String>) -> Vec<Target> {
    let Some(label) = label else {
        return targets;
    };
    targets
        .into_iter()
        .map(|mut t| {
            t.condition = Some(match t.condition.take() {
                Some(inner) => format!("{label}+{inner}"),
                None => label.clone(),
            });
            t
        })
        .collect()
}

/// One target per file: unconditional when any branch reaches the file
/// unconditionally, else the first condition; direct when any branch is.
fn dedupe(targets: Vec<Target>) -> Vec<Target> {
    let mut out: Vec<Target> = Vec::new();
    for target in targets {
        match out.iter_mut().find(|t| t.path == target.path) {
            Some(existing) => {
                if target.condition.is_none() {
                    existing.condition = None;
                }
                if target.certainty == Certainty::Direct {
                    existing.certainty = Certainty::Direct;
                }
            }
            None => out.push(target),
        }
    }
    out
}

/// Append `.ext` to `path` without replacing a dotted suffix it already has.
fn append_extension(path: &Path, ext: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".");
    name.push(ext);
    PathBuf::from(name)
}

fn read_text(path: &Path) -> Option<String> {
    let small = fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() <= MAX_CONFIG_BYTES);
    if !small {
        return None;
    }
    fs::read_to_string(path).ok()
}

/// A JSON config, optionally with comments and trailing commas.
fn read_json(path: &Path, jsonc: bool) -> Option<Json> {
    let text = read_text(path)?;
    if jsonc {
        Json::parse_jsonc(&text)
    } else {
        Json::parse(&text)
    }
}

/// Bundler and test-runner configs that are programs, not data: their
/// aliases are only known by running them.
const EXECUTABLE_CONFIGS: &[&str] = &[
    "vite.config",
    "vitest.config",
    "webpack.config",
    "rollup.config",
    "jest.config",
    "babel.config",
    "next.config",
    "nuxt.config",
    "svelte.config",
    "astro.config",
    "rspack.config",
    "rsbuild.config",
    "craco.config",
    "metro.config",
];

/// An `unsupported` fact for every executable config that sets module
/// aliases (`resolve.alias`, `moduleNameMapper`, ...), which CodeDna
/// cannot read without running the config. Imports through those aliases
/// may show up as unresolved or external; the fact says why.
pub fn executable_config_facts(files: &[PathBuf]) -> Vec<DependencyFact> {
    let mut facts = Vec::new();
    for file in files {
        let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let Some((stem, ext)) = name.rsplit_once('.') else {
            continue;
        };
        if !matches!(ext, "js" | "mjs" | "cjs" | "ts" | "mts" | "cts")
            || !EXECUTABLE_CONFIGS.contains(&stem)
        {
            continue;
        }
        let Some(text) = read_text(file) else {
            continue;
        };
        let found = text.lines().enumerate().find_map(|(i, line)| {
            ["moduleNameMapper", "alias"]
                .into_iter()
                .find(|key| has_word(line, key))
                .map(|key| (i + 1, key))
        });
        if let Some((line, key)) = found {
            let source = normalize(file);
            facts.push(
                DependencyFact::new(
                    &source,
                    key,
                    DependencyKind::ConfiguredEntry,
                    Certainty::Possibility,
                    "js.config_alias_unsupported",
                )
                .line(line)
                .resolution(Resolution::Unsupported),
            );
        }
    }
    facts
}

/// Whether `word` appears in `line` as a whole identifier.
fn has_word(line: &str, word: &str) -> bool {
    let is_ident = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '$';
    line.match_indices(word).any(|(at, _)| {
        let before = line[..at].chars().next_back();
        let after = line[at + word.len()..].chars().next();
        !before.is_some_and(is_ident) && !after.is_some_and(is_ident)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_package_names() {
        assert_eq!(split_package("@a/b"), Some(("@a/b", ".".to_string())));
        assert_eq!(
            split_package("@a/b/c/d"),
            Some(("@a/b", "./c/d".to_string()))
        );
        assert_eq!(split_package("x/y"), Some(("x", "./y".to_string())));
        assert_eq!(split_package("x"), Some(("x", ".".to_string())));
        assert_eq!(split_package("@a"), None);
    }

    #[test]
    fn exports_pattern_prefers_longest_prefix() {
        let map = match Json::parse(
            r#"{"./*": "./a/*.js", "./x/*": "./b/*.js", "./x/*.js": "./c/*.js"}"#,
        ) {
            Some(Json::Object(map)) => map,
            _ => unreachable!(),
        };
        let (entry, star) = lookup(&map, "./x/y.js").unwrap();
        assert_eq!(entry.as_str(), Some("./c/*.js"));
        assert_eq!(star, "y");
        let (entry, star) = lookup(&map, "./z").unwrap();
        assert_eq!(entry.as_str(), Some("./a/*.js"));
        assert_eq!(star, "z");
    }

    #[test]
    fn unknown_conditions_become_alternatives() {
        let entry = Json::parse(
            r#"{"browser": {"import": "./b.mjs", "require": "./b.cjs"}, "node": "./n.js", "import": "./i.mjs", "default": "./d.js"}"#,
        )
        .unwrap();
        let conditions = Conditions {
            mode: "import",
            types_only: false,
            custom: &[],
        };
        let mut out = Vec::new();
        assert!(walk(&entry, &conditions, &mut Vec::new(), &mut out));
        assert_eq!(
            out,
            [
                ("./b.mjs".to_string(), Some("browser".to_string())),
                ("./n.js".to_string(), Some("node".to_string())),
                ("./i.mjs".to_string(), None),
            ]
        );
    }

    #[test]
    fn matches_workspace_globs() {
        let g = |p: &str, s: &str| {
            glob_segments(
                &p.split('/').collect::<Vec<_>>(),
                &s.split('/').collect::<Vec<_>>(),
            )
        };
        assert!(g("packages/*", "packages/ui"));
        assert!(!g("packages/*", "packages/ui/x"));
        assert!(g("packages/**", "packages/ui/x"));
        assert!(g("apps/web-*", "apps/web-a"));
        assert!(!g("apps/web-*", "tools/web-a"));
    }

    #[test]
    fn finds_whole_words() {
        assert!(has_word("resolve: { alias: {", "alias"));
        assert!(!has_word("const aliases = 1", "alias"));
        assert!(!has_word("myalias", "alias"));
    }
}
