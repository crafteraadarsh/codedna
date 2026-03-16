//! Framework and database detection module — parallel file scanning via `rayon`.
//!
//! Detects frameworks and databases from dependency manifests:
//! - `package.json`     → JavaScript / Node.js frameworks + databases
//! - `requirements.txt` → Python frameworks + databases
//! - `Cargo.toml`       → Rust crates + databases
//! - `go.mod`           → Go modules
//!
//! Also provides `detect_files_using_framework` for per-file import analysis.
//!
//! Detection rules follow `Detectors.md`.

use rayon::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};

/// Result of framework and database detection for a repository.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrameworkDetectionResult {
    /// Detected application frameworks (e.g., React, Next.js, FastAPI, Axum).
    pub frameworks: Vec<String>,
    /// Detected databases and data-layer technologies (e.g., PostgreSQL, Redis).
    pub databases: Vec<String>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Scan a list of repository files and detect frameworks + databases from any
/// manifest files found among them.
pub fn detect_frameworks(files: &[PathBuf]) -> FrameworkDetectionResult {
    let mut frameworks: Vec<String> = Vec::new();
    let mut databases: Vec<String> = Vec::new();

    for file in files {
        match file.file_name().and_then(|n| n.to_str()) {
            Some("package.json") => {
                let (fw, db) = parse_package_json(file);
                frameworks.extend(fw);
                databases.extend(db);
            }
            Some("requirements.txt") => {
                let (fw, db) = parse_requirements_txt(file);
                frameworks.extend(fw);
                databases.extend(db);
            }
            Some("Cargo.toml") => {
                let (fw, db) = parse_cargo_toml(file);
                frameworks.extend(fw);
                databases.extend(db);
            }
            Some("go.mod") => {
                frameworks.extend(parse_go_mod(file));
            }
            _ => {}
        }
    }

    dedup_sorted(&mut frameworks);
    dedup_sorted(&mut databases);

    FrameworkDetectionResult {
        frameworks,
        databases,
    }
}

/// Detect infrastructure and DevOps tooling from a list of repository files.
///
/// Detection targets (from `Detectors.md`):
/// - Docker        — `Dockerfile` exists
/// - Docker Compose — `docker-compose.yml` / `docker-compose.yaml` exists
/// - GitHub Actions — any file inside `.github/workflows/`
/// - Makefile       — `Makefile` exists
/// - Kubernetes     — `*.yaml` files in a `k8s/` or `kubernetes/` directory
pub fn detect_infrastructure(files: &[PathBuf]) -> Vec<String> {
    let mut infra: Vec<String> = Vec::new();

    for file in files {
        let file_name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");

        match file_name {
            "Dockerfile" => infra.push("Docker".to_string()),
            "docker-compose.yml" | "docker-compose.yaml" => {
                infra.push("Docker Compose".to_string())
            }
            "Makefile" => infra.push("Makefile".to_string()),
            _ => {}
        }

        // GitHub Actions: any file inside .github/workflows/
        let path_str = file.display().to_string();
        if (path_str.contains("/.github/workflows/") || path_str.contains("\\.github\\workflows\\"))
            && !infra.contains(&"GitHub Actions".to_string())
        {
            infra.push("GitHub Actions".to_string());
        }

        // Kubernetes: yaml files in k8s/ or kubernetes/ directories
        if (path_str.contains("/k8s/") || path_str.contains("/kubernetes/"))
            && (file_name.ends_with(".yaml") || file_name.ends_with(".yml"))
            && !infra.contains(&"Kubernetes".to_string())
        {
            infra.push("Kubernetes".to_string());
        }
    }

    dedup_sorted(&mut infra);
    infra
}

/// Sort and deduplicate a `Vec<String>` in place.
fn dedup_sorted(v: &mut Vec<String>) {
    v.sort_unstable();
    v.dedup();
}

// ---------------------------------------------------------------------------
// Per-file framework import detection
// ---------------------------------------------------------------------------

