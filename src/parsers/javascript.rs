//! JavaScript and TypeScript dependency references, read from a syntax tree.
//!
//! Files are parsed with [Oxc](https://oxc.rs). Only syntax counts: import
//! and export declarations, `require` and `import()` calls, TypeScript
//! `import x = require()` and `import("x")` types, and the runtime loaders
//! (`new Worker`, `new URL(.., import.meta.url)`, `spawn`/`fork`/`exec`).
//! Text that only looks like an import (in a string, a comment, a regular
//! expression or component markup) is never one.
//!
//! Vue and Svelte components are parsed for their `<script>` blocks, Astro
//! components for their frontmatter and `<script>` blocks. Spans and lines
//! always point into the original file.
//!
//! A `require` or `Worker` the file binds itself (a parameter, a local
//! function, an import from elsewhere) is not the real one, so what it loads
//! is only a possibility. A file the parser gives up on is scanned as text,
//! and everything that scan finds is a possibility too.

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, ArrayExpressionElement, BinaryOperator, BindingPattern, CallExpression,
    ExportAllDeclaration, ExportFromDeclaration, Expression, ImportDeclaration,
    ImportDeclarationSpecifier, ImportExpression, NewExpression, ObjectPropertyKind,
    TSImportEqualsDeclaration, TSImportType, TSModuleReference, VariableDeclarator,
};
use oxc_ast_visit::{walk, Visit};
use oxc_parser::{ParseOptions, Parser};
use oxc_semantic::{Scoping, SemanticBuilder, SymbolFlags, SymbolId};
use oxc_span::{GetSpan, SourceType, Span};

use serde::{Deserialize, Serialize};

use super::{ParseDiagnostic, ParseStatus};

/// The parser behind [`scan`] and the version of the scan itself. Cached
/// scans made under another value are parsed again: bump the scan number
/// whenever [`scan`] finds something different in the same text, and keep the
/// Oxc version in step with `Cargo.toml` (a test checks it).
pub const PARSER_VERSION: &str = "oxc 0.154.0; scan 1";

/// How a reference loads its target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RefKind {
    /// `import x from "y"`, `import {} from "y"`
    Import,
    /// `import type { T } from "y"`
    ImportType,
    /// `import "y"`
    SideEffect,
    /// `export { x } from "y"`, `export * from "y"`
    ExportFrom,
    /// `export type { T } from "y"`, `export type * from "y"`
    ExportTypeFrom,
    /// `type T = typeof import("y")`
    ImportTypeExpression,
    /// `/** @import { T } from "y" */` or `/** @type {import("y").T} */` in
    /// a JavaScript file, which TypeScript reads as a type import
    JsDocImport,
    /// `import x = require("y")`
    ImportEquals,
    /// `import type x = require("y")`
    ImportTypeEquals,
    /// `require("y")` with the global or a `createRequire` require
    Require,
    /// `require("y")` where `require` is a binding of the file's own
    RequireShadowed,
    /// `import("y")`
    DynamicImport,
    /// `` import(`./y/${x}`) `` or `` require(`./y/${x}`) ``
    TemplateImport,
    /// `import(name)`
    ComputedImport,
    /// `require("./y/" + name)`
    ComputedRequire,
    /// `new URL("./y.js", import.meta.url)`
    Url,
    /// `new Worker("./y.js")`
    Worker,
    /// `new Worker("./y.js")` where `Worker` is not a worker constructor
    WorkerShadowed,
    /// `spawn("node", ["./y.js"])`
    Process,
    /// `import.meta.glob("./pages/*.ts")`: the specifier is the pattern,
    /// or a JSON array of patterns when there are several.
    Glob,
    /// `import.meta.glob("./x/*.svg", { query: "?url" })`: the files are
    /// loaded as assets, not as modules.
    GlobAsset,
    /// `import.meta.glob(pattern)`: a pattern known only at runtime.
    ComputedGlob,
    /// Found by the text scan of a file that did not parse.
    TextScan,
}

impl RefKind {
    /// Whether this is a runtime load named by a file path (worker, process,
    /// URL), only kept when the path names a source file.
    pub fn is_runtime_load(self) -> bool {
        matches!(
            self,
            RefKind::Url | RefKind::Worker | RefKind::WorkerShadowed | RefKind::Process
        )
    }
}

/// One dependency reference in a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsRef {
    /// The specifier as the code means it: the string's value, a folded
    /// constant, a template's text without the backticks, or the source of
    /// a computed expression.
    pub specifier: String,
    pub kind: RefKind,
    /// Byte range of the specifier in the whole file, without quotes.
    pub span: [usize; 2],
    /// Line number, from 1.
    pub line: usize,
}

/// Everything read from one file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsScan {
    /// References in source order, runtime loads last.
    pub refs: Vec<JsRef>,
    pub status: ParseStatus,
    /// The first syntax errors, in source order.
    pub diagnostics: Vec<ParseDiagnostic>,
}

/// How many syntax errors to keep per file.
const MAX_DIAGNOSTICS: usize = 3;

/// Modules whose exports start a file in another process or thread.
pub const LOADER_MODULES: &[&str] = &[
    "child_process",
    "node:child_process",
    "worker_threads",
    "node:worker_threads",
    "execa",
    "cross-spawn",
];

/// Modules whose default export is itself a process loader.
const DEFAULT_LOADER_MODULES: &[&str] = &["execa", "cross-spawn"];

/// Calls that run a file in another process: `spawn("tsx", ["w.ts"])`.
const PROCESS_LOADERS: &[&str] = &[
    "spawn",
    "spawnSync",
    "fork",
    "exec",
    "execSync",
    "execFile",
    "execFileSync",
    "execa",
    "execaSync",
    "execaNode",
    "execaCommand",
    "execaCommandSync",
];

/// Process loaders that take one command line rather than an argument list.
const COMMAND_LINE_LOADERS: &[&str] = &["exec", "execSync", "execaCommand", "execaCommandSync"];

/// Constructors that run a file in a worker thread: `new Worker("./w.js")`.
const WORKER_CONSTRUCTORS: &[&str] = &["Worker", "SharedWorker"];

