//! File roles: which layer each source file plays, from settings, code
//! signs, folder and file names, and imports.

use super::{display_path, CodeSign};
use crate::cache::Cache;
use crate::safe_text::sanitize;
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Layers in top-down order, with the folder and file names that mark them.
pub(super) const LAYERS: &[(&str, &[&str])] = &[
    (
        "UI",
        &[
            "components",
            "component",
            "pages",
            "screens",
            "layouts",
            "ui",
            "widgets",
            "hooks",
            "frontend",
            "client",
        ],
    ),
    (
        "Routes",
        &[
            "routes",
            "route",
            "router",
            "routers",
            "controllers",
            "controller",
            "handlers",
            "handler",
            "endpoints",
            "resolvers",
            "resolver",
            "api",
            "views",
            "urls",
        ],
    ),
    (
        "Middleware",
        &[
            "middleware",
            "middlewares",
            "guards",
            "guard",
            "interceptors",
        ],
    ),
    (
        "Services",
        &[
            "services",
            "service",
            "usecases",
            "use_cases",
            "use-cases",
            "domain",
            "jobs",
            "workers",
        ],
    ),
    (
        "Data",
        &[
            "models",
            "model",
            "entities",
            "entity",
            "repositories",
            "repository",
            "repos",
            "db",
            "database",
            "dao",
            "schema",
            "schemas",
            "migrations",
            "prisma",
        ],
    ),
];

/// Extensions of UI files; `views/` and `pages/` only mean UI for these.
pub(super) const UI_EXTENSIONS: &[&str] = &["tsx", "jsx", "vue", "svelte", "astro"];

/// Every placed file with its layer, what placed it and what matched, plus
/// each file's code sign and the names its code overruled.
pub(super) struct Placed<'a> {
    pub(super) by_code: HashMap<&'a PathBuf, (usize, CodeSign)>,
    pub(super) layer_of: HashMap<&'a PathBuf, (usize, Source, String)>,
    pub(super) name_said: HashMap<&'a PathBuf, String>,
}

/// Place each source file in a layer: settings, then code, then names, then
/// imports.
pub(super) fn place<'a>(
    root: &Path,
    files: &'a [PathBuf],
    graph: &'a HashMap<PathBuf, Vec<PathBuf>>,
    cache: &Cache,
) -> Placed<'a> {
    let rel = |p: &Path| p.strip_prefix(root).unwrap_or(p).to_path_buf();
    let overrides = Overrides::load(root);

    // Every source file, including languages whose imports are not followed.
    let mut nodes: Vec<&PathBuf> = graph.keys().collect();
    nodes.extend(files.iter().filter(|f| {
        !graph.contains_key(*f)
            && f.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                let e = e.to_ascii_lowercase();
                e == "astro" || crate::runbook::CODE_EXTENSIONS.contains(&e.as_str())
            })
    }));
    let no_deps: Vec<PathBuf> = Vec::new();
    let deps_of = |file: &PathBuf| graph.get(file).unwrap_or(&no_deps);

    // What each file's code does, read once.
    let by_code: HashMap<&PathBuf, (usize, CodeSign)> = nodes
        .par_iter()
        .filter_map(|&file| {
            let relative = rel(file);
            if is_test(&relative) {
                return None;
            }
            cache
                .layer(file, || content_layer(&relative, &read_capped(file)?))
                .map(|found| (file, found))
        })
        .collect();

    // Settings first, then code, then names: a folder or file name is a
    // convention, the code is what the file does.
    // Each placed file: (layer, what placed it, what matched).
    let mut layer_of: HashMap<&PathBuf, (usize, Source, String)> = HashMap::new();
    // Files whose code overruled their name, and what the name suggested.
    let mut name_said: HashMap<&PathBuf, String> = HashMap::new();
    for &file in &nodes {
        let relative = rel(file);
        if let Some((index, prefix)) = overrides.layer(&relative) {
            layer_of.insert(file, (index, Source::Settings, prefix));
            continue;
        }
        let code = by_code.get(file);
        let found = match (classify(&relative), code) {
            (Some((name_index, name)), Some((code_index, sign)))
                if code_wins(*code_index, name_index) =>
            {
                if *code_index != name_index {
                    name_said.insert(file, format!("{name} ({})", LAYERS[name_index].0));
                }
                (*code_index, Source::Code, sign.sign.clone())
            }
            (Some((index, name)), _) => (index, Source::Name, name),
            (None, Some((index, sign))) => (*index, Source::Code, sign.sign.clone()),
            (None, None) => continue,
        };
        layer_of.insert(file, found);
    }

    // Sample apps in examples/ or docs/ show how the project is used, not
    // how it is built; they count only when nothing else has layers.
    let in_samples = |file: &PathBuf| crate::framework_detector::in_sample_dir(&rel(file));
    if layer_of
        .iter()
        .any(|(file, (_, source, _))| *source != Source::Settings && !in_samples(file))
    {
        layer_of.retain(|file, (_, source, _)| *source == Source::Settings || !in_samples(file));
    }

    // A file nothing else placed, imported by routes or middleware and
    // importing data, is the service layer between them.
    let mut importers: HashMap<&PathBuf, Vec<&PathBuf>> = HashMap::new();
    for (file, deps) in graph {
        for dep in deps {
            importers.entry(dep).or_default().push(file);
        }
    }
    let in_layer = |file: &PathBuf, layers: &[usize]| {
        layer_of
            .get(file)
            .is_some_and(|(index, _, _)| layers.contains(index))
    };
    let between: Vec<(&PathBuf, String)> = nodes
        .iter()
        .copied()
        .filter(|file| {
            let relative = rel(file);
            !layer_of.contains_key(file) && !is_test(&relative) && !is_shape(&relative)
        })
        .filter_map(|file| {
            let by = importers
                .get(file)?
                .iter()
                .filter(|f| in_layer(f, &[1, 2]))
                .min()?;
            let data = deps_of(file).iter().filter(|d| in_layer(d, &[4])).min()?;
            Some((
                file,
                format!(
                    "imported by {}, imports {}",
                    display_path(root, by),
                    display_path(root, data)
                ),
            ))
        })
        .collect();
    for (file, matched) in between {
        layer_of.insert(file, (3, Source::Imports, matched));
    }

    Placed {
        by_code,
        layer_of,
        name_said,
    }
}