/// Maps a lowercase canonical framework name to a list of
/// `(file_extensions, import_substrings)` matchers.
///
/// A file matches when its extension is in the extensions list AND at least
/// one of the import substrings appears on a non-comment line of the file.
const FRAMEWORK_FILE_PATTERNS: &[(&str, &[&str], &[&str])] = &[
    // ── JavaScript / TypeScript ──────────────────────────────────────────────
    (
        "react",
        &["ts", "tsx", "js", "jsx", "mjs"],
        &[
            "from 'react'",
            "from \"react\"",
            "from 'react-dom'",
            "from \"react-dom\"",
            "require('react')",
            "require(\"react\")",
        ],
    ),
    (
        "next.js",
        &["ts", "tsx", "js", "jsx"],
        &[
            "from 'next'",
            "from \"next\"",
            "from 'next/",
            "from \"next/",
            "require('next')",
            "require(\"next\")",
        ],
    ),
    (
        "express",
        &["ts", "js", "mjs"],
        &[
            "from 'express'",
            "from \"express\"",
            "require('express')",
            "require(\"express\")",
        ],
    ),
    (
        "vite",
        &["ts", "js", "mjs"],
        &[
            "from 'vite'",
            "from \"vite\"",
            "from 'vite/",
            "from \"vite/",
        ],
    ),
    (
        "vue",
        &["ts", "tsx", "js", "jsx", "vue"],
        &[
            "from 'vue'",
            "from \"vue\"",
            "require('vue')",
            "require(\"vue\")",
        ],
    ),
    (
        "svelte",
        &["ts", "js", "svelte"],
        &[
            "from 'svelte'",
            "from \"svelte\"",
            "from 'svelte/",
            "from \"svelte/",
        ],
    ),
    (
        "nestjs",
        &["ts", "js"],
        &["from '@nestjs/", "from \"@nestjs/"],
    ),
    (
        "fastify",
        &["ts", "js", "mjs"],
        &[
            "from 'fastify'",
            "from \"fastify\"",
            "require('fastify')",
            "require(\"fastify\")",
        ],
    ),
    (
        "koa",
        &["ts", "js", "mjs"],
        &[
            "from 'koa'",
            "from \"koa\"",
            "require('koa')",
            "require(\"koa\")",
        ],
    ),
    (
        "remix",
        &["ts", "tsx", "js", "jsx"],
        &["from '@remix-run/", "from \"@remix-run/"],
    ),
    (
        "gatsby",
        &["ts", "tsx", "js", "jsx"],
        &["from 'gatsby'", "from \"gatsby\""],
    ),
    (
        "nuxt",
        &["ts", "js", "vue"],
        &[
            "from 'nuxt'",
            "from \"nuxt\"",
            "from '#app'",
            "from \"#app\"",
        ],
    ),
    (
        "astro",
        &["ts", "js", "astro"],
        &[
            "from 'astro'",
            "from \"astro\"",
            "from 'astro/",
            "from \"astro/",
        ],
    ),
    // ── Python ───────────────────────────────────────────────────────────────
    ("fastapi", &["py"], &["from fastapi", "import fastapi"]),
    ("django", &["py"], &["from django", "import django"]),
    ("flask", &["py"], &["from flask", "import flask"]),
    (
        "starlette",
        &["py"],
        &["from starlette", "import starlette"],
    ),
    ("aiohttp", &["py"], &["from aiohttp", "import aiohttp"]),
    // ── Rust ─────────────────────────────────────────────────────────────────
    ("tokio", &["rs"], &["use tokio", "tokio::"]),
    ("axum", &["rs"], &["use axum", "axum::"]),
    ("actix-web", &["rs"], &["use actix_web", "actix_web::"]),
    ("rocket", &["rs"], &["use rocket", "rocket::"]),
    ("warp", &["rs"], &["use warp", "warp::"]),
    ("leptos", &["rs"], &["use leptos", "leptos::"]),
    ("dioxus", &["rs"], &["use dioxus", "dioxus::"]),
    ("yew", &["rs"], &["use yew", "yew::"]),
    ("tauri", &["rs"], &["use tauri", "tauri::"]),
    // ── Go ───────────────────────────────────────────────────────────────────
    ("gin", &["go"], &["\"github.com/gin-gonic/gin\""]),
    ("echo", &["go"], &["\"github.com/labstack/echo"]),
    ("fiber", &["go"], &["\"github.com/gofiber/fiber"]),
    ("chi", &["go"], &["\"github.com/go-chi/chi"]),
];

/// Scan repository files and return those that contain import statements for
/// the specified framework.
///
/// `framework` is matched case-insensitively against the canonical names in
/// `FRAMEWORK_FILE_PATTERNS`.
///
/// Returns a sorted `Vec<PathBuf>` of matching files.
pub fn detect_files_using_framework(files: &[PathBuf], framework: &str) -> Vec<PathBuf> {
    let framework_lower = framework.to_lowercase();

    // Look up matchers for the requested framework.
    let matchers: Vec<(&[&str], &[&str])> = FRAMEWORK_FILE_PATTERNS
        .iter()
        .filter(|(name, _, _)| *name == framework_lower.as_str())
        .map(|(_, exts, patterns)| (*exts, *patterns))
        .collect();

    if matchers.is_empty() {
        return Vec::new();
    }

    // Each file is checked independently — par_iter gives free parallelism.
    let mut results: Vec<PathBuf> = files
        .par_iter()
        .filter(|file| {
            let ext = file
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();

            // Find matchers whose extension list includes this file's extension.
            let applicable: Vec<&[&str]> = matchers
                .iter()
                .filter(|(exts, _)| exts.contains(&ext.as_str()))
                .map(|(_, patterns)| *patterns)
                .collect();

            if applicable.is_empty() {
                return false;
            }

            // Read the file and scan line by line.
            let content = match fs::read_to_string(file.as_path()) {
                Ok(c) => c,
                Err(_) => return false,
            };

            for line in content.lines() {
                let trimmed = line.trim();

                // Skip comment lines.
                // Note: use "# " (hash + space) for Python-style comments so that
                // Rust attributes like `#[tokio::main]` are NOT skipped.
                if trimmed.starts_with("//")
                    || trimmed.starts_with("# ")
                    || trimmed == "#"
                    || trimmed.starts_with("/*")
                    || trimmed.starts_with('*')
                {
                    continue;
                }

                for patterns in &applicable {
                    for pattern in *patterns {
                        if trimmed.contains(pattern) {
                            return true;
                        }
                    }
                }
            }

            false
        })
        .cloned()
        .collect();

    results.sort_unstable();
    results
}

