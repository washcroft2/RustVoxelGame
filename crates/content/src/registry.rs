use std::collections::HashMap;

use voxel_core::{BlockClass, BlockClasses, BlockId, Face, Identifier};

use crate::pack::TextureSource;
use crate::{ContentError, Pack};

/// Block IDs are `u16`, so this is the most a registry can hold.
/// The name the engine gives the empty block.
const AIR_NAME: &str = "base:air";
const MAX_BLOCKS: usize = u16::MAX as usize;
/// Quads carry a 16-bit material.
const MAX_TEXTURES: usize = u16::MAX as usize;

/// Index into [`Registry::textures`]. Quads use it as their material.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct TextureId(pub u16);

/// Which mesh group a block's faces go into.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum RenderLayer {
    Opaque,
    /// Alpha-tested: leaves, plants.
    Cutout,
    /// Blended: glass, water.
    Translucent,
}

#[derive(Clone, Debug)]
pub struct BlockDef {
    pub name: Identifier,
    pub layer: RenderLayer,
    /// Hides the faces of neighbouring blocks that touch it.
    pub opaque: bool,
    /// Texture per face, in [`Face`] order. `None` for air.
    pub faces: Option<[TextureId; 6]>,
}

/// Blocks and textures from every loaded pack, with runtime IDs assigned.
///
/// Air is built into the engine as `base:air` with [`BlockId::AIR`]; packs can't define it.
/// Other blocks follow in pack order, then path order.
#[derive(Debug)]
pub struct Registry {
    blocks: Vec<BlockDef>,
    by_name: HashMap<Identifier, BlockId>,
    textures: Vec<Identifier>,
}

impl BlockClasses for Registry {
    /// Unknown IDs count as air.
    fn class(&self, id: BlockId) -> BlockClass {
        match self.block(id) {
            None => BlockClass::Air,
            Some(_) if id == BlockId::AIR => BlockClass::Air,
            Some(b) => match b.layer {
                RenderLayer::Opaque => BlockClass::Opaque,
                RenderLayer::Cutout => BlockClass::Cutout,
                RenderLayer::Translucent => BlockClass::Translucent,
            },
        }
    }
}

impl Registry {
    /// Builds a registry from packs in load order. Later packs may use textures from earlier ones.
    pub fn build(packs: &[Pack]) -> Result<Self, ContentError> {
        let mut texture_ids: HashMap<Identifier, TextureId> = HashMap::new();
        let mut textures = Vec::new();
        for pack in packs {
            for t in &pack.textures {
                let name = qualified(&pack.manifest.id, t, "textures")?;
                if textures.len() >= MAX_TEXTURES {
                    return Err(ContentError::TooManyTextures(MAX_TEXTURES));
                }
                texture_ids.insert(name.clone(), TextureId(textures.len() as u16));
                textures.push(name);
            }
        }

        let mut blocks = vec![BlockDef {
            name: Identifier::new(AIR_NAME).expect("AIR_NAME is a valid identifier"),
            layer: RenderLayer::Opaque,
            opaque: false,
            faces: None,
        }];
        for pack in packs {
            for (path, src) in &pack.blocks {
                let file = format!("{}/blocks/{path}.toml", pack.manifest.id);
                let name = qualified(&pack.manifest.id, path, &file)?;
                let layer = match src.layer.as_str() {
                    "opaque" => RenderLayer::Opaque,
                    "cutout" => RenderLayer::Cutout,
                    "translucent" => RenderLayer::Translucent,
                    other => {
                        return Err(ContentError::Invalid {
                            path: file,
                            message: format!("unknown layer {other:?}"),
                        });
                    }
                };
                let faces =
                    resolve_faces(&src.textures, &pack.manifest.id, &name, &file, &texture_ids)?;
                blocks.push(BlockDef {
                    name,
                    layer,
                    opaque: layer == RenderLayer::Opaque,
                    faces: Some(faces),
                });
            }
        }
        if blocks.len() > MAX_BLOCKS {
            return Err(ContentError::TooManyBlocks(MAX_BLOCKS));
        }
        let mut by_name = HashMap::with_capacity(blocks.len());
        for (i, b) in blocks.iter().enumerate() {
            if by_name.insert(b.name.clone(), BlockId(i as u16)).is_some() {
                return Err(ContentError::DuplicateBlock(b.name.to_string()));
            }
        }
        Ok(Self {
            blocks,
            by_name,
            textures,
        })
    }

    pub fn block(&self, id: BlockId) -> Option<&BlockDef> {
        self.blocks.get(id.0 as usize)
    }

    pub fn block_id(&self, name: &Identifier) -> Option<BlockId> {
        self.by_name.get(name).copied()
    }

    pub fn blocks(&self) -> &[BlockDef] {
        &self.blocks
    }

    /// Texture names in [`TextureId`] order.
    pub fn textures(&self) -> &[Identifier] {
        &self.textures
    }

    /// Whether the block hides touching faces. Unknown IDs count as not opaque.
    pub fn is_opaque(&self, id: BlockId) -> bool {
        self.block(id).is_some_and(|b| b.opaque)
    }

    /// The texture drawn on `face` of `id`, or `None` for air and unknown IDs.
    pub fn face_texture(&self, id: BlockId, face: Face) -> Option<TextureId> {
        self.block(id)?.faces.map(|f| f[face.index()])
    }
}

fn qualified(namespace: &str, path: &str, file: &str) -> Result<Identifier, ContentError> {
    Identifier::new(&format!("{namespace}:{path}")).map_err(|source| ContentError::Name {
        path: file.to_owned(),
        source,
    })
}

fn resolve_faces(
    t: &TextureSource,
    namespace: &str,
    block: &Identifier,
    file: &str,
    ids: &HashMap<Identifier, TextureId>,
) -> Result<[TextureId; 6], ContentError> {
    let lookup = |name: &str| -> Result<TextureId, ContentError> {
        // A bare name belongs to this pack; `other:name` reaches into another.
        let full = if name.contains(':') {
            name.to_owned()
        } else {
            format!("{namespace}:{name}")
        };
        let id = qualified_str(&full, file)?;
        ids.get(&id).copied().ok_or(ContentError::MissingTexture {
            block: block.to_string(),
            texture: full,
        })
    };
    let pick = |specific: &Option<String>, group: &Option<String>| {
        specific
            .as_ref()
            .or(group.as_ref())
            .or(t.all.as_ref())
            .ok_or_else(|| ContentError::Invalid {
                path: file.to_owned(),
                message: "textures must give `all`, or cover every face".into(),
            })
            .and_then(|n| lookup(n))
    };
    Ok([
        pick(&t.pos_x, &t.side)?,
        pick(&t.neg_x, &t.side)?,
        pick(&t.pos_y, &t.top)?,
        pick(&t.neg_y, &t.bottom)?,
        pick(&t.pos_z, &t.side)?,
        pick(&t.neg_z, &t.side)?,
    ])
}

fn qualified_str(full: &str, file: &str) -> Result<Identifier, ContentError> {
    Identifier::new(full).map_err(|source| ContentError::Name {
        path: file.to_owned(),
        source,
    })
}
