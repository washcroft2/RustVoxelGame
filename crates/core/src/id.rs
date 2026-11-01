use std::fmt;

/// Runtime block ID: an index into the block registry built when packs load.
/// Not stable across pack sets, so it is never saved or sent without a
/// name table. [`BlockId::AIR`] is always 0.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct BlockId(pub u16);

impl BlockId {
    pub const AIR: BlockId = BlockId(0);
}

/// How a block takes part in rendering and occlusion. The mesher and the
/// chunk's cached opacity masks are built on this, not on block names.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum BlockClass {
    /// Empty. Always [`BlockId::AIR`], and no other block.
    Air,
    /// Hides the faces of anything touching it.
    Opaque,
    /// Alpha-tested, like leaves.
    Cutout,
    /// Blended, like glass.
    Translucent,
}

/// Answers [`BlockClass`] for block IDs. Implemented by the content registry;
/// a closure works in tests.
pub trait BlockClasses {
    fn class(&self, id: BlockId) -> BlockClass;
}

impl<F: Fn(BlockId) -> BlockClass> BlockClasses for F {
    fn class(&self, id: BlockId) -> BlockClass {
        self(id)
    }
}

/// Runtime fluid ID. [`FluidId::NONE`] is always 0.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct FluidId(pub u16);

impl FluidId {
    pub const NONE: FluidId = FluidId(0);
}

/// Dimension ID, part of every chunk key.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct DimensionId(pub u16);

impl DimensionId {
    pub const OVERWORLD: DimensionId = DimensionId(0);
}

/// Simulation tick counter.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct Tick(pub u64);

#[derive(Debug, Clone, Eq, PartialEq, thiserror::Error)]
pub enum IdentifierError {
    #[error("identifier has no ':' between namespace and path")]
    MissingNamespace,
    #[error("identifier is empty or longer than {} bytes", Identifier::MAX_LEN)]
    BadLength,
    #[error("namespace or path is empty")]
    EmptyPart,
    #[error("invalid character {0:?} (allowed: a-z 0-9 _ - . and / in the path)")]
    BadChar(char),
}

/// A namespaced name such as `base:stone`. All content is addressed by these.
///
/// The namespace allows `a-z 0-9 _ - .`; the path also allows `/`.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct Identifier(String);

impl Identifier {
    pub const MAX_LEN: usize = 128;

    pub fn new(s: &str) -> Result<Self, IdentifierError> {
        if s.is_empty() || s.len() > Self::MAX_LEN {
            return Err(IdentifierError::BadLength);
        }
        let (ns, path) = s.split_once(':').ok_or(IdentifierError::MissingNamespace)?;
        if ns.is_empty() || path.is_empty() {
            return Err(IdentifierError::EmptyPart);
        }
        let ok = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || "_-.".contains(c);
        if let Some(c) = ns.chars().find(|&c| !ok(c)) {
            return Err(IdentifierError::BadChar(c));
        }
        if let Some(c) = path.chars().find(|&c| !ok(c) && c != '/') {
            return Err(IdentifierError::BadChar(c));
        }
        Ok(Self(s.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn namespace(&self) -> &str {
        self.0.split_once(':').map_or("", |(n, _)| n)
    }

    pub fn path(&self) -> &str {
        self.0.split_once(':').map_or("", |(_, p)| p)
    }
}

impl fmt::Display for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for Identifier {
    type Err = IdentifierError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}
