//! `cargo xtask perf`: runs the game's benchmark and compares the results with the targets in
//! `perf/targets.toml` (plan section 2). Regression baselines are not part of this yet.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::Value;

/// One comparison between a measurement and a target.
#[derive(Debug, PartialEq)]
pub struct Check {
    pub name: &'static str,
    pub measured: f64,
    pub limit: f64,
    /// True when the measured value must be at least the limit; false when at most.
    pub at_least: bool,
    pub enforced: bool,
    pub unit: &'static str,
}

impl Check {
    pub fn passed(&self) -> bool {
        if self.at_least {
            self.measured >= self.limit
        } else {
            self.measured <= self.limit
        }
    }
}

pub struct Spec {
    pub seconds: f64,
    pub out_dir: PathBuf,
}

/// Runs the benchmark, prints the table and writes `report.json`. Fails if an enforced target is missed.
pub fn run(spec: &Spec) -> Result<()> {
    let targets_text = std::fs::read_to_string("perf/targets.toml")
        .context("reading perf/targets.toml (run from the repository root)")?;
    let targets: toml::Table = targets_text
        .parse()
        .context("perf/targets.toml is not valid TOML")?;
    let num = |table: &str, key: &str| -> Result<f64> {
        targets
            .get(table)
            .and_then(|t| t.get(key))
            .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
            .with_context(|| format!("perf/targets.toml: missing {table}.{key}"))
    };
    let (radius, width, height) = (
        num("flythrough", "radius")?,
        num("flythrough", "width")?,
        num("flythrough", "height")?,
    );

    std::fs::create_dir_all(&spec.out_dir)?;
    let json_path = spec.out_dir.join("game.json");
    let status = Command::new("cargo")
        .args(["run", "--release", "-p", "voxel-game", "--", "--radius"])
        .arg(format!("{radius}"))
        .args(["bench", "--seconds"])
        .arg(format!("{}", spec.seconds))
        .args([
            "--width",
            &format!("{width}"),
            "--height",
            &format!("{height}"),
            "--json",
        ])
        .arg(&json_path)
        .status()
        .context("could not start the benchmark")?;
    if !status.success() {
        bail!("the benchmark failed");
    }
    let result: Value = serde_json::from_str(&std::fs::read_to_string(&json_path)?)
        .context("benchmark output is not JSON")?;

    let checks = evaluate(&targets, &result)?;
    print_table(&result, &checks);
    let report = serde_json::json!({
        "measurements": result,
        "checks": checks.iter().map(|c| serde_json::json!({
            "name": c.name, "measured": c.measured, "limit": c.limit,
            "at_least": c.at_least, "enforced": c.enforced, "passed": c.passed(), "unit": c.unit,
        })).collect::<Vec<_>>(),
    });
    let report_path = spec.out_dir.join("report.json");
    std::fs::write(&report_path, serde_json::to_string_pretty(&report)?)?;
    eprintln!("report: {}", report_path.display());

    let failed: Vec<&str> = checks
        .iter()
        .filter(|c| c.enforced && !c.passed())
        .map(|c| c.name)
        .collect();
    if failed.is_empty() {
        Ok(())
    } else {
        bail!("perf targets missed: {}", failed.join(", "))
    }
}