/// Words whose absence from a script means no `require`, worker, process or
/// URL load can be in it, so its scopes need not be analysed.
const SCOPE_TRIGGERS: &[&str] = &[
    "require",
    "Worker",
    "child_process",
    "worker_threads",
    "execa",
    "cross-spawn",
    "Bun",
    "URL",
];

/// A JS/TS source file named by its extension: `"./worker.ts"` or
/// `'jobs/sync.mjs'`, but not URLs, absolute paths or command lines.
pub fn names_source_file(literal: &str) -> bool {
    let is_source = Path::new(literal)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["js", "mjs", "cjs", "ts", "mts", "cts", "tsx", "jsx"].contains(&e));
    is_source
        && !literal.contains("://")
        && !literal.contains("${")
        && !literal.contains(char::is_whitespace)
        && !literal.starts_with('/')
}

/// Read the dependency references of a file with extension `ext`
/// (lower-case, without the dot).
pub fn scan(ext: &str, content: &str) -> JsScan {
    let lines = LineIndex::new(content);
    let mut scan = JsScan::default();
    let mut loads = Vec::new();
    for block in blocks(ext, content) {
        let text = &content[block.start..block.end];
        let found = catch_unwind(AssertUnwindSafe(|| parse_block(text, block.source_type)))
            .unwrap_or_else(|_| BlockScan {
                status: ParseStatus::Failed,
                refs: text_scan(text),
                diagnostics: vec![(0, "the parser stopped unexpectedly".to_string())],
            });
        scan.status = scan.status.worst(found.status);
        for (offset, message) in found.diagnostics {
            if scan.diagnostics.len() < MAX_DIAGNOSTICS {
                scan.diagnostics.push(ParseDiagnostic {
                    line: lines.line(block.start + offset),
                    message,
                });
            }
        }
        for (specifier, kind, [start, end]) in found.refs {
            let span = [block.start + start, block.start + end];
            let found = JsRef {
                specifier,
                kind,
                span,
                line: lines.line(span[0]),
            };
            if kind.is_runtime_load() {
                loads.push(found);
            } else {
                scan.refs.push(found);
            }
        }
    }
    scan.refs.extend(loads);
    scan
}

// ---------------------------------------------------------------------------
// Script blocks
// ---------------------------------------------------------------------------

/// A run of script inside a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Block {
    start: usize,
    end: usize,
    source_type: SourceType,
}

/// The scripts of a file: the whole file for JS/TS, `<script>` blocks for
/// Vue and Svelte, frontmatter and `<script>` blocks for Astro.
fn blocks(ext: &str, content: &str) -> Vec<Block> {
    let whole = |source_type| {
        vec![Block {
            start: 0,
            end: content.len(),
            source_type,
        }]
    };
    match ext {
        "js" | "jsx" => whole(SourceType::unambiguous().with_jsx(true)),
        "mjs" => whole(SourceType::mjs().with_jsx(true)),
        "cjs" => whole(SourceType::cjs()),
        "ts" | "mts" | "cts" | "tsx" => {
            whole(SourceType::from_extension(ext).unwrap_or(SourceType::ts()))
        }
        "vue" | "svelte" => script_blocks(content, 0, SourceType::unambiguous()),
        "astro" => {
            let mut found = Vec::new();
            let mut rest = 0;
            if let Some((start, end, after)) = astro_frontmatter(content) {
                found.push(Block {
                    start,
                    end,
                    source_type: SourceType::ts(),
                });
                rest = after;
            }
            found.extend(script_blocks(content, rest, SourceType::ts()));
            found
        }
        _ => Vec::new(),
    }
}

/// The body of an Astro file's leading `---` fence, and where it ends.
fn astro_frontmatter(content: &str) -> Option<(usize, usize, usize)> {
    let lead = content.len() - content.trim_start().len();
    let after_open = lead + content[lead..].strip_prefix("---")?.len();
    let after_open = content.len() - after_open;
    let body_start = after_open + content[after_open..].find('\n')? + 1;
    let mut at = body_start;
    while at <= content.len() {
        let line_end = content[at..].find('\n').map_or(content.len(), |i| at + i);
        if content[at..line_end].trim_end() == "---" {
            return Some((body_start, at, line_end));
        }
        if line_end == content.len() {
            break;
        }
        at = line_end + 1;
    }
    None
}

/// `<script>` blocks of a component from byte `from` on, skipping HTML
/// comments, external scripts and blocks of a non-script `type`.
fn script_blocks(content: &str, from: usize, default: SourceType) -> Vec<Block> {
    let lower = content.to_ascii_lowercase();
    let mut found = Vec::new();
    let mut at = from;
    while at < content.len() {
        let next_comment = lower[at..].find("<!--").map(|i| at + i);
        let Some(open) = lower[at..].find("<script").map(|i| at + i) else {
            break;
        };
        if let Some(comment) = next_comment.filter(|&c| c < open) {
            at = lower[comment..]
                .find("-->")
                .map_or(content.len(), |i| comment + i + 3);
            continue;
        }
        let name_end = open + "<script".len();
        // `<scripts>` or `<script-x>` is another tag.
        if !lower[name_end..].starts_with(|c: char| c.is_whitespace() || c == '>' || c == '/') {
            at = name_end;
            continue;
        }
        let Some(tag_end) = lower[name_end..].find('>').map(|i| name_end + i) else {
            break;
        };
        let attrs = &lower[name_end..tag_end];
        if attrs.trim_end().ends_with('/') {
            at = tag_end + 1;
            continue;
        }
        let body_start = tag_end + 1;
        let body_end = lower[body_start..]
            .find("</script")
            .map_or(content.len(), |i| body_start + i);
        if let Some(source_type) = script_type(attrs, default) {
            found.push(Block {
                start: body_start,
                end: body_end,
                source_type,
            });
        }
        at = body_end;
    }
    found
}

