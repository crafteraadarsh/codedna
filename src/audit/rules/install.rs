//! Install-time execution: npm lifecycle scripts, `setup.py` and Cargo
//! build scripts that download or run code when a package is installed.

use super::line_of;
use super::typosquat::{typosquat, typosquat_finding, NPM_POPULAR};
use crate::audit::{finding, Finding, Severity, INSTALL_SCRIPT};

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
                out.push(finding(
                    Severity::High,
                    INSTALL_SCRIPT,
                    file,
                    line,
                    format!(
                        "\"{hook}\" script uses `{}`: it fetches or runs code when the package is installed",
                        risky.trim()
                    ),
                ));
            } else if *hook != "prepare" && !private {
                out.push(finding(
                    Severity::Low,
                    INSTALL_SCRIPT,
                    file,
                    line,
                    format!("\"{hook}\" script runs automatically when the package is installed"),
                ));
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
            Some(finding(
                Severity::Medium,
                INSTALL_SCRIPT,
                file,
                Some(i + 1),
                format!("setup.py calls `{call}`: this runs when the package is installed"),
            ))
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
            out.push(finding(
                Severity::Medium,
                INSTALL_SCRIPT,
                file,
                Some(i + 1),
                format!("build.rs mentions `{net}`: build scripts that download code run on every build"),
            ));
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
