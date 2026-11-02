//! The game client. Phase 1: a free camera over generated test terrain.

mod app;
mod bench;
mod overlay;
mod stream;
mod world;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use voxel_render::PresentPreference;

#[derive(Parser)]
#[command(about = "Voxel game (Phase 1 test world)")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// Folder containing `base/`; found automatically from a source checkout or a download.
    #[arg(long, global = true)]
    packs: Option<PathBuf>,
    #[arg(long, default_value_t = 1, global = true)]
    seed: u64,
    /// A flat plain instead of hills.
    #[arg(long, global = true)]
    flat: bool,
    /// Draw distance in chunks (32 blocks each).
    #[arg(long, default_value_t = 16, global = true)]
    radius: i32,
    #[arg(long, value_enum, default_value_t = Present::LowLatency)]
    present: Present,
}

#[derive(Clone, Copy, ValueEnum)]
enum Present {
    Vsync,
    LowLatency,
    Uncapped,
}

#[derive(Subcommand)]
enum Command {
    /// Fly a fixed path with no window and report frame times.
    Bench {
        #[arg(long, default_value_t = 10.0)]
        seconds: f64,
        #[arg(long, default_value_t = 1920)]
        width: u32,
        #[arg(long, default_value_t = 1080)]
        height: u32,
        /// Blocks per second.
        #[arg(long, default_value_t = 40.0)]
        speed: f64,
        /// Save the last frame as a PNG.
        #[arg(long)]
        screenshot: Option<PathBuf>,
        /// Draw the F3 overlay on the saved frame.
        #[arg(long)]
        overlay: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let world = world::World::load(cli.packs.as_deref(), cli.seed, cli.flat)?;
    match cli.command {
        Some(Command::Bench {
            seconds,
            width,
            height,
            speed,
            screenshot,
            overlay,
        }) => bench::run(
            world,
            bench::BenchOptions {
                radius: cli.radius,
                seconds,
                size: (width, height),
                speed,
                screenshot,
                overlay,
            },
        ),
        None => app::run(
            world,
            app::Options {
                radius: cli.radius,
                present: match cli.present {
                    Present::Vsync => PresentPreference::Vsync,
                    Present::LowLatency => PresentPreference::LowLatency,
                    Present::Uncapped => PresentPreference::Uncapped,
                },
            },
        ),
    }
}
