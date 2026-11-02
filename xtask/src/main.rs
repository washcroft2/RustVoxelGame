//! Project automation (plan section 12). Run through the `cargo xtask` alias.

use std::path::PathBuf;
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, bail};

mod bundle;
mod perf;
mod rules;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "xtask", about = "Project automation commands")]
struct Cli {
    #[command(subcommand)]
    command: Task,
}

#[derive(Subcommand)]
enum Task {
    /// Format, clippy (warnings denied), tests and cargo-deny.
    Verify,
    /// Run the game benchmark and compare it with the targets in perf/targets.toml.
    Perf {
        /// Seconds of flythrough to measure.
        #[arg(long, default_value_t = 10.0)]
        seconds: f64,
    },
    /// Pack the built Phase 0 spike programs into a zip for another machine.
    Bundle {
        /// Directory holding the built programs.
        #[arg(long, default_value = "target/release")]
        bin_dir: PathBuf,
        /// Where to write the zip.
        #[arg(long, default_value = "dist")]
        out_dir: PathBuf,
        /// Architecture label for the zip name; defaults to this machine's.
        #[arg(long)]
        arch_label: Option<String>,
    },
}

/// One named check inside `verify`: a program plus its arguments.
struct Step {
    name: &'static str,
    program: &'static str,
    args: &'static [&'static str],
}

const VERIFY_STEPS: &[Step] = &[
    Step {
        name: "format",
        program: "cargo",
        args: &["fmt", "--all", "--check"],
    },
    Step {
        name: "clippy",
        program: "cargo",
        args: &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
    },
    Step {
        name: "test",
        program: "cargo",
        args: &["test", "--workspace"],
    },
    Step {
        name: "deny",
        program: "cargo",
        // All features, so optional dependencies (the profiling build's Tracy) are checked too.
        args: &["deny", "--all-features", "check"],
    },
    Step {
        name: "profile build",
        program: "cargo",
        args: &["check", "-p", "voxel-game", "--features", "profile"],
    },
];

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    match Cli::parse().command {
        Task::Verify => verify(),
        Task::Perf { seconds } => perf::run(&perf::Spec {
            seconds,
            out_dir: perf::default_out_dir().to_path_buf(),
        }),
        Task::Bundle {
            bin_dir,
            out_dir,
            arch_label,
        } => {
            let label = arch_label.unwrap_or_else(|| std::env::consts::ARCH.to_string());
            let zip = bundle::build(&bundle::Spec {
                bin_dir: &bin_dir,
                out_dir: &out_dir,
                os: std::env::consts::OS,
                arch_label: &label,
            })?;
            eprintln!("wrote {}", zip.display());
            Ok(())
        }
    }
}

/// Runs every step even after a failure, so one run reports everything that is wrong.
fn verify() -> Result<()> {
    let mut failed = Vec::new();
    for step in VERIFY_STEPS {
        eprintln!(
            "==> {}: {} {}",
            step.name,
            step.program,
            step.args.join(" ")
        );
        let status = Command::new(step.program)
            .args(step.args)
            .status()
            .with_context(|| {
                format!("could not start `{}` for step {}", step.program, step.name)
            })?;
        if !status.success() {
            failed.push(step.name);
        }
    }
    eprintln!("==> dependency rules: cargo metadata");
    match dependency_rules() {
        Ok(violations) if violations.is_empty() => {}
        Ok(violations) => {
            for v in &violations {
                eprintln!("rule violated: {v}");
            }
            failed.push("dependency rules");
        }
        Err(e) => {
            eprintln!("could not check dependency rules: {e:#}");
            failed.push("dependency rules");
        }
    }
    if failed.is_empty() {
        eprintln!("verify: all {} steps passed", VERIFY_STEPS.len() + 1);
        eprintln!("verify: not yet implemented: short fuzz runs (no fuzz targets exist yet)");
        Ok(())
    } else {
        bail!("verify failed: {}", failed.join(", "))
    }
}

/// Checks the crate graph against the architecture rules (see `rules.rs`).
fn dependency_rules() -> Result<Vec<rules::Violation>> {
    let out = Command::new("cargo")
        .args(["metadata", "--format-version", "1"])
        .output()
        .context("could not start `cargo metadata`")?;
    if !out.status.success() {
        bail!(
            "`cargo metadata` failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let json = String::from_utf8(out.stdout).context("cargo metadata output is not UTF-8")?;
    Ok(rules::parse(&json)?.check())
}
