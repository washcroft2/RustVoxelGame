//! Project automation (plan section 12). Run through the `cargo xtask` alias.

use std::path::PathBuf;
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, bail};

mod bundle;
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
        args: &["deny", "check"],
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
    if failed.is_empty() {
        eprintln!("verify: all {} steps passed", VERIFY_STEPS.len());
        eprintln!(
            "verify: not yet implemented: crate dependency rules, short fuzz runs (no such crates exist yet)"
        );
        Ok(())
    } else {
        bail!("verify failed: {}", failed.join(", "))
    }
}