/// Whether a file's code layer overrules the layer its name suggests.
///
/// Code wins, except that request handlers in a routes folder share the
/// middleware signature (`(req, res, next)`, `NextFunction`) without being
/// middleware.
pub(super) fn code_wins(code_index: usize, name_index: usize) -> bool {
    !(code_index == 2 && name_index == 1)
}

/// Layer index for a relative path, from its file name first, then its
/// closest matching folder. Tests mirror the layout they test, so they are
/// left out.
pub(super) fn classify(path: &Path) -> Option<(usize, String)> {
    if is_test(path) {
        return None;
    }
    let is_ui_file = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| UI_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()));
    match classify_by_name(path)? {
        // File-based routes written as components (`routes/index.tsx`) are UI.
        (1, name) if is_ui_file => Some((0, name)),
        found => Some(found),
    }
}

/// Whether a path is a test file or lives in a test folder.
pub(crate) fn is_test(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    name.contains(".test.")
        || name.contains(".spec.")
        || name.starts_with("test_")
        || name.ends_with("_test.py")
        || name.ends_with("_test.go")
        || path.parent().is_some_and(|p| {
            p.iter().filter_map(|c| c.to_str()).any(|c| {
                matches!(
                    c.to_ascii_lowercase().as_str(),
                    "test" | "tests" | "__tests__" | "spec" | "e2e"
                )
            })
        })
}

/// Layer index and the file name (`user.controller.ts`) or folder
/// (`routes/`) that marks it.
pub(super) fn classify_by_name(path: &Path) -> Option<(usize, String)> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let is_ui_file = UI_EXTENSIONS.contains(&ext.as_str());
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    // `user.controller.ts`, `user_service.py`, `UserRepository.java`.
    let marker = stem
        .rsplit(['.', '_', '-'])
        .next()
        .filter(|m| *m != stem)
        .map(str::to_string);
    for name in marker.iter().chain(std::iter::once(&stem)) {
        if let Some(index) = layer_for_name(name, is_ui_file) {
            let file = path
                .file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default();
            return Some((index, sanitize(&file)));
        }
    }

    path.parent()?
        .iter()
        .rev()
        .filter_map(|c| c.to_str())
        .find_map(|dir| {
            layer_for_name(&dir.to_ascii_lowercase(), is_ui_file)
                .map(|index| (index, format!("{}/", sanitize(dir))))
        })
}

pub(super) fn layer_for_name(name: &str, is_ui_file: bool) -> Option<usize> {
    // Server-rendered `views/` and file-routed `pages/` are routes unless the
    // file is a UI component.
    if matches!(name, "views" | "pages") && is_ui_file {
        return Some(0);
    }
    if name == "pages" {
        return Some(1);
    }
    LAYERS
        .iter()
        .position(|(_, names)| names.contains(&name))
        .filter(|&index| index != 0 || is_ui_file || name != "client")
}

