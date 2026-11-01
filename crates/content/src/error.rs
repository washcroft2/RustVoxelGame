use voxel_core::IdentifierError;

#[derive(Debug, thiserror::Error)]
pub enum ContentError {
    #[error("{path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("{path}: {message}")]
    Parse { path: String, message: String },
    #[error("{path}: invalid name: {source}")]
    Name {
        path: String,
        source: IdentifierError,
    },
    #[error("{path}: file is larger than {limit} bytes")]
    TooLarge { path: String, limit: usize },
    #[error("{path}: {message}")]
    Invalid { path: String, message: String },
    #[error("duplicate block {0}")]
    DuplicateBlock(String),
    #[error("texture {texture} (used by {block}) is not in any loaded pack")]
    MissingTexture { block: String, texture: String },
    #[error("expected exactly one air block, found {0}")]
    AirCount(usize),
    #[error("more than {0} blocks")]
    TooManyBlocks(usize),
    #[error("more than {0} textures")]
    TooManyTextures(usize),
}