/// Compares benchmark results with the targets table.
pub fn evaluate(targets: &toml::Table, result: &Value) -> Result<Vec<Check>> {
    let target = |table: &str, key: &str| -> Result<f64> {
        targets
            .get(table)
            .and_then(|t| t.get(key))
            .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
            .with_context(|| format!("targets: missing {table}.{key}"))
    };
    let measured = |path: &[&str]| -> Result<f64> {
        let mut v = result;
        for p in path {
            v = &v[*p];
        }
        v.as_f64()
            .with_context(|| format!("benchmark output: missing {}", path.join(".")))
    };
    let mean_ms = measured(&["flight", "mean_ms"])?;
    let (median_ms, p99_ms) = (
        measured(&["flight", "median_ms"])?,
        measured(&["flight", "p99_ms"])?,
    );
    let enforced = targets
        .get("frame_time")
        .and_then(|t| t.get("enforced"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(true);
    Ok(vec![
        Check {
            name: "frame rate (mean over the flythrough)",
            measured: 1e3 / mean_ms.max(1e-6),
            limit: target("flythrough", "min_fps")?,
            at_least: true,
            enforced: true,
            unit: "fps",
        },
        Check {
            name: "remesh median per core",
            measured: measured(&["remesh", "median_us"])?,
            limit: target("remesh", "max_median_us")?,
            at_least: false,
            enforced: true,
            unit: "us",
        },
        Check {
            name: "frame time p99 / median",
            measured: p99_ms / median_ms.max(1e-6),
            limit: target("frame_time", "max_p99_over_median")?,
            at_least: false,
            enforced,
            unit: "x",
        },
    ])
}

fn print_table(result: &Value, checks: &[Check]) {
    eprintln!(
        "\nperf on {}",
        result["gpu"].as_str().unwrap_or("unknown GPU")
    );
    for c in checks {
        let verdict = match (c.passed(), c.enforced) {
            (true, _) => "pass",
            (false, true) => "FAIL",
            (false, false) => "not enforced (missed)",
        };
        eprintln!(
            "  {:<40} {:>9.2} {} (target {} {:.2})  {verdict}",
            c.name,
            c.measured,
            c.unit,
            if c.at_least { ">=" } else { "<=" },
            c.limit
        );
    }
}

/// Where `perf` writes by default.
pub fn default_out_dir() -> &'static Path {
    Path::new("target/perf")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn targets(enforced: bool) -> toml::Table {
        format!(
            "[flythrough]\nmin_fps = 144\n[remesh]\nmax_median_us = 100\n[frame_time]\nmax_p99_over_median = 1.5\nenforced = {enforced}\n"
        )
        .parse()
        .unwrap()
    }

    fn result(mean_ms: f64, median_ms: f64, p99_ms: f64, remesh_us: f64) -> Value {
        serde_json::json!({
            "flight": { "mean_ms": mean_ms, "median_ms": median_ms, "p99_ms": p99_ms },
            "remesh": { "median_us": remesh_us },
        })
    }

    fn outcome(checks: &[Check]) -> Vec<(bool, bool)> {
        checks.iter().map(|c| (c.passed(), c.enforced)).collect()
    }

    #[test]
    fn good_numbers_pass() {
        let c = evaluate(&targets(true), &result(5.0, 5.0, 6.0, 60.0)).unwrap();
        assert_eq!(outcome(&c), [(true, true), (true, true), (true, true)]);
    }

    #[test]
    fn slow_frames_and_slow_remesh_fail() {
        let c = evaluate(&targets(true), &result(8.0, 5.0, 6.0, 150.0)).unwrap();
        assert_eq!(outcome(&c), [(false, true), (false, true), (true, true)]);
    }

    #[test]
    fn the_spread_target_is_reported_but_does_not_fail_while_unenforced() {
        let c = evaluate(&targets(false), &result(0.6, 0.4, 2.7, 50.0)).unwrap();
        assert_eq!(outcome(&c), [(true, true), (true, true), (false, false)]);
        let c = evaluate(&targets(true), &result(0.6, 0.4, 2.7, 50.0)).unwrap();
        assert_eq!(outcome(&c)[2], (false, true));
    }

    #[test]
    fn missing_data_is_an_error_not_a_pass() {
        assert!(evaluate(&targets(true), &serde_json::json!({})).is_err());
        let incomplete: toml::Table = "[flythrough]\nmin_fps = 144\n".parse().unwrap();
        assert!(evaluate(&incomplete, &result(5.0, 5.0, 6.0, 60.0)).is_err());
    }

    #[test]
    fn the_committed_targets_file_has_every_number_evaluate_needs() {
        let text =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../perf/targets.toml"))
                .unwrap();
        let t: toml::Table = text.parse().unwrap();
        assert!(evaluate(&t, &result(5.0, 5.0, 6.0, 60.0)).is_ok());
    }
}