// ---------------------------------------------------------------------------
// Evidence from code and settings
// ---------------------------------------------------------------------------

/// What placed a file in its layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Source {
    Settings,
    Name,
    Code,
    Imports,
}

impl Source {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Source::Settings => "settings",
            Source::Name => "name",
            Source::Code => "code",
            Source::Imports => "imports",
        }
    }
}

/// What the code of each layer shows, for evidence lines.
pub(super) const LAYER_SIGNS: &[&str] = &[
    "renders components",
    "defines routes",
    "wraps requests",
    "declares services",
    "defines models or tables",
];

/// Per-repository settings file.
pub const SETTINGS_FILE: &str = ".codedna.toml";

/// Largest settings file read.
pub(super) const MAX_SETTINGS_BYTES: u64 = 64 * 1024;

/// Largest source file read for content evidence.
pub(super) const MAX_CODE_BYTES: u64 = 512 * 1024;

/// Middleware: functions that sit around a request.
pub(super) const MIDDLEWARE_SIGNS: &[&str] = &[
    "(req, res, next)",
    "NextFunction",
    "implements NestMiddleware",
    "implements CanActivate",
    "implements HttpInterceptor",
    "MiddlewareMixin",
    "BaseHTTPMiddleware",
    "func(next http.Handler) http.Handler",
    "middleware::from_fn",
    "HandlerInterceptor",
    "OncePerRequestFilter",
];

/// Data: model, table and repository definitions (not every query, so a
/// service that calls the database stays a service).
pub(super) const DATA_SIGNS: &[&str] = &[
    "@Entity(",
    "(models.Model)",
    "declarative_base(",
    "(DeclarativeBase)",
    "mapped_column(",
    "(SQLModel, table=True)",
    "new Schema(",
    "mongoose.Schema(",
    "mongoose.model(",
    "sequelize.define(",
    "#[derive(Queryable",
    "derive(sqlx::FromRow",
    "diesel::table!",
    "gorm.Model",
    "@Table(",
    "extends Model",
    "< ApplicationRecord",
    "< ActiveRecord::Base",
    "CREATE TABLE",
    "@Repository",
    "extends JpaRepository",
    "pgTable(",
    "sqliteTable(",
    "mysqlTable(",
    ": DbContext",
    "DbSet<",
];

/// Services: classes a framework injects.
pub(super) const SERVICE_SIGNS: &[&str] = &["@Service", "@Injectable(", "@Transactional"];

/// Controllers whose routes come from attributes or annotations on the class.
pub(super) const CONTROLLER_SIGNS: &[&str] = &[
    "[ApiController]",
    ": ControllerBase",
    "[HttpGet",
    "[HttpPost",
    "@RestController",
    "@Controller",
];

/// Files that describe data shapes rather than work: never placed in
/// Services by their imports alone.
pub(super) const SHAPE_NAMES: &[&str] = &[
    "serializer",
    "schema",
    "dto",
    "types",
    "type",
    "util",
    "helper",
    "const",
    "config",
    "settings",
    "renderer",
];

pub(super) fn is_shape(path: &Path) -> bool {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    SHAPE_NAMES.iter().any(|n| stem.contains(n))
}

/// Read a source file for content evidence, skipping very large ones.
pub(super) fn read_capped(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > MAX_CODE_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Layer index shown by a file's code, if any, with what matched and where.
pub(super) fn content_layer(path: &Path, text: &str) -> Option<(usize, CodeSign)> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(ext.as_str(), "vue" | "svelte" | "astro") {
        let sign = CodeSign {
            line: 0,
            sign: format!(".{ext} component"),
        };
        return Some((0, sign));
    }
    // Signs count only in code, not in strings, comments or fixtures.
    let original = text;
    let code = crate::code_text::code_only(&ext, text);
    let text = code.as_str();
    if matches!(ext.as_str(), "tsx" | "jsx") {
        let markup = [text.find("/>"), text.find("</")]
            .into_iter()
            .flatten()
            .min();
        if let Some(at) = markup {
            return Some((0, sign_at(text, at, "JSX markup")));
        }
    }
    let first = |signs: &[&'static str]| {
        signs
            .iter()
            .filter_map(|s| text.find(s).map(|at| (at, *s)))
            .min()
            .map(|(at, s)| sign_at(text, at, s))
    };
    let file = path.to_string_lossy().replace('\\', "/");
    if let Some(route) = crate::runbook::routes(&file, original).into_iter().next() {
        let sign = CodeSign {
            line: route.line,
            sign: sanitize(&format!("{} {}", route.method, route.path)),
        };
        return Some((1, sign));
    }
    if let Some(sign) = first(CONTROLLER_SIGNS) {
        return Some((1, sign));
    }
    if let Some(sign) = first(MIDDLEWARE_SIGNS) {
        return Some((2, sign));
    }
    if let Some(sign) = first(DATA_SIGNS) {
        return Some((4, sign));
    }
    first(SERVICE_SIGNS).map(|sign| (3, sign))
}