// ---------------------------------------------------------------------------
// package.json — JavaScript / Node.js frameworks
// ---------------------------------------------------------------------------

/// JS framework detection rules: (package name, display label)
const JS_FRAMEWORK_RULES: &[(&str, &str)] = &[
    ("react", "React"),
    ("next", "Next.js"),
    ("express", "Express"),
    ("vite", "Vite"),
    ("vue", "Vue"),
    ("nuxt", "Nuxt"),
    ("svelte", "Svelte"),
    ("astro", "Astro"),
    ("@nestjs/core", "NestJS"),
    ("fastify", "Fastify"),
    ("koa", "Koa"),
    ("hapi", "Hapi"),
    ("remix", "Remix"),
    ("gatsby", "Gatsby"),
];

/// JS database detection rules: (package name, display label)
///
/// Rules from `Detectors.md`.
const JS_DATABASE_RULES: &[(&str, &str)] = &[
    ("pg", "PostgreSQL"),
    ("postgres", "PostgreSQL"),
    ("mongoose", "MongoDB"),
    ("mongodb", "MongoDB"),
    ("mysql", "MySQL"),
    ("mysql2", "MySQL"),
    ("redis", "Redis"),
    ("ioredis", "Redis"),
    ("prisma", "Prisma"),
    ("@prisma/client", "Prisma"),
    ("better-sqlite3", "SQLite"),
    ("sqlite3", "SQLite"),
    ("knex", "Knex"),
    ("sequelize", "Sequelize"),
    ("typeorm", "TypeORM"),
    ("drizzle-orm", "Drizzle ORM"),
    ("cassandra-driver", "Cassandra"),
    ("couchdb", "CouchDB"),
    ("@elastic/elasticsearch", "Elasticsearch"),
    ("dynamoose", "DynamoDB"),
];

/// Parse `package.json` and return `(frameworks, databases)`.
///
/// Checks `dependencies`, `devDependencies`, and `peerDependencies`.
fn parse_package_json(path: &Path) -> (Vec<String>, Vec<String>) {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return (Vec::new(), Vec::new()),
    };

    let json: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return (Vec::new(), Vec::new()),
    };

    let deps = collect_package_json_keys(&json, "dependencies");
    let dev_deps = collect_package_json_keys(&json, "devDependencies");
    let peer_deps = collect_package_json_keys(&json, "peerDependencies");

    let all_deps: Vec<&str> = deps
        .iter()
        .chain(dev_deps.iter())
        .chain(peer_deps.iter())
        .map(|s| s.as_str())
        .collect();

    let mut frameworks = Vec::new();
    for (package, label) in JS_FRAMEWORK_RULES {
        if all_deps.contains(package) {
            frameworks.push(label.to_string());
        }
    }

    // Deduplicate databases (e.g. both "pg" and "postgres" → only one "PostgreSQL")
    let mut databases: Vec<String> = Vec::new();
    for (package, label) in JS_DATABASE_RULES {
        if all_deps.contains(package) && !databases.contains(&label.to_string()) {
            databases.push(label.to_string());
        }
    }

    (frameworks, databases)
}

