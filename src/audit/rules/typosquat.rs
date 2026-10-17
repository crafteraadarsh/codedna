//! Typosquatting: dependency names one edit away from a popular package.

use crate::audit::{finding, Finding, Severity, TYPOSQUAT};
use crate::safe_text::sanitize;

/// Longest package name shown in a finding.
const MAX_NAME: usize = 100;

/// Widely used npm packages that attackers imitate.
#[rustfmt::skip]
pub(crate) const NPM_POPULAR: &[&str] = &[
    "react", "react-dom", "express", "lodash", "axios", "chalk", "commander", "moment", "webpack",
    "typescript", "eslint", "tslint", "prettier", "jquery", "redux", "dotenv", "mongoose",
    "nodemon", "socket.io", "body-parser", "jsonwebtoken", "bcrypt", "bcryptjs", "yargs",
    "inquirer", "colors", "color", "debug", "async", "request", "underscore", "rollup", "esbuild",
    "vitest", "mocha", "sinon", "cross-env", "rimraf", "minimist", "semver", "node-fetch",
    "puppeteer", "playwright", "sequelize", "prisma", "graphql", "tailwindcss", "postcss",
    "autoprefixer", "styled-components", "classnames", "electron", "discord.js", "ethers",
    "nodemailer", "dayjs", "date-fns", "cheerio", "babel-cli",
];

/// Widely used PyPI packages that attackers imitate.
#[rustfmt::skip]
const PYPI_POPULAR: &[&str] = &[
    "requests", "numpy", "pandas", "django", "flask", "fastapi", "pydantic", "sqlalchemy", "boto3",
    "botocore", "urllib3", "setuptools", "pytest", "scipy", "matplotlib", "pillow", "cryptography",
    "pyyaml", "jinja2", "celery", "redis", "psycopg2", "pymongo", "aiohttp", "httpx", "uvicorn",
    "gunicorn", "tensorflow", "torch", "transformers", "scikit-learn", "openai", "beautifulsoup4",
    "selenium", "paramiko", "click", "colorama", "tqdm", "python-dateutil", "attrs", "certifi",
    "charset-normalizer", "typing-extensions", "protobuf", "grpcio", "docker", "kubernetes",
    "jsonschema", "websockets", "pyjwt", "bcrypt", "psutil", "pycryptodome", "python-dotenv",
];

/// Real, unrelated packages that happen to be one edit from a popular name.
#[rustfmt::skip]
const KNOWN_NEIGHBOURS: &[&str] = &[
    "preact", "dotenvx", "torchx", "scapy", "jinja", "attr", "psycopg", "boto", "colord",
    "graphiql", "gaxios",
];

/// The popular package `name` is one edit away from, if it is not that
/// package itself. Python names compare case-, `_`- and `.`-insensitively.
pub(crate) fn typosquat(
    name: &str,
    popular: &[&'static str],
    python: bool,
) -> Option<&'static str> {
    let normalize = |s: &str| {
        let s = s.to_ascii_lowercase();
        if python {
            s.replace(['_', '.'], "-")
        } else {
            s
        }
    };
    let name = normalize(name);
    if name.chars().count() < 4
        || KNOWN_NEIGHBOURS.contains(&name.as_str())
        || popular.iter().any(|p| normalize(p) == name)
    {
        return None;
    }
    popular
        .iter()
        .copied()
        .filter(|p| p.len() >= 5)
        .find(|p| edit_distance(&name, &normalize(p)) == 1)
}

pub(crate) fn typosquat_finding(
    file: &str,
    line: Option<usize>,
    name: &str,
    popular: &str,
) -> Finding {
    let shown: String = sanitize(name).chars().take(MAX_NAME).collect();
    finding(
        Severity::Medium,
        TYPOSQUAT,
        file,
        line,
        format!("Dependency \"{shown}\" is one letter away from the popular package \"{popular}\": check it is the one you meant"),
    )
}

/// Edit distance counting insertions, deletions, substitutions and swaps of
/// two neighbouring characters. Returns 2 early for lengths further apart.
pub(crate) fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > 1 {
        return 2;
    }
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = best;
        }
    }
    d[a.len()][b.len()]
}

/// Dependency names in a pip requirements file that imitate a popular package.
pub(crate) fn requirements_txt(file: &str, text: &str) -> Vec<Finding> {
    text.lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('-') {
                return None;
            }
            let end = line
                .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
                .unwrap_or(line.len());
            let name = &line[..end];
            let popular = typosquat(name, PYPI_POPULAR, true)?;
            Some(typosquat_finding(file, Some(i + 1), name, popular))
        })
        .collect()
}