/// How to parse a `<script>` with these (lower-cased) attributes, or `None`
/// for one that holds no script to parse.
fn script_type(attrs: &str, default: SourceType) -> Option<SourceType> {
    if attribute(attrs, "src").is_some() {
        return None;
    }
    if let Some(kind) = attribute(attrs, "type") {
        let script = [
            "module",
            "text/javascript",
            "application/javascript",
            "text/typescript",
            "application/typescript",
            "text/babel",
            "text/jsx",
        ];
        if !kind.is_empty() && !script.contains(&kind) {
            return None;
        }
    }
    Some(match attribute(attrs, "lang") {
        Some("ts" | "typescript") => SourceType::ts(),
        Some("tsx") => SourceType::tsx(),
        Some("jsx") => SourceType::unambiguous().with_jsx(true),
        Some("js" | "javascript") => SourceType::unambiguous(),
        _ => default,
    })
}

/// The value of attribute `name` in `attrs`: `lang="ts"` gives `ts`, a bare
/// `src` gives `""`.
fn attribute<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let bytes = attrs.as_bytes();
    let mut search = 0;
    while let Some(i) = attrs[search..].find(name).map(|i| search + i) {
        search = i + name.len();
        let before_ok = i == 0 || bytes[i - 1].is_ascii_whitespace();
        let rest = &attrs[search..];
        let after = rest.trim_start();
        if !before_ok {
            continue;
        }
        if let Some(value) = after.strip_prefix('=') {
            let value = value.trim_start();
            return Some(match value.chars().next() {
                Some(q @ ('"' | '\'')) => {
                    let inner = &value[1..];
                    &inner[..inner.find(q).unwrap_or(inner.len())]
                }
                _ => value.split_whitespace().next().unwrap_or(""),
            });
        }
        if rest.is_empty() || rest.starts_with(char::is_whitespace) || rest.starts_with('/') {
            return Some("");
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// What one block held, with block-relative offsets.
struct BlockScan {
    status: ParseStatus,
    refs: Vec<(String, RefKind, [usize; 2])>,
    diagnostics: Vec<(usize, String)>,
}

/// Parse one block, retrying JavaScript that fails as TypeScript with JSX
/// (which also reads most Flow annotations), and falling back to a text
/// scan when no attempt gives a tree.
fn parse_block(text: &str, source_type: SourceType) -> BlockScan {
    // JSDoc type imports only mean something in JavaScript; TypeScript
    // ignores JSDoc types in its own files.
    let jsdoc = !source_type.is_typescript();
    let first = parse_with(text, source_type, jsdoc);
    let first = match first {
        Ok(found) if found.diagnostics.is_empty() => return found,
        other => other,
    };
    if !source_type.is_typescript() {
        let retry = SourceType::unambiguous()
            .with_typescript(true)
            .with_jsx(true);
        if let Ok(found) = parse_with(text, retry, jsdoc) {
            if found.diagnostics.is_empty() {
                return found;
            }
        }
    }
    match first {
        Ok(found) => found,
        Err(diagnostics) => BlockScan {
            status: ParseStatus::Failed,
            refs: text_scan(text),
            diagnostics,
        },
    }
}

/// Parse `text` as `source_type`: the references and status, or the
/// syntax errors when the parser could not build a tree.
fn parse_with(
    text: &str,
    source_type: SourceType,
    jsdoc: bool,
) -> Result<BlockScan, Vec<(usize, String)>> {
    let allocator = Allocator::default();
    let options = ParseOptions {
        allow_return_outside_function: true,
        ..ParseOptions::default()
    };
    let parsed = Parser::new(&allocator, text, source_type)
        .with_options(options)
        .parse();
    let mut diagnostics: Vec<(usize, String)> = parsed
        .diagnostics
        .errors()
        .map(|d| {
            let offset = d.labels.first().map_or(0, |l| l.offset() as usize);
            (offset.min(text.len()), d.message.to_string())
        })
        .collect();
    diagnostics.sort_by_key(|d| d.0);
    if parsed.fatal_error {
        if diagnostics.is_empty() {
            diagnostics.push((0, "the file could not be parsed".to_string()));
        }
        return Err(diagnostics);
    }

    let program = &parsed.program;
    let semantic;
    let scoping = if SCOPE_TRIGGERS.iter().any(|t| text.contains(t)) {
        semantic = SemanticBuilder::new().build(program).semantic;
        Some(semantic.scoping())
    } else {
        None
    };
    let mut collector = Collector {
        text,
        scoping,
        items: Vec::new(),
        bindings: HashMap::new(),
    };
    collector.visit_program(program);
    let mut refs = collector.finish();
    if jsdoc {
        for comment in program.comments.iter().filter(|c| c.is_jsdoc()) {
            let span = comment.content_span();
            refs.extend(jsdoc_imports(text, span.start as usize, span.end as usize));
        }
        refs.sort_by_key(|r| r.2[0]);
    }
    Ok(BlockScan {
        status: if diagnostics.is_empty() {
            ParseStatus::Parsed
        } else {
            ParseStatus::Partial
        },
        refs,
        diagnostics,
    })
}

/// Type imports in one JSDoc comment, `text[start..end]`: the module of
/// each `@import ... from "y"` tag and each `import("y")` type.
fn jsdoc_imports(text: &str, start: usize, end: usize) -> Vec<(String, RefKind, [usize; 2])> {
    let Some(comment) = text.get(start..end) else {
        return Vec::new();
    };
    // The quoted string at `at`, after optional whitespace: its value's span.
    let quoted = |mut at: usize| -> Option<[usize; 2]> {
        let bytes = comment.as_bytes();
        while bytes.get(at).is_some_and(|b| b.is_ascii_whitespace()) {
            at += 1;
        }
        let quote = *bytes.get(at).filter(|b| matches!(b, b'"' | b'\''))?;
        let len = comment[at + 1..].find([quote as char, '\n'])?;
        let value = [at + 1, at + 1 + len];
        (len > 0 && bytes.get(value[1]) == Some(&quote)).then_some(value)
    };
    let mut found = Vec::new();
    let mut search = 0;
    while let Some(at) = comment[search..].find("import").map(|i| search + i) {
        search = at + "import".len();
        let before = comment[..at].chars().next_back();
        let value = if before == Some('@') {
            // `@import { T } from "y"`: the string after the tag's `from`.
            let tag_end = comment[search..]
                .find('@')
                .map_or(comment.len(), |i| search + i);
            comment[search..tag_end]
                .find(" from")
                .and_then(|i| quoted(search + i + " from".len()))
        } else if before.is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '$'))
            && comment[search..].trim_start().starts_with('(')
        {
            let open = search + comment[search..].find('(').unwrap_or(0);
            quoted(open + 1).filter(|v| comment[v[1] + 1..].trim_start().starts_with(')'))
        } else {
            None
        };
        if let Some([a, b]) = value {
            found.push((
                comment[a..b].to_string(),
                RefKind::JsDocImport,
                [start + a, start + b],
            ));
        }
    }
    found
}