/// Extract all package names from a specific section of `package.json`.
fn collect_package_json_keys(json: &serde_json::Value, section: &str) -> Vec<String> {
    json.get(section)
        .and_then(|v| v.as_object())
        .map(|obj| obj.keys().cloned().collect())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// requirements.txt — Python frameworks + databases
// ---------------------------------------------------------------------------

/// Python framework detection rules: (lowercase package name, display label)
const PYTHON_FRAMEWORK_RULES: &[(&str, &str)] = &[
    ("fastapi", "FastAPI"),
    ("django", "Django"),
    ("flask", "Flask"),
    ("starlette", "Starlette"),
    ("tornado", "Tornado"),
    ("aiohttp", "aiohttp"),
    ("pyramid", "Pyramid"),
    ("sanic", "Sanic"),
    ("litestar", "Litestar"),
];

/// Python database detection rules: (lowercase package name, display label)
///
/// Rules from `Detectors.md`.
const PYTHON_DATABASE_RULES: &[(&str, &str)] = &[
    ("sqlalchemy", "SQLAlchemy"),
    ("psycopg2", "PostgreSQL"),
    ("psycopg2-binary", "PostgreSQL"),
    ("psycopg", "PostgreSQL"),
    ("asyncpg", "PostgreSQL"),
    ("pymongo", "MongoDB"),
    ("motor", "MongoDB"),
    ("redis", "Redis"),
    ("aioredis", "Redis"),
    ("pymysql", "MySQL"),
    ("aiomysql", "MySQL"),
    ("mysqlclient", "MySQL"),
    ("tortoise-orm", "Tortoise ORM"),
    ("peewee", "Peewee"),
    ("databases", "Databases"),
    ("elasticsearch", "Elasticsearch"),
    ("cassandra-driver", "Cassandra"),
];

/// Parse `requirements.txt` and return `(frameworks, databases)`.
///
/// Handles version specifiers (e.g. `fastapi==0.95.0`, `Django>=4.0`).
fn parse_requirements_txt(path: &Path) -> (Vec<String>, Vec<String>) {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return (Vec::new(), Vec::new()),
    };

    let mut frameworks = Vec::new();
    let mut databases = Vec::new();

    for line in content.lines() {
        let line = line.trim();

        // Skip comments and empty lines.
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Strip version specifiers: `fastapi==0.95.0` → `fastapi`
        let package_name = line
            .split(|c: char| c == '=' || c == '>' || c == '<' || c == '!' || c == '[' || c == ';')
            .next()
            .unwrap_or(line)
            .trim()
            .to_lowercase();

        for (name, label) in PYTHON_FRAMEWORK_RULES {
            if package_name == *name {
                frameworks.push(label.to_string());
                break;
            }
        }

        for (name, label) in PYTHON_DATABASE_RULES {
            if package_name == *name && !databases.contains(&label.to_string()) {
                databases.push(label.to_string());
                break;
            }
        }
    }

    (frameworks, databases)
}

// ---------------------------------------------------------------------------
// Cargo.toml — Rust frameworks / crates + databases
// ---------------------------------------------------------------------------

/// Rust framework/crate detection rules: (crate name, display label)
const RUST_CRATE_RULES: &[(&str, &str)] = &[
    ("tokio", "Tokio"),
    ("axum", "Axum"),
    ("actix-web", "Actix-web"),
    ("rocket", "Rocket"),
    ("warp", "Warp"),
    ("tonic", "Tonic"),
    ("poem", "Poem"),
    ("salvo", "Salvo"),
    ("tide", "Tide"),
    ("hyper", "Hyper"),
    ("tauri", "Tauri"),
    ("leptos", "Leptos"),
    ("dioxus", "Dioxus"),
    ("yew", "Yew"),
];

/// Rust database crate detection rules: (crate name, display label)
const RUST_DATABASE_RULES: &[(&str, &str)] = &[
    ("sqlx", "SQLx"),
    ("diesel", "Diesel"),
    ("sea-orm", "SeaORM"),
    ("tokio-postgres", "PostgreSQL"),
    ("postgres", "PostgreSQL"),
    ("mongodb", "MongoDB"),
    ("redis", "Redis"),
    ("rusqlite", "SQLite"),
    ("sled", "Sled"),
    ("elasticsearch", "Elasticsearch"),
];

/// Parse `Cargo.toml` and return `(frameworks, databases)`.
///
/// Uses section-aware line scanning to stay within `[dependencies]` blocks.
fn parse_cargo_toml(path: &Path) -> (Vec<String>, Vec<String>) {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return (Vec::new(), Vec::new()),
    };

    let mut frameworks = Vec::new();
    let mut databases = Vec::new();
    let mut in_dependencies = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Track section boundaries.
        if trimmed.starts_with('[') {
            in_dependencies = matches!(
                trimmed,
                "[dependencies]"
                    | "[dev-dependencies]"
                    | "[build-dependencies]"
                    | "[workspace.dependencies]"
            );
            continue;
        }

        if !in_dependencies {
            continue;
        }

        // Dependency key: `axum = "0.7"` or `tokio = { version = "1" }`
        let key = trimmed
            .split('=')
            .next()
            .unwrap_or("")
            .trim()
            .trim_matches('"');

        for (crate_name, label) in RUST_CRATE_RULES {
            if key == *crate_name {
                frameworks.push(label.to_string());
                break;
            }
        }

        for (crate_name, label) in RUST_DATABASE_RULES {
            if key == *crate_name && !databases.contains(&label.to_string()) {
                databases.push(label.to_string());
                break;
            }
        }
    }

    (frameworks, databases)
}

// ---------------------------------------------------------------------------
// go.mod — Go modules / frameworks
// ---------------------------------------------------------------------------

/// Go module detection rules: (module path substring, display label)
const GO_MODULE_RULES: &[(&str, &str)] = &[
    ("github.com/gin-gonic/gin", "Gin"),
    ("github.com/labstack/echo", "Echo"),
    ("github.com/gofiber/fiber", "Fiber"),
    ("github.com/gorilla/mux", "Gorilla Mux"),
    ("github.com/beego/beego", "Beego"),
    ("github.com/go-chi/chi", "Chi"),
    ("github.com/revel/revel", "Revel"),
    ("go.uber.org/fx", "Uber FX"),
];

