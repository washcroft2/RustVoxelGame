//! Project automation (plan section 12). Run through the `cargo xtask` alias.

use std::process::{Command, ExitCode};

use anyhow::{Context, Result, bail};
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
