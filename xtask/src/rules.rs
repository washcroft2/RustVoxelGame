//! Crate dependency rules from AGENTS.md ("Architecture rules") and plan section 3, checked
//! against `cargo metadata`. Throwaway spikes and xtask itself are exempt.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, anyhow};
use serde_json::Value;

/// Where a workspace package lives, which decides which rules apply to it.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum Home {
    Crate,
    App,
    /// Phase 0 spikes and xtask: exempt.
    Exempt,
}

struct Node {
    home: Option<Home>,
    /// Direct normal and build dependencies, by package name.
    deps: BTreeSet<String>,
}

pub struct Graph {
    nodes: BTreeMap<String, Node>,
}

#[derive(Debug, Eq, PartialEq)]
pub struct Violation {
    pub rule: &'static str,
    pub package: String,
    pub detail: String,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}: {}", self.rule, self.package, self.detail)
    }
}

/// Parses `cargo metadata --format-version 1` output. Dev-dependencies are ignored, because
/// tests may use tools the shipped crate must not.
pub fn parse(json: &str) -> Result<Graph> {
    let v: Value = serde_json::from_str(json).context("cargo metadata output is not JSON")?;
    let missing = || anyhow!("cargo metadata output has an unexpected shape");
    let mut name_of = BTreeMap::new();
    let mut home_of = BTreeMap::new();
    let members: BTreeSet<&str> = v["workspace_members"]
        .as_array()
        .ok_or_else(missing)?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    for p in v["packages"].as_array().ok_or_else(missing)? {
        let (id, name) = (
            p["id"].as_str().ok_or_else(missing)?,
            p["name"].as_str().ok_or_else(missing)?,
        );
        name_of.insert(id, name);
        if members.contains(id) {
            let path = p["manifest_path"]
                .as_str()
                .unwrap_or_default()
                .replace('\\', "/");
            let home = if path.contains("/crates/") {
                Home::Crate
            } else if path.contains("/apps/") {
                Home::App
            } else {
                Home::Exempt
            };
            home_of.insert(id, home);
        }
    }
    let mut nodes: BTreeMap<String, Node> = BTreeMap::new();
    for n in v["resolve"]["nodes"].as_array().ok_or_else(missing)? {
        let id = n["id"].as_str().ok_or_else(missing)?;
        let name = name_of.get(id).ok_or_else(missing)?;
        let node = nodes.entry((*name).to_owned()).or_insert_with(|| Node {
            home: home_of.get(id).copied(),
            deps: BTreeSet::new(),
        });
        for d in n["deps"].as_array().ok_or_else(missing)? {
            // `kind` is null for normal dependencies, "build" or "dev" otherwise.
            let shipped = d["dep_kinds"]
                .as_array()
                .ok_or_else(missing)?
                .iter()
                .any(|k| k["kind"].as_str() != Some("dev"));
            if shipped && let Some(dep) = d["pkg"].as_str().and_then(|id| name_of.get(id)) {
                node.deps.insert((*dep).to_owned());
            }
        }
    }
    Ok(Graph { nodes })
}

/// Packages allowed to depend on a crate directly. Anything else that does is a violation.
const OWNERS: &[(&str, &[&str])] = &[
    ("wgpu", &["voxel-render"]),
    ("egui-wgpu", &["voxel-render"]),
    ("wasmtime", &["voxel-script"]),
    ("redb", &["voxel-storage"]),
    ("hecs", &["voxel-sim"]),
    ("quinn", &["voxel-net"]),
    ("tokio", &["voxel-net"]),
    ("kira", &["voxel-audio"]),
];

/// Crates that must stay free of GPU, window, network, storage and script code.
const PURE: &[&str] = &["voxel-mesh", "voxel-light", "voxel-proto", "voxel-worldgen"];
const IMPURE: &[&str] = &[
    "wgpu",
    "winit",
    "egui",
    "egui-wgpu",
    "tokio",
    "quinn",
    "rustls",
    "redb",
    "wasmtime",
    "kira",
    "gilrs",
    "ureq",
    "voxel-render",
    "voxel-audio",
    "voxel-net",
    "voxel-storage",
    "voxel-script",
    "voxel-server",
    "voxel-client",
];

