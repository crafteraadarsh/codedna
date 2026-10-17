//! The checks, one module per rule family. A rule only extracts what it
//! sees in one file and states how strong that evidence is; the file's
//! context and the shared confidence model live in the parent module.

pub(crate) mod binary;
pub(crate) mod install;
pub(crate) mod minified;
pub(crate) mod obfuscation;
pub(crate) mod secrets;
pub(crate) mod typosquat;
pub(crate) mod unicode;

use super::Finding;

/// Extensions treated as program source.
pub(crate) const CODE_EXTENSIONS: &[&str] = &[
    "js", "mjs", "cjs", "jsx", "ts", "mts", "cts", "tsx", "py", "rb", "php", "sh", "bash", "zsh",
    "ps1", "go", "rs", "pl", "lua", "java", "kt", "cs", "c", "cc", "cpp", "h", "hpp", "swift",
    "vue", "svelte",
];

/// Folders that hold copies of other projects' code.
const VENDOR_DIRS: &[&str] = &[
    "vendor",
    "vendors",
    "third_party",
    "third-party",
    "thirdparty",
    "bower_components",
];

/// Check one file. `file` is its sanitized relative path.
pub(crate) fn audit_file(file: &str, bytes: &[u8]) -> Vec<Finding> {
    let mut out = Vec::new();
    let name = file.rsplit('/').next().unwrap_or(file);
    let lower_name = name.to_ascii_lowercase();
    let ext = lower_name
        .rsplit_once('.')
        .map(|(_, e)| e.to_string())
        .unwrap_or_default();
    let code = CODE_EXTENSIONS.contains(&ext.as_str());

    if let Some(f) = binary::check(file, bytes) {
        out.push(f);
        return out;
    }
    if bytes[..bytes.len().min(8_192)].contains(&0) {
        return out;
    }
    let text = String::from_utf8_lossy(bytes);

    let minified = lower_name.contains(".min.");
    // Minifiers produce `Function(`, `atob(`, hex escapes and long
    // strings, so these patterns say nothing about third-party bundles.
    let third_party = minified
        || lower_name.contains("-min.")
        || text.starts_with("/*!")
        || file
            .to_ascii_lowercase()
            .split('/')
            .any(|dir| VENDOR_DIRS.contains(&dir));
    let mut blob_reported = false;
    let mut long_line_reported = false;
    let lines: Vec<&str> = text.lines().collect();
    // A long line near the top marks a generated bundle.
    let bundle = lines.iter().take(5).any(|l| l.len() >= minified::LONG_LINE);

    for (index, &line) in lines.iter().enumerate() {
        let n = Some(index + 1);

        out.extend(unicode::check_line(file, index, line, code));

        let secrets = secrets::check_line(
            file,
            n,
            line,
            lines.get(index + 1).copied(),
            &lines[index.saturating_sub(3)..index],
        );
        let secret_on_line = !secrets.is_empty();
        out.extend(secrets);

        if !code || third_party {
            if code && !long_line_reported && !minified && line.len() >= minified::LONG_LINE {
                long_line_reported = true;
                out.push(minified::long_line(file, n, line));
            }
            continue;
        }

        let mut blob_on_line = false;
        out.extend(obfuscation::check_line(
            file,
            n,
            line,
            bundle,
            secret_on_line,
            &mut blob_reported,
            &mut blob_on_line,
        ));

        if !long_line_reported && !blob_on_line {
            if let Some(f) = minified::check_line(file, n, line) {
                long_line_reported = true;
                out.push(f);
            }
        }
    }

    match lower_name.as_str() {
        "package.json" => out.extend(install::package_json(file, &text)),
        "setup.py" => out.extend(install::setup_py(file, &text)),
        "build.rs" => out.extend(install::build_rs(file, &text)),
        _ if lower_name.starts_with("requirements") && ext == "txt" => {
            out.extend(typosquat::requirements_txt(file, &text))
        }
        _ => {}
    }
    out
}

/// The 1-based line of the first line containing `needle`.
pub(crate) fn line_of(text: &str, needle: &str) -> Option<usize> {
    text.lines().position(|l| l.contains(needle)).map(|i| i + 1)
}
