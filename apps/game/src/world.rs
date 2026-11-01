//! Loading the base pack and setting up the test world.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use voxel_content::{Pack, Registry, TextureImage, load_texture_images};
use voxel_worldgen::{Generator, NoiseParams, Terrain, TerrainBlocks};

/// Everything the game needs to start: content, textures and a terrain generator.
pub struct World {
    pub registry: Arc<Registry>,
    pub images: Vec<TextureImage>,
    pub generator: Generator,
}

/// Looks for `packs/base` in the working directory, beside the executable and in their parents,
/// so it works from a source checkout and from an unpacked download.
fn find_packs(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p.to_path_buf());
    }
    let mut starts = vec![std::env::current_dir().unwrap_or_default()];
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        starts.push(dir.to_path_buf());
    }
    for start in starts {
        for dir in start.ancestors().take(5) {
            let candidate = dir.join("packs");
            if candidate.join("base").join("pack.toml").is_file() {
                return Ok(candidate);
            }
        }
    }
    bail!("could not find a `packs/base` folder; pass --packs <folder>")
}

impl World {
    pub fn load(packs: Option<&Path>, seed: u64, flat: bool) -> Result<Self> {
        let dir = find_packs(packs)?.join("base");
        let pack = Pack::read_dir(&dir).with_context(|| format!("loading {}", dir.display()))?;
        let registry =
            Registry::build(std::slice::from_ref(&pack)).context("building the block registry")?;
        let images = load_texture_images(&registry, &[pack]).context("loading textures")?;
        let blocks = TerrainBlocks::from_registry(&registry)
            .context("the base pack lacks terrain blocks")?;
        let terrain = if flat {
            Terrain::Flat { surface: 64 }
        } else {
            Terrain::Noise(NoiseParams::default())
        };
        Ok(Self {
            registry: Arc::new(registry),
            images,
            generator: Generator::new(seed, terrain, blocks),
        })
    }

    /// Chunk rows that can hold terrain: from the world floor to just above the highest hill.
    pub fn vertical_chunks(&self) -> (i32, i32) {
        let top = match self.generator.terrain {
            Terrain::Flat { surface } => surface,
            Terrain::Noise(p) => (p.base + p.amplitude) as i32,
        };
        (0, top.div_euclid(32))
    }
}
