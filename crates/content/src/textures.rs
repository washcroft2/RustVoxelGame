use std::fs;

use crate::{ContentError, Pack, Registry};

/// Largest texture file read, and largest edge accepted.
const MAX_FILE_BYTES: u64 = 1 << 20;
const MAX_EDGE: u32 = 1024;

/// One decoded texture: tightly packed RGBA8, row by row.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct TextureImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Decodes every texture the registry lists, in [`Registry::textures`] order.
///
/// `packs` must be the packs the registry was built from, read from disk (so
/// they have a root). All textures must be the same size, because the
/// renderer stores them as the layers of one array.
pub fn load_texture_images(
    registry: &Registry,
    packs: &[Pack],
) -> Result<Vec<TextureImage>, ContentError> {
    let mut out: Vec<TextureImage> = Vec::with_capacity(registry.textures().len());
    for name in registry.textures() {
        let err = |message: String| ContentError::Texture {
            texture: name.to_string(),
            message,
        };
        let pack = packs
            .iter()
            .find(|p| p.manifest.id == name.namespace())
            .ok_or_else(|| err("its pack was not given".into()))?;
        let root = pack
            .root
            .as_ref()
            .ok_or_else(|| err("its pack was not read from a directory".into()))?;
        let path = root.join("textures").join(format!("{}.png", name.path()));
        let len = fs::metadata(&path)
            .map_err(|e| err(format!("{}: {e}", path.display())))?
            .len();
        if len > MAX_FILE_BYTES {
            return Err(err(format!("file is larger than {MAX_FILE_BYTES} bytes")));
        }
        let bytes = fs::read(&path).map_err(|e| err(format!("{}: {e}", path.display())))?;
        // Read the size from the header first, so a hostile file can't ask for a huge buffer.
        let reader =
            image::ImageReader::with_format(std::io::Cursor::new(&bytes), image::ImageFormat::Png);
        let (w, h) = reader.into_dimensions().map_err(|e| err(e.to_string()))?;
        if w == 0 || h == 0 || w > MAX_EDGE || h > MAX_EDGE {
            return Err(err(format!("size {w}x{h} is outside 1..={MAX_EDGE}")));
        }
        let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
            .map_err(|e| err(e.to_string()))?
            .into_rgba8();
        let tex = TextureImage {
            width: w,
            height: h,
            rgba: img.into_raw(),
        };
        if let Some(first) = out.first()
            && (first.width, first.height) != (w, h)
        {
            return Err(err(format!(
                "size {w}x{h} differs from the first texture's {}x{}",
                first.width, first.height
            )));
        }
        out.push(tex);
    }
    Ok(out)
}