/// A sign found at byte `at` of `text`.
pub(super) fn sign_at(text: &str, at: usize, sign: &str) -> CodeSign {
    CodeSign {
        line: text[..at].bytes().filter(|b| *b == b'\n').count() + 1,
        sign: sign.to_string(),
    }
}

/// Layer folders set by hand in `.codedna.toml`:
///
/// ```toml
/// [layers]
/// routes = ["src/http/"]
/// data = ["src/store/", "src/tables.rs"]
/// ```
///
/// Keys are `ui`, `routes`, `middleware`, `services` and `data`; values are
/// path prefixes relative to the repository root. The longest match wins.
#[derive(Debug, Default)]
pub(super) struct Overrides {
    prefixes: Vec<(String, usize)>,
}

impl Overrides {
    fn load(root: &Path) -> Self {
        let path = root.join(SETTINGS_FILE);
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            return Self::default();
        };
        if !meta.is_file() || meta.len() > MAX_SETTINGS_BYTES {
            return Self::default();
        }
        std::fs::read_to_string(&path)
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    fn parse(text: &str) -> Self {
        let Ok(table) = text.parse::<toml::Table>() else {
            return Self::default();
        };
        let Some(layers) = table.get("layers").and_then(|v| v.as_table()) else {
            return Self::default();
        };
        let mut prefixes = Vec::new();
        for (key, value) in layers {
            let Some(index) = LAYERS
                .iter()
                .position(|(name, _)| name.eq_ignore_ascii_case(key))
            else {
                continue;
            };
            let Some(list) = value.as_array() else {
                continue;
            };
            for prefix in list.iter().filter_map(|v| v.as_str()) {
                let prefix = normalise(prefix);
                if !prefix.is_empty() {
                    prefixes.push((prefix, index));
                }
            }
        }
        prefixes.sort_by_key(|(p, _)| std::cmp::Reverse(p.len()));
        Self { prefixes }
    }

    /// Layer index and the `[layers]` entry that matched.
    fn layer(&self, relative: &Path) -> Option<(usize, String)> {
        if self.prefixes.is_empty() {
            return None;
        }
        let path = normalise(&relative.to_string_lossy());
        self.prefixes
            .iter()
            .find(|(prefix, _)| {
                path == *prefix.trim_end_matches('/')
                    || path.starts_with(prefix.as_str())
                        && (prefix.ends_with('/') || path[prefix.len()..].starts_with('/'))
            })
            .map(|(prefix, index)| {
                let key = LAYERS[*index].0.to_ascii_lowercase();
                (*index, sanitize(&format!("{key} = \"{prefix}\"")))
            })
    }
}

/// `./src\\x` → `src/x`.
pub(super) fn normalise(path: &str) -> String {
    let path = path.replace('\\', "/");
    let mut path = path.as_str();
    while let Some(rest) = path.strip_prefix("./") {
        path = rest;
    }
    path.trim_start_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_place_files_by_prefix() {
        let o = Overrides::parse(
            "[layers]\nroutes = [\"./src/http/\"]\nData = [\"src/store\", \"src/http/db/\"]\nnonsense = [\"x/\"]\n",
        );
        let at = |p: &str| o.layer(Path::new(p)).map(|(i, _)| LAYERS[i].0);
        assert_eq!(at("src/http/users.rs"), Some("Routes"));
        assert_eq!(at("src/http/db/pool.rs"), Some("Data"));
        assert_eq!(at("src/store/mod.rs"), Some("Data"));
        assert_eq!(at("src/store"), Some("Data"));
        assert_eq!(at("src/storefront/a.rs"), None);
        assert_eq!(at("x/a.rs"), None);
        assert!(Overrides::parse("not toml [[").prefixes.is_empty());
        assert!(Overrides::parse("[other]\na = 1").prefixes.is_empty());
    }
}