impl Graph {
    /// Every package reachable from `start` through dependencies, not including `start`.
    fn reachable(&self, start: &str) -> BTreeSet<&str> {
        let mut seen = BTreeSet::new();
        let mut todo = vec![start];
        while let Some(name) = todo.pop() {
            let Some(node) = self.nodes.get(name) else {
                continue;
            };
            for d in &node.deps {
                if seen.insert(d.as_str()) {
                    todo.push(d);
                }
            }
        }
        seen
    }

    pub fn check(&self) -> Vec<Violation> {
        let mut out = Vec::new();
        let mut add = |rule, package: &str, detail: String| {
            out.push(Violation {
                rule,
                package: package.to_owned(),
                detail,
            });
        };

        for (name, node) in &self.nodes {
            let Some(home) = node.home else { continue };
            if home == Home::Exempt {
                continue;
            }
            for (dep, owners) in OWNERS {
                if node.deps.contains(*dep) && !owners.contains(&name.as_str()) {
                    add(
                        "owned-dependency",
                        name,
                        format!(
                            "depends on `{dep}`, which only {} may use",
                            owners.join(", ")
                        ),
                    );
                }
            }
            if home == Home::Crate && node.deps.contains("anyhow") {
                add(
                    "anyhow-only-in-apps",
                    name,
                    "library crates use thiserror; anyhow belongs in apps/ and xtask".into(),
                );
            }
        }

        let forbid = |out: &mut Vec<Violation>, rule, package: &str, banned: &[&str]| {
            if !self.nodes.contains_key(package) {
                return;
            }
            let reach = self.reachable(package);
            for b in banned {
                if reach.contains(b) {
                    out.push(Violation {
                        rule,
                        package: package.to_owned(),
                        detail: format!("depends, directly or not, on `{b}`"),
                    });
                }
            }
        };
        forbid(
            &mut out,
            "server-has-no-presentation",
            "voxel-server",
            &["voxel-render", "voxel-audio", "winit"],
        );
        forbid(
            &mut out,
            "client-has-no-worldgen",
            "voxel-client",
            &["voxel-worldgen"],
        );
        for pure in PURE {
            forbid(&mut out, "pure-crates-stay-pure", pure, IMPURE);
        }
        if let Some(core) = self.nodes.get("voxel-core") {
            for d in &core.deps {
                if d != "thiserror" {
                    out.push(Violation {
                        rule: "core-stays-light",
                        package: "voxel-core".into(),
                        detail: format!("depends on `{d}`; core allows only thiserror"),
                    });
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny workspace as `cargo metadata` would describe it: `(name, home dir, deps)`.
    fn metadata(pkgs: &[(&str, &str, &[&str])]) -> String {
        let id = |n: &str| format!("path+file:///w/{n}#0.0.0");
        let packages: Vec<Value> = pkgs
            .iter()
            .map(|(n, dir, _)| {
                serde_json::json!({
                    "id": id(n), "name": n,
                    "manifest_path": format!("/w/{dir}/{n}/Cargo.toml"),
                })
            })
            .collect();
        let nodes: Vec<Value> = pkgs
            .iter()
            .map(|(n, _, deps)| {
                let deps: Vec<Value> = deps
                    .iter()
                    .map(|d| {
                        // A dependency written `name:dev` is a dev-dependency.
                        let (d, kind) = d.split_once(':').map_or((*d, Value::Null), |(d, k)| (d, k.into()));
                        serde_json::json!({ "name": d, "pkg": id(d), "dep_kinds": [{ "kind": kind, "target": null }] })
                    })
                    .collect();
                serde_json::json!({ "id": id(n), "deps": deps })
            })
            .collect();
        let members: Vec<String> = pkgs
            .iter()
            .filter(|(_, dir, _)| ["crates", "apps", "spikes"].contains(dir))
            .map(|(n, _, _)| id(n))
            .collect();
        serde_json::json!({ "packages": packages, "workspace_members": members, "resolve": { "nodes": nodes } })
            .to_string()
    }

    fn violations(pkgs: &[(&str, &str, &[&str])]) -> Vec<(&'static str, String)> {
        parse(&metadata(pkgs))
            .unwrap()
            .check()
            .into_iter()
            .map(|v| (v.rule, v.package))
            .collect()
    }

    #[test]
    fn a_clean_workspace_passes() {
        let v = violations(&[
            ("voxel-core", "crates", &["thiserror"]),
            ("thiserror", "ext", &[]),
            ("voxel-mesh", "crates", &["voxel-core"]),
            (
                "voxel-render",
                "crates",
                &["wgpu", "voxel-mesh", "wgpu:dev"],
            ),
            ("wgpu", "ext", &[]),
            ("voxel-game", "apps", &["anyhow", "voxel-render", "winit"]),
            ("anyhow", "ext", &[]),
            ("winit", "ext", &[]),
        ]);
        assert!(v.is_empty(), "{v:?}");
    }

    #[test]
    fn only_the_owner_may_use_wgpu() {
        let v = violations(&[
            ("voxel-game", "apps", &["wgpu"]),
            ("wgpu", "ext", &[]),
            ("voxel-render", "crates", &["wgpu"]),
        ]);
        assert_eq!(v, [("owned-dependency", "voxel-game".to_string())]);
    }

    #[test]
    fn spikes_and_dev_dependencies_are_exempt() {
        let v = violations(&[
            ("spike-gpu", "spikes", &["wgpu", "anyhow"]),
            ("voxel-mesh", "crates", &["wgpu:dev"]),
            ("wgpu", "ext", &[]),
            ("anyhow", "ext", &[]),
        ]);
        assert!(v.is_empty(), "{v:?}");
    }

    #[test]
    fn libraries_do_not_use_anyhow() {
        let v = violations(&[
            ("voxel-content", "crates", &["anyhow"]),
            ("anyhow", "ext", &[]),
        ]);
        assert_eq!(v, [("anyhow-only-in-apps", "voxel-content".to_string())]);
    }

    #[test]
    fn pure_crates_cannot_reach_gpu_code_even_indirectly() {
        let v = violations(&[
            ("voxel-worldgen", "crates", &["voxel-content"]),
            ("voxel-content", "crates", &["voxel-render"]),
            ("voxel-render", "crates", &["wgpu"]),
            ("wgpu", "ext", &[]),
        ]);
        assert!(
            v.contains(&("pure-crates-stay-pure", "voxel-worldgen".to_string())),
            "{v:?}"
        );
    }

    #[test]
    fn server_never_reaches_render_and_client_never_reaches_worldgen() {
        let v = violations(&[
            ("voxel-server", "crates", &["voxel-sim"]),
            ("voxel-sim", "crates", &["voxel-render"]),
            ("voxel-render", "crates", &[]),
            ("voxel-client", "crates", &["voxel-worldgen"]),
            ("voxel-worldgen", "crates", &[]),
        ]);
        assert!(
            v.contains(&("server-has-no-presentation", "voxel-server".to_string())),
            "{v:?}"
        );
        assert!(
            v.contains(&("client-has-no-worldgen", "voxel-client".to_string())),
            "{v:?}"
        );
    }

    #[test]
    fn core_allows_only_thiserror() {
        let v = violations(&[
            ("voxel-core", "crates", &["thiserror", "glam"]),
            ("thiserror", "ext", &[]),
            ("glam", "ext", &[]),
        ]);
        assert_eq!(v, [("core-stays-light", "voxel-core".to_string())]);
    }

    /// The real workspace obeys its own rules.
    #[test]
    fn this_workspace_passes() {
        let manifest = concat!(env!("CARGO_MANIFEST_DIR"), "/../Cargo.toml");
        let out = std::process::Command::new("cargo")
            .args([
                "metadata",
                "--format-version",
                "1",
                "--manifest-path",
                manifest,
            ])
            .output()
            .expect("run cargo metadata");
        assert!(out.status.success());
        let graph = parse(&String::from_utf8(out.stdout).unwrap()).unwrap();
        let v = graph.check();
        assert!(
            v.is_empty(),
            "{}",
            v.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}