/// Parse `go.mod` and return detected Go framework names.
fn parse_go_mod(path: &Path) -> Vec<String> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut detected = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        for (module_path, label) in GO_MODULE_RULES {
            if trimmed.contains(module_path) {
                detected.push(label.to_string());
                break;
            }
        }
    }

    detected
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}_{nanos}"));
        fs::create_dir_all(&path).expect("failed to create temp dir");
        path
    }

    // -----------------------------------------------------------------------
    // detect_files_using_framework
    // -----------------------------------------------------------------------

    #[test]
    fn detects_react_usage_in_tsx_files() {
        let dir = unique_temp_dir("codedna_ffu_react");
        let app = dir.join("App.tsx");
        let other = dir.join("utils.ts");
        fs::write(
            &app,
            "import React from 'react';\nexport const App = () => null;",
        )
        .unwrap();
        fs::write(&other, "export const add = (a: number) => a + 1;").unwrap();

        let result = detect_files_using_framework(&[app.clone(), other.clone()], "react");
        assert!(result.contains(&app));
        assert!(!result.contains(&other));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_react_from_statement() {
        let dir = unique_temp_dir("codedna_ffu_react_from");
        let f = dir.join("index.tsx");
        fs::write(&f, "import { useState } from 'react';").unwrap();

        let result = detect_files_using_framework(&[f.clone()], "react");
        assert!(result.contains(&f));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_express_require_in_js() {
        let dir = unique_temp_dir("codedna_ffu_express");
        let f = dir.join("server.js");
        fs::write(
            &f,
            "const express = require('express');\nconst app = express();",
        )
        .unwrap();

        let result = detect_files_using_framework(&[f.clone()], "express");
        assert!(result.contains(&f));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_fastapi_in_python_file() {
        let dir = unique_temp_dir("codedna_ffu_fastapi");
        let f = dir.join("main.py");
        fs::write(&f, "from fastapi import FastAPI\napp = FastAPI()").unwrap();

        let result = detect_files_using_framework(&[f.clone()], "fastapi");
        assert!(result.contains(&f));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_django_in_python_file() {
        let dir = unique_temp_dir("codedna_ffu_django");
        let f = dir.join("models.py");
        fs::write(
            &f,
            "from django.db import models\nclass User(models.Model): pass",
        )
        .unwrap();

        let result = detect_files_using_framework(&[f.clone()], "django");
        assert!(result.contains(&f));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_axum_in_rust_file() {
        let dir = unique_temp_dir("codedna_ffu_axum");
        let f = dir.join("server.rs");
        fs::write(&f, "use axum::{Router, routing::get};\nfn main() {}").unwrap();

        let result = detect_files_using_framework(&[f.clone()], "axum");
        assert!(result.contains(&f));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_tokio_in_rust_file() {
        let dir = unique_temp_dir("codedna_ffu_tokio");
        let f = dir.join("main.rs");
        fs::write(&f, "#[tokio::main]\nasync fn main() {}").unwrap();

        let result = detect_files_using_framework(&[f.clone()], "tokio");
        assert!(result.contains(&f));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn skips_commented_import_lines() {
        let dir = unique_temp_dir("codedna_ffu_comments");
        let f = dir.join("server.ts");
        // commented out — should NOT be flagged
        fs::write(&f, "// import express from \"express\";\nconst x = 1;").unwrap();

        let result = detect_files_using_framework(&[f.clone()], "express");
        assert!(result.is_empty());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn returns_empty_for_unknown_framework() {
        let dir = unique_temp_dir("codedna_ffu_unknown");
        let f = dir.join("app.ts");
        fs::write(&f, "import something from 'somewhere';").unwrap();

        let result = detect_files_using_framework(&[f.clone()], "nonexistentframework");
        assert!(result.is_empty());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn returns_empty_when_framework_not_used_in_files() {
        let dir = unique_temp_dir("codedna_ffu_not_used");
        let f = dir.join("app.ts");
        fs::write(&f, "import { something } from './local';").unwrap();

        let result = detect_files_using_framework(&[f.clone()], "react");
        assert!(result.is_empty());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_multiple_files_using_same_framework() {
        let dir = unique_temp_dir("codedna_ffu_multi");
        let a = dir.join("App.tsx");
        let b = dir.join("Navbar.tsx");
        let c = dir.join("utils.ts"); // no React

        fs::write(&a, "import React from 'react';").unwrap();
        fs::write(&b, "import { useState } from 'react';").unwrap();
        fs::write(&c, "export const x = 1;").unwrap();

        let mut result = detect_files_using_framework(&[a.clone(), b.clone(), c.clone()], "react");
        result.sort_unstable();

        assert!(result.contains(&a));
        assert!(result.contains(&b));
        assert!(!result.contains(&c));
        assert_eq!(result.len(), 2);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn output_is_sorted() {
        let dir = unique_temp_dir("codedna_ffu_sorted");
        let z = dir.join("z_component.tsx");
        let a = dir.join("a_component.tsx");

        fs::write(&z, "import React from 'react';").unwrap();
        fs::write(&a, "import React from 'react';").unwrap();

        let result = detect_files_using_framework(&[z.clone(), a.clone()], "react");
        let mut expected = result.clone();
        expected.sort_unstable();
        assert_eq!(result, expected);
        fs::remove_dir_all(dir).ok();
    }

    // -----------------------------------------------------------------------
    // package.json — frameworks
    // -----------------------------------------------------------------------

    #[test]
    fn detects_react_from_package_json() {
        let dir = unique_temp_dir("codedna_fw_react");
        let file = dir.join("package.json");
        fs::write(
            &file,
            r#"{ "dependencies": { "react": "^18.0.0", "react-dom": "^18.0.0" } }"#,
        )
        .unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.frameworks.contains(&"React".to_string()));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_nextjs_from_package_json() {
        let dir = unique_temp_dir("codedna_fw_next");
        let file = dir.join("package.json");
        fs::write(&file, r#"{ "dependencies": { "next": "14.0.0" } }"#).unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.frameworks.contains(&"Next.js".to_string()));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_express_from_package_json() {
        let dir = unique_temp_dir("codedna_fw_express");
        let file = dir.join("package.json");
        fs::write(&file, r#"{ "dependencies": { "express": "^4.18.0" } }"#).unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.frameworks.contains(&"Express".to_string()));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_vite_from_dev_dependencies() {
        let dir = unique_temp_dir("codedna_fw_vite");
        let file = dir.join("package.json");
        fs::write(&file, r#"{ "devDependencies": { "vite": "^5.0.0" } }"#).unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.frameworks.contains(&"Vite".to_string()));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_vue_from_package_json() {
        let dir = unique_temp_dir("codedna_fw_vue");
        let file = dir.join("package.json");
        fs::write(&file, r#"{ "dependencies": { "vue": "^3.3.0" } }"#).unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.frameworks.contains(&"Vue".to_string()));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_multiple_js_frameworks() {
        let dir = unique_temp_dir("codedna_fw_multi_js");
        let file = dir.join("package.json");
        fs::write(
            &file,
            r#"{
                "dependencies": { "react": "^18.0.0", "express": "^4.18.0" },
                "devDependencies": { "vite": "^5.0.0" }
            }"#,
        )
        .unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.frameworks.contains(&"React".to_string()));
        assert!(result.frameworks.contains(&"Express".to_string()));
        assert!(result.frameworks.contains(&"Vite".to_string()));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn returns_empty_for_no_known_frameworks_in_package_json() {
        let dir = unique_temp_dir("codedna_fw_no_known_js");
        let file = dir.join("package.json");
        fs::write(&file, r#"{ "dependencies": { "lodash": "^4.17.0" } }"#).unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.frameworks.is_empty());
        fs::remove_dir_all(dir).ok();
    }

    // -----------------------------------------------------------------------
    // package.json — databases
    // -----------------------------------------------------------------------

    #[test]
    fn detects_postgresql_via_pg_from_package_json() {
        let dir = unique_temp_dir("codedna_db_pg");
        let file = dir.join("package.json");
        fs::write(&file, r#"{ "dependencies": { "pg": "^8.0.0" } }"#).unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.databases.contains(&"PostgreSQL".to_string()));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_postgresql_via_postgres_from_package_json() {
        let dir = unique_temp_dir("codedna_db_postgres");
        let file = dir.join("package.json");
        fs::write(&file, r#"{ "dependencies": { "postgres": "^3.0.0" } }"#).unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.databases.contains(&"PostgreSQL".to_string()));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn deduplicates_postgresql_when_both_pg_and_postgres_present() {
        let dir = unique_temp_dir("codedna_db_pg_dedup");
        let file = dir.join("package.json");
        fs::write(
            &file,
            r#"{ "dependencies": { "pg": "^8.0.0", "postgres": "^3.0.0" } }"#,
        )
        .unwrap();
        let result = detect_frameworks(&[file]);
        let count = result
            .databases
            .iter()
            .filter(|d| d.as_str() == "PostgreSQL")
            .count();
        assert_eq!(count, 1, "PostgreSQL should appear only once");
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_mongodb_via_mongoose_from_package_json() {
        let dir = unique_temp_dir("codedna_db_mongoose");
        let file = dir.join("package.json");
        fs::write(&file, r#"{ "dependencies": { "mongoose": "^7.0.0" } }"#).unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.databases.contains(&"MongoDB".to_string()));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_mongodb_via_mongodb_driver_from_package_json() {
        let dir = unique_temp_dir("codedna_db_mongodb");
        let file = dir.join("package.json");
        fs::write(&file, r#"{ "dependencies": { "mongodb": "^5.0.0" } }"#).unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.databases.contains(&"MongoDB".to_string()));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_mysql_from_package_json() {
        let dir = unique_temp_dir("codedna_db_mysql");
        let file = dir.join("package.json");
        fs::write(&file, r#"{ "dependencies": { "mysql2": "^3.0.0" } }"#).unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.databases.contains(&"MySQL".to_string()));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_redis_from_package_json() {
        let dir = unique_temp_dir("codedna_db_redis");
        let file = dir.join("package.json");
        fs::write(&file, r#"{ "dependencies": { "redis": "^4.0.0" } }"#).unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.databases.contains(&"Redis".to_string()));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_redis_via_ioredis_from_package_json() {
        let dir = unique_temp_dir("codedna_db_ioredis");
        let file = dir.join("package.json");
        fs::write(&file, r#"{ "dependencies": { "ioredis": "^5.0.0" } }"#).unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.databases.contains(&"Redis".to_string()));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_prisma_from_package_json() {
        let dir = unique_temp_dir("codedna_db_prisma");
        let file = dir.join("package.json");
        fs::write(
            &file,
            r#"{ "dependencies": { "@prisma/client": "^5.0.0" }, "devDependencies": { "prisma": "^5.0.0" } }"#,
        )
        .unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.databases.contains(&"Prisma".to_string()));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_full_stack_project_databases_and_frameworks() {
        let dir = unique_temp_dir("codedna_db_full_stack");
        let file = dir.join("package.json");
        fs::write(
            &file,
            r#"{
                "dependencies": {
                    "react": "^18.0.0",
                    "express": "^4.18.0",
                    "pg": "^8.0.0",
                    "redis": "^4.0.0"
                },
                "devDependencies": { "vite": "^5.0.0" }
            }"#,
        )
        .unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.frameworks.contains(&"React".to_string()));
        assert!(result.frameworks.contains(&"Express".to_string()));
        assert!(result.frameworks.contains(&"Vite".to_string()));
        assert!(result.databases.contains(&"PostgreSQL".to_string()));
        assert!(result.databases.contains(&"Redis".to_string()));
        fs::remove_dir_all(dir).ok();
    }

    // -----------------------------------------------------------------------
    // requirements.txt — frameworks
    // -----------------------------------------------------------------------

    #[test]
    fn detects_fastapi_from_requirements_txt() {
        let dir = unique_temp_dir("codedna_fw_fastapi");
        let file = dir.join("requirements.txt");
        fs::write(&file, "fastapi==0.95.0\nuvicorn>=0.20.0\n").unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.frameworks.contains(&"FastAPI".to_string()));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_django_from_requirements_txt() {
        let dir = unique_temp_dir("codedna_fw_django");
        let file = dir.join("requirements.txt");
        fs::write(&file, "Django>=4.0\npsycopg2-binary\n").unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.frameworks.contains(&"Django".to_string()));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_flask_from_requirements_txt() {
        let dir = unique_temp_dir("codedna_fw_flask");
        let file = dir.join("requirements.txt");
        fs::write(&file, "flask==2.3.0\nclick\n").unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.frameworks.contains(&"Flask".to_string()));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn ignores_comments_and_empty_lines_in_requirements_txt() {
        let dir = unique_temp_dir("codedna_fw_req_comments");
        let file = dir.join("requirements.txt");
        fs::write(&file, "# production dependencies\n\nflask==2.3.0\n").unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.frameworks.contains(&"Flask".to_string()));
        assert_eq!(result.frameworks.len(), 1);
        fs::remove_dir_all(dir).ok();
    }

    // -----------------------------------------------------------------------
    // requirements.txt — databases
    // -----------------------------------------------------------------------

    #[test]
    fn detects_sqlalchemy_from_requirements_txt() {
        let dir = unique_temp_dir("codedna_db_sqlalchemy");
        let file = dir.join("requirements.txt");
        fs::write(&file, "sqlalchemy>=2.0\n").unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.databases.contains(&"SQLAlchemy".to_string()));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_postgresql_via_psycopg2_from_requirements_txt() {
        let dir = unique_temp_dir("codedna_db_psycopg2");
        let file = dir.join("requirements.txt");
        fs::write(&file, "psycopg2-binary==2.9.6\n").unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.databases.contains(&"PostgreSQL".to_string()));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_redis_from_requirements_txt() {
        let dir = unique_temp_dir("codedna_db_redis_py");
        let file = dir.join("requirements.txt");
        fs::write(&file, "redis==4.6.0\n").unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.databases.contains(&"Redis".to_string()));
        fs::remove_dir_all(dir).ok();
    }

    // -----------------------------------------------------------------------
    // Cargo.toml — frameworks
    // -----------------------------------------------------------------------

    #[test]
    fn detects_tokio_from_cargo_toml() {
        let dir = unique_temp_dir("codedna_fw_tokio");
        let file = dir.join("Cargo.toml");
        fs::write(
            &file,
            "[package]\nname = \"myapp\"\n\n[dependencies]\ntokio = { version = \"1\", features = [\"full\"] }\n",
        )
        .unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.frameworks.contains(&"Tokio".to_string()));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_axum_from_cargo_toml() {
        let dir = unique_temp_dir("codedna_fw_axum");
        let file = dir.join("Cargo.toml");
        fs::write(
            &file,
            "[package]\nname = \"myapp\"\n\n[dependencies]\naxum = \"0.7\"\n",
        )
        .unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.frameworks.contains(&"Axum".to_string()));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_actix_from_cargo_toml() {
        let dir = unique_temp_dir("codedna_fw_actix");
        let file = dir.join("Cargo.toml");
        fs::write(
            &file,
            "[package]\nname = \"myapp\"\n\n[dependencies]\nactix-web = \"4\"\n",
        )
        .unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.frameworks.contains(&"Actix-web".to_string()));
        fs::remove_dir_all(dir).ok();
    }

    // -----------------------------------------------------------------------
    // Cargo.toml — databases
    // -----------------------------------------------------------------------

    #[test]
    fn detects_sqlx_from_cargo_toml() {
        let dir = unique_temp_dir("codedna_db_sqlx");
        let file = dir.join("Cargo.toml");
        fs::write(
            &file,
            "[package]\nname = \"myapp\"\n\n[dependencies]\nsqlx = \"0.7\"\n",
        )
        .unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.databases.contains(&"SQLx".to_string()));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_diesel_from_cargo_toml() {
        let dir = unique_temp_dir("codedna_db_diesel");
        let file = dir.join("Cargo.toml");
        fs::write(
            &file,
            "[package]\nname = \"myapp\"\n\n[dependencies]\ndiesel = { version = \"2\", features = [\"postgres\"] }\n",
        )
        .unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.databases.contains(&"Diesel".to_string()));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_redis_from_cargo_toml() {
        let dir = unique_temp_dir("codedna_db_redis_rs");
        let file = dir.join("Cargo.toml");
        fs::write(
            &file,
            "[package]\nname = \"myapp\"\n\n[dependencies]\nredis = \"0.24\"\n",
        )
        .unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.databases.contains(&"Redis".to_string()));
        fs::remove_dir_all(dir).ok();
    }

    // -----------------------------------------------------------------------
    // go.mod — frameworks
    // -----------------------------------------------------------------------

    #[test]
    fn detects_gin_from_go_mod() {
        let dir = unique_temp_dir("codedna_fw_gin");
        let file = dir.join("go.mod");
        fs::write(
            &file,
            "module myapp\n\ngo 1.21\n\nrequire github.com/gin-gonic/gin v1.9.0\n",
        )
        .unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.frameworks.contains(&"Gin".to_string()));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_echo_from_go_mod() {
        let dir = unique_temp_dir("codedna_fw_echo");
        let file = dir.join("go.mod");
        fs::write(
            &file,
            "module myapp\n\ngo 1.21\n\nrequire github.com/labstack/echo v4.11.0\n",
        )
        .unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.frameworks.contains(&"Echo".to_string()));
        fs::remove_dir_all(dir).ok();
    }

    // -----------------------------------------------------------------------
    // detect_frameworks wiring
    // -----------------------------------------------------------------------

    #[test]
    fn handles_missing_manifest_gracefully() {
        let missing = PathBuf::from("/no/such/path/package.json");
        let result = detect_frameworks(&[missing]);
        assert!(result.frameworks.is_empty());
        assert!(result.databases.is_empty());
    }

    #[test]
    fn handles_malformed_package_json_gracefully() {
        let dir = unique_temp_dir("codedna_fw_malformed");
        let file = dir.join("package.json");
        fs::write(&file, "this is not json {{{{").unwrap();
        let result = detect_frameworks(&[file]);
        assert!(result.frameworks.is_empty());
        assert!(result.databases.is_empty());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn deduplicates_frameworks_across_multiple_manifests() {
        let dir = unique_temp_dir("codedna_fw_dedup");
        let pkg = dir.join("package.json");
        fs::write(&pkg, r#"{ "dependencies": { "react": "^18.0.0" } }"#).unwrap();
        let sub = dir.join("sub");
        fs::create_dir_all(&sub).unwrap();
        let pkg2 = sub.join("package.json");
        fs::write(&pkg2, r#"{ "dependencies": { "react": "^18.0.0" } }"#).unwrap();
        let result = detect_frameworks(&[pkg, pkg2]);
        let react_count = result
            .frameworks
            .iter()
            .filter(|f| f.as_str() == "React")
            .count();
        assert_eq!(react_count, 1, "React should appear only once");
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn deduplicates_databases_across_multiple_manifests() {
        let dir = unique_temp_dir("codedna_db_dedup");
        let pkg = dir.join("package.json");
        fs::write(&pkg, r#"{ "dependencies": { "pg": "^8.0.0" } }"#).unwrap();
        let sub = dir.join("sub");
        fs::create_dir_all(&sub).unwrap();
        let pkg2 = sub.join("package.json");
        fs::write(&pkg2, r#"{ "dependencies": { "pg": "^8.0.0" } }"#).unwrap();
        let result = detect_frameworks(&[pkg, pkg2]);
        let pg_count = result
            .databases
            .iter()
            .filter(|d| d.as_str() == "PostgreSQL")
            .count();
        assert_eq!(pg_count, 1, "PostgreSQL should appear only once");
        fs::remove_dir_all(dir).ok();
    }
}
