use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use voxel_core::Identifier;

use crate::ContentError;

/// Longest pack file read, so a hostile pack can't make the loader allocate without bound.
const MAX_FILE_BYTES: usize = 64 * 1024;
/// Largest font file read.
const MAX_FONT_BYTES: u64 = 4 << 20;

/// `pack.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackManifest {
    /// The namespace for everything this pack defines.
    pub id: String,
    pub version: String,
}

/// A block definition file: `blocks/<path>.toml`, named `<pack id>:<path>`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockSource {
    /// `opaque`, `cutout` or `translucent`.
    #[serde(default = "default_layer")]
    pub layer: String,
    pub textures: TextureSource,
}

fn default_layer() -> String {
    "opaque".to_owned()
}

/// Either one texture for every face, or top/bottom/side, or all six faces.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextureSource {
    pub all: Option<String>,
    pub top: Option<String>,
    pub bottom: Option<String>,
    pub side: Option<String>,
    pub pos_x: Option<String>,
    pub neg_x: Option<String>,
    pub pos_y: Option<String>,
    pub neg_y: Option<String>,
    pub pos_z: Option<String>,
    pub neg_z: Option<String>,
}

/// A pack read into memory. Parsing is separate from file access so it can be tested without files.
#[derive(Debug, Clone)]
pub struct Pack {
    pub manifest: PackManifest,
    /// Block files as `(path without extension, parsed)`, sorted by path.
    pub blocks: Vec<(String, BlockSource)>,
    /// Texture paths without the `.png` extension, sorted.
    pub textures: Vec<String>,
    /// Font files under `fonts/`, as paths without the `.ttf` extension, sorted.
    pub fonts: Vec<String>,
    /// Where the pack was read from; `None` for packs built in memory.
    pub root: Option<PathBuf>,
}

impl Pack {
    /// Parses a pack from file contents. `blocks` is `(relative path without extension, TOML text)`.
    pub fn from_parts(
        manifest: &str,
        blocks: &[(String, String)],
        textures: Vec<String>,
    ) -> Result<Self, ContentError> {
        let manifest: PackManifest = parse("pack.toml", manifest)?;
        // The pack id becomes the namespace, so it must be a valid one.
        Identifier::new(&format!("{}:x", manifest.id)).map_err(|source| ContentError::Name {
            path: "pack.toml".into(),
            source,
        })?;
        let mut parsed = Vec::with_capacity(blocks.len());
        for (path, text) in blocks {
            let full = format!("blocks/{path}.toml");
            Identifier::new(&format!("{}:{path}", manifest.id)).map_err(|source| {
                ContentError::Name {
                    path: full.clone(),
                    source,
                }
            })?;
            parsed.push((path.clone(), parse(&full, text)?));
        }
        parsed.sort_by(|a, b| a.0.cmp(&b.0));
        let mut textures = textures;
        textures.sort();
        Ok(Self {
            manifest,
            blocks: parsed,
            textures,
            fonts: Vec::new(),
            root: None,
        })
    }

    /// Reads a pack directory: `pack.toml`, `blocks/**/*.toml` and `textures/**/*.png`.
    pub fn read_dir(dir: &Path) -> Result<Self, ContentError> {
        let manifest = read_text(&dir.join("pack.toml"))?;
        let mut blocks = Vec::new();
        for rel in list(&dir.join("blocks"), "toml")? {
            blocks.push((
                rel.clone(),
                read_text(&dir.join("blocks").join(format!("{rel}.toml")))?,
            ));
        }
        let textures = list(&dir.join("textures"), "png")?;
        let mut pack = Self::from_parts(&manifest, &blocks, textures)?;
        pack.fonts = list(&dir.join("fonts"), "ttf")?;
        pack.root = Some(dir.to_path_buf());
        Ok(pack)
    }

    /// Reads one of the pack's fonts by name (as listed in [`Pack::fonts`]).
    pub fn read_font(&self, name: &str) -> Result<Vec<u8>, ContentError> {
        let path = format!("fonts/{name}.ttf");
        let err = |message: String| ContentError::Invalid {
            path: path.clone(),
            message,
        };
        // Only names found by listing the folder are accepted, so `name` can't climb out of it.
        if !self.fonts.iter().any(|f| f == name) {
            return Err(err("no such font in this pack".into()));
        }
        let root = self
            .root
            .as_ref()
            .ok_or_else(|| err("the pack was not read from a directory".into()))?;
        let file = root.join(&path);
        let len = fs::metadata(&file).map_err(|e| err(e.to_string()))?.len();
        if len > MAX_FONT_BYTES {
            return Err(ContentError::TooLarge {
                path,
                limit: MAX_FONT_BYTES as usize,
            });
        }
        fs::read(&file).map_err(|e| err(e.to_string()))
    }
}

fn parse<T: for<'de> Deserialize<'de>>(path: &str, text: &str) -> Result<T, ContentError> {
    if text.len() > MAX_FILE_BYTES {
        return Err(ContentError::TooLarge {
            path: path.into(),
            limit: MAX_FILE_BYTES,
        });
    }
    toml::from_str(text).map_err(|e| ContentError::Parse {
        path: path.into(),
        message: e.to_string(),
    })
}

fn read_text(path: &Path) -> Result<String, ContentError> {
    let io = |source| ContentError::Io {
        path: path.display().to_string(),
        source,
    };
    let len = fs::metadata(path).map_err(io)?.len();
    if len > MAX_FILE_BYTES as u64 {
        return Err(ContentError::TooLarge {
            path: path.display().to_string(),
            limit: MAX_FILE_BYTES,
        });
    }
    fs::read_to_string(path).map_err(io)
}

/// Relative paths (with `/` separators, extension removed) of files under `dir` with `ext`.
/// A missing directory is an empty list.
fn list(dir: &Path, ext: &str) -> Result<Vec<String>, ContentError> {
    let mut out = Vec::new();
    if dir.is_dir() {
        walk(dir, "", ext, &mut out)?;
    }
    out.sort();
    Ok(out)
}

fn walk(dir: &Path, prefix: &str, ext: &str, out: &mut Vec<String>) -> Result<(), ContentError> {
    let io = |source| ContentError::Io {
        path: dir.display().to_string(),
        source,
    };
    for entry in fs::read_dir(dir).map_err(io)? {
        let entry = entry.map_err(io)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let ty = entry.file_type().map_err(io)?;
        if ty.is_dir() {
            walk(&entry.path(), &format!("{prefix}{name}/"), ext, out)?;
        } else if ty.is_file()
            && let Some(stem) = name.strip_suffix(&format!(".{ext}"))
        {
            out.push(format!("{prefix}{stem}"));
        }
    }
    Ok(())
}