/// The fallback for a block that did not parse: quoted strings right after
/// `from`, `import`, `import(` or `require(`. Possibilities only.
fn text_scan(text: &str) -> Vec<(String, RefKind, [usize; 2])> {
    let bytes = text.as_bytes();
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$';
    let mut found = Vec::new();
    for keyword in ["from", "import", "require"] {
        let mut search = 0;
        while let Some(at) = text[search..].find(keyword).map(|i| search + i) {
            search = at + keyword.len();
            let bounded = (at == 0 || !is_ident(bytes[at - 1]))
                && bytes.get(search).is_none_or(|&b| !is_ident(b));
            if !bounded {
                continue;
            }
            let mut i = search;
            while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
                i += 1;
            }
            if keyword != "from" && bytes.get(i) == Some(&b'(') {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
            } else if keyword == "require" {
                continue;
            }
            let Some(&quote) = bytes.get(i).filter(|b| matches!(b, b'"' | b'\'')) else {
                continue;
            };
            let start = i + 1;
            let Some(len) = text[start..].find([quote as char, '\n']) else {
                continue;
            };
            let end = start + len;
            if bytes.get(end) != Some(&quote) || len == 0 {
                continue;
            }
            found.push((
                text[start..end].to_string(),
                RefKind::TextScan,
                [start, end],
            ));
        }
    }
    found.sort_by_key(|f| f.2);
    found.dedup_by(|a, b| a.2 == b.2);
    found
}

// ---------------------------------------------------------------------------
// The syntax tree walk
// ---------------------------------------------------------------------------

/// What a binding of the file holds, when it matters for loaders.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Binding {
    /// `import { spawn as run } from "child_process"`, or a destructured
    /// `require("child_process")`.
    Named { module: String, name: String },
    /// `import * as cp from ..`, `import cp from ..`, `const cp = require(..)`
    Module { module: String },
    /// `const require = createRequire(import.meta.url)`
    CreateRequire,
}

/// A name a call or `new` refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Name {
    name: String,
    /// The file's own binding it refers to; `None` for a global.
    symbol: Option<SymbolId>,
}

/// What a call or `new` calls.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Callee {
    Name(Name),
    /// `cp.spawn`, `Bun.spawn`
    Member {
        object: Name,
        property: String,
    },
    /// `require("child_process").spawn`
    RequiredMember {
        module: String,
        property: String,
    },
}

/// A string argument: its value and block-relative span without quotes.
type Literal = (String, [usize; 2]);

/// A reference found during the walk, some classified only once every
/// binding is known (functions and imports are hoisted).
enum Item {
    Ready(String, RefKind, [usize; 2]),
    Require {
        name: Name,
        specifier: Specifier,
    },
    New {
        callee: Callee,
        literal: Literal,
    },
    Call {
        callee: Callee,
        literals: Vec<Literal>,
    },
}

/// What a module specifier expression is.
enum Specifier {
    /// A string, a template with no `${}` or a sum of those.
    Constant(Literal),
    /// A template with `${}`: its text without backticks.
    Template(Literal),
    /// Anything else: its source (first line, shortened).
    Computed(Literal),
}

struct Collector<'s> {
    text: &'s str,
    scoping: Option<&'s Scoping>,
    items: Vec<Item>,
    bindings: HashMap<SymbolId, Binding>,
}

/// The span of a quoted literal without its quotes.
fn inner(span: Span) -> [usize; 2] {
    let (start, end) = (span.start as usize, span.end as usize);
    if end >= start + 2 {
        [start + 1, end - 1]
    } else {
        [start, end]
    }
}

fn whole(span: Span) -> [usize; 2] {
    [span.start as usize, span.end as usize]
}

/// A relative glob pattern under Vite's `base` option:
/// `with_base("./widgets", "!./*.ts")` is `"!./widgets/*.ts"`. Root and
/// alias patterns ignore `base`.
fn with_base(base: &str, pattern: &str) -> String {
    let (negated, rest) = match pattern.strip_prefix('!') {
        Some(rest) => ("!", rest),
        None => ("", pattern),
    };
    if !(rest.starts_with("./") || rest.starts_with("../")) {
        return pattern.to_string();
    }
    let base = base.trim_end_matches('/');
    let rest = rest.strip_prefix("./").unwrap_or(rest);
    let base = if base.starts_with('.') || base.starts_with('/') {
        base.to_string()
    } else {
        format!("./{base}")
    };
    format!("{negated}{base}/{rest}")
}

/// The constant string `expr` always is, folding `+` over strings and
/// templates without `${}`.
fn constant(expr: &Expression, depth: usize) -> Option<String> {
    if depth > 8 {
        return None;
    }
    match expr.get_inner_expression() {
        Expression::StringLiteral(s) => Some(s.value.as_str().to_string()),
        Expression::TemplateLiteral(t) if t.is_no_substitution_template() => t
            .quasis
            .first()
            .and_then(|q| q.value.cooked.as_ref())
            .map(|c| c.as_str().to_string()),
        Expression::BinaryExpression(b) if b.operator == BinaryOperator::Addition => {
            Some(constant(&b.left, depth + 1)? + &constant(&b.right, depth + 1)?)
        }
        _ => None,
    }
}

impl<'s> Collector<'s> {
    fn source(&self, span: [usize; 2]) -> &'s str {
        self.text.get(span[0]..span[1]).unwrap_or("")
    }

    /// Classify a module specifier expression.
    fn specifier(&self, expr: &Expression) -> Specifier {
        let expr = expr.get_inner_expression();
        if let Some(value) = constant(expr, 0) {
            let span = match expr {
                Expression::StringLiteral(s) => inner(s.span),
                Expression::TemplateLiteral(t) => inner(t.span),
                other => whole(other.span()),
            };
            return Specifier::Constant((value, span));
        }
        if let Expression::TemplateLiteral(t) = expr {
            let span = inner(t.span);
            return Specifier::Template((self.source(span).to_string(), span));
        }
        let span = whole(expr.span());
        let source = self.source(span);
        let first_line = source.lines().next().unwrap_or("").trim_end();
        let shortened = match first_line.char_indices().nth(80) {
            Some((cut, _)) => &first_line[..cut],
            None => first_line,
        };
        Specifier::Computed((shortened.to_string(), span))
    }

    /// The file's binding `name` refers to, if any. Ambient declarations
    /// (`declare const require`) describe the global, so they count as one.
    fn name(&self, name: &str, reference: Option<oxc_semantic::ReferenceId>) -> Name {
        let symbol = self.scoping.zip(reference).and_then(|(scoping, id)| {
            let symbol = scoping.get_reference(id).symbol_id()?;
            (!scoping.symbol_flags(symbol).contains(SymbolFlags::Ambient)).then_some(symbol)
        });
        Name {
            name: name.to_string(),
            symbol,
        }
    }

    fn callee(&self, expr: &Expression) -> Option<Callee> {
        match expr.get_inner_expression() {
            Expression::Identifier(id) => Some(Callee::Name(
                self.name(id.name.as_str(), id.reference_id.get()),
            )),
            Expression::StaticMemberExpression(member) => {
                let property = member.property.name.as_str().to_string();
                match member.object.get_inner_expression() {
                    Expression::Identifier(id) => Some(Callee::Member {
                        object: self.name(id.name.as_str(), id.reference_id.get()),
                        property,
                    }),
                    Expression::CallExpression(call) => {
                        let module = self.required_module(call)?;
                        Some(Callee::RequiredMember { module, property })
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Vite's `import.meta.glob(patterns, options)` (and the older
    /// `globEager`/`globEagerDefault`). Literal patterns, with a literal
    /// `base` folded into the relative ones, are a [`RefKind::Glob`];
    /// anything Vite would only know at runtime is a [`RefKind::ComputedGlob`].
    fn import_meta_glob(&self, call: &CallExpression) -> Option<Item> {
        let Expression::StaticMemberExpression(member) = call.callee.get_inner_expression() else {
            return None;
        };
        if !matches!(member.object, Expression::ImportMeta(_))
            || !matches!(
                member.property.name.as_str(),
                "glob" | "globEager" | "globEagerDefault"
            )
        {
            return None;
        }
        let first = call.arguments.first()?;
        let first_span = whole(first.span());
        let all_span = [
            first_span[0],
            call.arguments
                .last()
                .map_or(first_span[1], |a| a.span().end as usize),
        ];
        let computed = |item: Specifier| match item {
            Specifier::Constant((text, span))
            | Specifier::Template((text, span))
            | Specifier::Computed((text, span)) => {
                Some(Item::Ready(text, RefKind::ComputedGlob, span))
            }
        };
        let Some(expr) = first.as_expression() else {
            return computed(Specifier::Computed((
                self.source(first_span).to_string(),
                first_span,
            )));
        };
        let mut patterns = Vec::new();
        match expr.get_inner_expression() {
            Expression::ArrayExpression(array) => {
                for element in &array.elements {
                    match element.as_expression().and_then(|e| constant(e, 0)) {
                        Some(value) => patterns.push(value),
                        None => return computed(self.specifier(expr)),
                    }
                }
            }
            other => match constant(other, 0) {
                Some(value) => patterns.push(value),
                None => return computed(self.specifier(expr)),
            },
        }
        if patterns.is_empty() {
            return None;
        }
        let whole_call = || {
            computed(Specifier::Computed((
                self.source(all_span).to_string(),
                all_span,
            )))
        };
        let mut asset = false;
        if let Some(options) = call.arguments.get(1) {
            let Some(Expression::ObjectExpression(object)) = options
                .as_expression()
                .map(Expression::get_inner_expression)
            else {
                return whole_call();
            };
            for property in &object.properties {
                let ObjectPropertyKind::ObjectProperty(property) = property else {
                    return whole_call();
                };
                let Some(key) = property.key.static_name() else {
                    return whole_call();
                };
                match key.as_ref() {
                    "as" | "query" => asset = true,
                    "base" => {
                        let Some(base) = constant(&property.value, 0) else {
                            return whole_call();
                        };
                        for pattern in &mut patterns {
                            *pattern = with_base(&base, pattern);
                        }
                    }
                    _ => {}
                }
            }
        }
        let specifier = if patterns.len() == 1 {
            patterns.pop()?
        } else {
            serde_json::to_string(&patterns).ok()?
        };
        let span = match expr.get_inner_expression() {
            Expression::StringLiteral(s) => inner(s.span),
            other => whole(other.span()),
        };
        let kind = if asset {
            RefKind::GlobAsset
        } else {
            RefKind::Glob
        };
        Some(Item::Ready(specifier, kind, span))
    }

    /// `x` in `require("x")`, when the call is one.
    fn required_module(&self, call: &CallExpression) -> Option<String> {
        let Expression::Identifier(id) = call.callee.get_inner_expression() else {
            return None;
        };
        if id.name.as_str() != "require" {
            return None;
        }
        constant(call.arguments.first()?.as_expression()?, 0)
    }

    /// String literals among `args`, also inside array literals.
    fn literals(args: &[Argument]) -> Vec<Literal> {
        let mut found = Vec::new();
        let mut push = |expr: &Expression| {
            if let Some(value) = constant(expr, 0) {
                let span = match expr.get_inner_expression() {
                    Expression::StringLiteral(s) => inner(s.span),
                    Expression::TemplateLiteral(t) => inner(t.span),
                    other => whole(other.span()),
                };
                found.push((value, span));
            }
        };
        for arg in args {
            match arg.as_expression().map(Expression::get_inner_expression) {
                Some(Expression::ArrayExpression(array)) => {
                    for element in &array.elements {
                        if let Some(expr) = element.as_expression() {
                            push(expr);
                        } else if let ArrayExpressionElement::SpreadElement(_) = element {
                        }
                    }
                }
                Some(expr) => push(expr),
                None => {}
            }
        }
        found
    }

    fn bind(&mut self, symbol: Option<SymbolId>, binding: Binding) {
        if let Some(symbol) = symbol {
            self.bindings.insert(symbol, binding);
        }
    }

    /// Classify the deferred items, now that every binding is known.
    fn finish(self) -> Vec<(String, RefKind, [usize; 2])> {
        let mut found = Vec::new();
        for item in &self.items {
            match item {
                Item::Ready(spec, kind, span) => found.push((spec.clone(), *kind, *span)),
                Item::Require { name, specifier } => {
                    let real = match name.symbol {
                        None => true,
                        Some(symbol) => self.bindings.get(&symbol) == Some(&Binding::CreateRequire),
                    };
                    found.push(match specifier {
                        Specifier::Constant((value, span)) => {
                            let kind = if real {
                                RefKind::Require
                            } else {
                                RefKind::RequireShadowed
                            };
                            (value.clone(), kind, *span)
                        }
                        Specifier::Template((text, span)) => {
                            (text.clone(), RefKind::TemplateImport, *span)
                        }
                        Specifier::Computed((text, span)) => {
                            (text.clone(), RefKind::ComputedRequire, *span)
                        }
                    });
                }
                Item::New { callee, literal } => {
                    if let Some(kind) = self.worker(callee) {
                        if names_source_file(&literal.0) {
                            found.push((literal.0.clone(), kind, literal.1));
                        }
                    }
                }
                Item::Call { callee, literals } => {
                    let Some(command_line) = self.process_loader(callee) else {
                        continue;
                    };
                    for (value, span) in literals {
                        if !command_line {
                            if names_source_file(value) {
                                found.push((value.clone(), RefKind::Process, *span));
                            }
                            continue;
                        }
                        // Word by word, when the source text is the value.
                        let raw = self.source(*span);
                        if raw != value {
                            continue;
                        }
                        let mut offset = 0;
                        for word in raw.split_inclusive(char::is_whitespace) {
                            let trimmed = word.trim_end();
                            if names_source_file(trimmed) {
                                let start = span[0] + offset;
                                found.push((
                                    trimmed.to_string(),
                                    RefKind::Process,
                                    [start, start + trimmed.len()],
                                ));
                            }
                            offset += word.len();
                        }
                    }
                }
            }
        }
        found
    }

    /// What `new callee(..)` is: a worker, a look-alike, or nothing.
    fn worker(&self, callee: &Callee) -> Option<RefKind> {
        match callee {
            Callee::Name(name) => match name.symbol {
                None => WORKER_CONSTRUCTORS
                    .contains(&name.name.as_str())
                    .then_some(RefKind::Worker),
                Some(symbol) => match self.bindings.get(&symbol) {
                    Some(Binding::Named {
                        module,
                        name: imported,
                    }) if LOADER_MODULES.contains(&module.as_str())
                        && WORKER_CONSTRUCTORS.contains(&imported.as_str()) =>
                    {
                        Some(RefKind::Worker)
                    }
                    _ => WORKER_CONSTRUCTORS
                        .contains(&name.name.as_str())
                        .then_some(RefKind::WorkerShadowed),
                },
            },
            Callee::Member { object, property } => {
                let loader = object.symbol.is_some_and(|s| {
                    matches!(self.bindings.get(&s), Some(Binding::Module { module })
                        if LOADER_MODULES.contains(&module.as_str()))
                });
                (loader && WORKER_CONSTRUCTORS.contains(&property.as_str()))
                    .then_some(RefKind::Worker)
            }
            Callee::RequiredMember { module, property } => (LOADER_MODULES
                .contains(&module.as_str())
                && WORKER_CONSTRUCTORS.contains(&property.as_str()))
            .then_some(RefKind::Worker),
        }
    }

    /// Whether `callee` runs a file in another process: `Some(true)` for a
    /// loader that takes a command line, `Some(false)` for an argument list.
    fn process_loader(&self, callee: &Callee) -> Option<bool> {
        let loader = |module: &str, name: &str| {
            (LOADER_MODULES.contains(&module) && PROCESS_LOADERS.contains(&name))
                .then(|| COMMAND_LINE_LOADERS.contains(&name))
        };
        match callee {
            Callee::Name(name) => match self.bindings.get(&name.symbol?)? {
                Binding::Named { module, name } => loader(module, name),
                Binding::Module { module } => DEFAULT_LOADER_MODULES
                    .contains(&module.as_str())
                    .then_some(false),
                Binding::CreateRequire => None,
            },
            Callee::Member { object, property } => match object.symbol {
                None if object.name == "Bun" => {
                    matches!(property.as_str(), "spawn" | "spawnSync").then_some(false)
                }
                None => None,
                Some(symbol) => match self.bindings.get(&symbol)? {
                    Binding::Module { module } => loader(module, property),
                    _ => None,
                },
            },
            Callee::RequiredMember { module, property } => loader(module, property),
        }
    }
}

impl<'a> Visit<'a> for Collector<'_> {
    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
        let module = it.source.value.as_str();
        let kind = if it.import_kind.is_type() {
            RefKind::ImportType
        } else if it.specifiers.is_none() {
            RefKind::SideEffect
        } else {
            RefKind::Import
        };
        self.items
            .push(Item::Ready(module.to_string(), kind, inner(it.source.span)));
        if LOADER_MODULES.contains(&module) {
            for specifier in it.specifiers.iter().flatten() {
                let (local, binding) = match specifier {
                    ImportDeclarationSpecifier::ImportSpecifier(s) => (
                        &s.local,
                        Binding::Named {
                            module: module.to_string(),
                            name: s.imported.name().as_str().to_string(),
                        },
                    ),
                    ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => (
                        &s.local,
                        Binding::Module {
                            module: module.to_string(),
                        },
                    ),
                    ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => (
                        &s.local,
                        Binding::Module {
                            module: module.to_string(),
                        },
                    ),
                };
                self.bind(local.symbol_id.get(), binding);
            }
        }
        walk::walk_import_declaration(self, it);
    }

    fn visit_export_from_declaration(&mut self, it: &ExportFromDeclaration<'a>) {
        let kind = if it.export_kind.is_type() {
            RefKind::ExportTypeFrom
        } else {
            RefKind::ExportFrom
        };
        self.items.push(Item::Ready(
            it.source.value.as_str().to_string(),
            kind,
            inner(it.source.span),
        ));
        walk::walk_export_from_declaration(self, it);
    }

    fn visit_export_all_declaration(&mut self, it: &ExportAllDeclaration<'a>) {
        let kind = if it.export_kind.is_type() {
            RefKind::ExportTypeFrom
        } else {
            RefKind::ExportFrom
        };
        self.items.push(Item::Ready(
            it.source.value.as_str().to_string(),
            kind,
            inner(it.source.span),
        ));
        walk::walk_export_all_declaration(self, it);
    }

    fn visit_ts_import_equals_declaration(&mut self, it: &TSImportEqualsDeclaration<'a>) {
        if let TSModuleReference::ExternalModuleReference(reference) = &it.module_reference {
            let module = reference.expression.value.as_str();
            let kind = if it.import_kind.is_type() {
                RefKind::ImportTypeEquals
            } else {
                RefKind::ImportEquals
            };
            self.items.push(Item::Ready(
                module.to_string(),
                kind,
                inner(reference.expression.span),
            ));
            if LOADER_MODULES.contains(&module) {
                self.bind(
                    it.id.symbol_id.get(),
                    Binding::Module {
                        module: module.to_string(),
                    },
                );
            }
        }
        walk::walk_ts_import_equals_declaration(self, it);
    }

    fn visit_ts_import_type(&mut self, it: &TSImportType<'a>) {
        self.items.push(Item::Ready(
            it.source.value.as_str().to_string(),
            RefKind::ImportTypeExpression,
            inner(it.source.span),
        ));
        walk::walk_ts_import_type(self, it);
    }

    fn visit_import_expression(&mut self, it: &ImportExpression<'a>) {
        self.items.push(match self.specifier(&it.source) {
            Specifier::Constant((value, span)) => Item::Ready(value, RefKind::DynamicImport, span),
            Specifier::Template((text, span)) => Item::Ready(text, RefKind::TemplateImport, span),
            Specifier::Computed((text, span)) => Item::Ready(text, RefKind::ComputedImport, span),
        });
        walk::walk_import_expression(self, it);
    }

    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        if let Some(Expression::CallExpression(call)) =
            it.init.as_ref().map(Expression::get_inner_expression)
        {
            let creates_require = match call.callee.get_inner_expression() {
                Expression::Identifier(id) => id.name.as_str() == "createRequire",
                Expression::StaticMemberExpression(m) => {
                    m.property.name.as_str() == "createRequire"
                }
                _ => false,
            };
            if creates_require {
                if let BindingPattern::BindingIdentifier(id) = &it.id {
                    self.bind(id.symbol_id.get(), Binding::CreateRequire);
                }
            } else if let Some(module) = self
                .required_module(call)
                .filter(|m| LOADER_MODULES.contains(&m.as_str()))
            {
                match &it.id {
                    BindingPattern::BindingIdentifier(id) => {
                        self.bind(id.symbol_id.get(), Binding::Module { module });
                    }
                    BindingPattern::ObjectPattern(pattern) => {
                        for property in &pattern.properties {
                            let (Some(key), BindingPattern::BindingIdentifier(id)) =
                                (property.key.static_name(), &property.value)
                            else {
                                continue;
                            };
                            self.bind(
                                id.symbol_id.get(),
                                Binding::Named {
                                    module: module.clone(),
                                    name: key.to_string(),
                                },
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
        walk::walk_variable_declarator(self, it);
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if let Some(item) = self.import_meta_glob(it) {
            self.items.push(item);
        }
        if let Expression::Identifier(id) = it.callee.get_inner_expression() {
            if id.name.as_str() == "require" {
                if let Some(arg) = it.arguments.first().and_then(Argument::as_expression) {
                    let item = Item::Require {
                        name: self.name("require", id.reference_id.get()),
                        specifier: self.specifier(arg),
                    };
                    self.items.push(item);
                }
            }
        }
        if self.scoping.is_some() {
            let literals: Vec<Literal> = Self::literals(&it.arguments)
                .into_iter()
                .filter(|(value, _)| {
                    value.split_whitespace().any(names_source_file) || names_source_file(value)
                })
                .collect();
            if !literals.is_empty() {
                if let Some(callee) = self.callee(&it.callee) {
                    self.items.push(Item::Call { callee, literals });
                }
            }
        }
        walk::walk_call_expression(self, it);
    }

    fn visit_new_expression(&mut self, it: &NewExpression<'a>) {
        if self.scoping.is_some() {
            let first = it.arguments.first().and_then(Argument::as_expression);
            let literal = first.and_then(|expr| {
                let value = constant(expr, 0)?;
                let span = match expr.get_inner_expression() {
                    Expression::StringLiteral(s) => inner(s.span),
                    Expression::TemplateLiteral(t) => inner(t.span),
                    other => whole(other.span()),
                };
                Some((value, span))
            });
            if let (Some(literal), Some(callee)) = (literal, self.callee(&it.callee)) {
                let is_url =
                    matches!(&callee, Callee::Name(n) if n.name == "URL" && n.symbol.is_none());
                if is_url {
                    let base_is_module = it
                        .arguments
                        .get(1)
                        .and_then(Argument::as_expression)
                        .is_some_and(|base| match base.get_inner_expression() {
                            Expression::StaticMemberExpression(m) => {
                                m.property.name.as_str() == "url"
                                    && matches!(m.object, Expression::ImportMeta(_))
                            }
                            _ => false,
                        });
                    if base_is_module && names_source_file(&literal.0) {
                        self.items
                            .push(Item::Ready(literal.0, RefKind::Url, literal.1));
                    }
                } else {
                    self.items.push(Item::New { callee, literal });
                }
            }
        }
        walk::walk_new_expression(self, it);
    }
}

// ---------------------------------------------------------------------------
// Lines
// ---------------------------------------------------------------------------

/// Byte offset to line number.
struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    fn new(content: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(content.match_indices('\n').map(|(i, _)| i + 1));
        LineIndex { starts }
    }

    fn line(&self, offset: usize) -> usize {
        self.starts.partition_point(|&s| s <= offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_version_names_the_oxc_in_use() {
        let manifest = include_str!("../../Cargo.toml");
        let oxc = PARSER_VERSION
            .strip_prefix("oxc ")
            .and_then(|v| v.split(';').next())
            .unwrap();
        assert!(
            manifest.contains(&format!("oxc_parser = {{ version = \"={oxc}\"")),
            "PARSER_VERSION names oxc {oxc}; update it with Cargo.toml"
        );
    }

    fn found(ext: &str, content: &str) -> Vec<(String, RefKind)> {
        scan(ext, content)
            .refs
            .into_iter()
            .map(|r| (r.specifier, r.kind))
            .collect()
    }

    #[test]
    fn import_meta_glob_kinds() {
        assert_eq!(
            found("ts", "import.meta.glob('./a/*.ts', { eager: true });"),
            [("./a/*.ts".to_string(), RefKind::Glob)]
        );
        assert_eq!(
            found("ts", "import.meta.glob(['./a/*.ts', '!./a/_*.ts']);"),
            [(r#"["./a/*.ts","!./a/_*.ts"]"#.to_string(), RefKind::Glob)]
        );
        assert_eq!(
            found("ts", "import.meta.glob('./a/*.svg', { query: '?url' });"),
            [("./a/*.svg".to_string(), RefKind::GlobAsset)]
        );
        assert_eq!(
            found("ts", "import.meta.glob('./*.ts', { ...o });"),
            [("'./*.ts', { ...o }".to_string(), RefKind::ComputedGlob)]
        );
        assert_eq!(
            found("ts", "import.meta.glob(['./a/*.ts', p]);"),
            [("['./a/*.ts', p]".to_string(), RefKind::ComputedGlob)]
        );
        assert!(found("ts", "meta.glob('./a/*.ts'); import.meta.url;").is_empty());
    }

    #[test]
    fn glob_base_moves_relative_patterns_only() {
        assert_eq!(with_base("./widgets", "./*.ts"), "./widgets/*.ts");
        assert_eq!(with_base("widgets/", "!./_*.ts"), "!./widgets/_*.ts");
        assert_eq!(with_base("../up", "../*.ts"), "../up/../*.ts");
        assert_eq!(with_base("./w", "/src/*.ts"), "/src/*.ts");
    }

    #[test]
    fn spans_and_lines_point_into_the_file() {
        let content = "// x\nimport a from './a';\n";
        let scan = scan("js", content);
        let r = &scan.refs[0];
        assert_eq!(&content[r.span[0]..r.span[1]], "./a");
        assert_eq!(r.line, 2);
        assert_eq!(scan.status, ParseStatus::Parsed);
    }

    #[test]
    fn jsx_and_flow_like_javascript_parse() {
        assert_eq!(
            found("js", "import a from './a';\nconst x = <div>{a}</div>;\n"),
            [("./a".to_string(), RefKind::Import)]
        );
        let flow = "import type { T } from './t';\nfunction f(x: number): string { return ''; }\n";
        let s = scan("js", flow);
        assert_eq!(s.status, ParseStatus::Parsed);
        assert_eq!(s.refs[0].kind, RefKind::ImportType);
    }

    #[test]
    fn script_attributes() {
        assert_eq!(attribute(" setup lang=\"ts\"", "lang"), Some("ts"));
        assert_eq!(attribute(" src=x.js", "src"), Some("x.js"));
        assert_eq!(attribute(" async", "src"), None);
        assert_eq!(attribute(" xlang=\"ts\"", "lang"), None);
        assert!(script_type(" type=\"application/ld+json\"", SourceType::mjs()).is_none());
        assert!(script_type(" type=\"module\"", SourceType::mjs()).is_some());
    }

    #[test]
    fn astro_frontmatter_bounds() {
        let content = "---\nimport a from './a';\n---\n<p/>\n";
        let (start, end, _) = astro_frontmatter(content).unwrap();
        assert_eq!(&content[start..end], "import a from './a';\n");
        assert!(astro_frontmatter("<p/>\n---\n").is_none());
    }

    #[test]
    fn line_index() {
        let index = LineIndex::new("a\nb\n\nc");
        assert_eq!(index.line(0), 1);
        assert_eq!(index.line(2), 2);
        assert_eq!(index.line(4), 3);
        assert_eq!(index.line(5), 4);
    }

    #[test]
    fn text_scan_finds_quoted_specifiers() {
        let found: Vec<String> =
            text_scan("import a from './a'\nrequire('./b')\nimport('./c')\nfrom\n")
                .into_iter()
                .map(|f| f.0)
                .collect();
        assert_eq!(found, ["./a", "./b", "./c"]);
    }

    #[test]
    fn statuses_follow_how_far_the_parser_got() {
        let partial = scan("ts", "import { a } from \"./a\";\nconst x = a ?? 1 || 2;\n");
        assert_eq!(partial.status, ParseStatus::Partial);
        assert_eq!(partial.diagnostics[0].line, 2);
        assert_eq!(partial.refs[0].kind, RefKind::Import);

        let failed = scan("js", "require(\"./a\");\n<<<<<<< HEAD\n");
        assert_eq!(failed.status, ParseStatus::Failed);
        assert_eq!(
            found("js", "require(\"./a\");\n<<<<<<< HEAD\n")[0].1,
            RefKind::TextScan
        );
    }

    #[test]
    fn an_ambient_require_is_the_global_one() {
        assert_eq!(
            found("ts", "declare const require: any;\nrequire(\"./x\");\n"),
            [("./x".to_string(), RefKind::Require)]
        );
        assert_eq!(
            found("js", "function f(require) { require(\"./x\"); }\n"),
            [("./x".to_string(), RefKind::RequireShadowed)]
        );
    }
}
