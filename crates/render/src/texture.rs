use voxel_content::TextureImage;
use wgpu::util::DeviceExt;

use crate::RenderError;

/// Mip levels for a square-ish texture of this size.
pub fn mip_count(width: u32, height: u32) -> u32 {
    32 - width.max(height).max(1).leading_zeros()
}

/// Averages 2x2 blocks. Works on the stored (sRGB-encoded) bytes, which is
/// slightly dark for gradients but fine for blocky textures.
fn downsample(src: &[u8], w: u32, h: u32) -> (Vec<u8>, u32, u32) {
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    let mut out = vec![0u8; (nw * nh * 4) as usize];
    for y in 0..nh {
        for x in 0..nw {
            for c in 0..4 {
                let mut sum = 0u32;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let sx = (x * 2 + dx).min(w - 1);
                    let sy = (y * 2 + dy).min(h - 1);
                    sum += u32::from(src[((sy * w + sx) * 4 + c) as usize]);
                }
                out[((y * nw + x) * 4 + c) as usize] = ((sum + 2) / 4) as u8;
            }
        }
    }
    (out, nw, nh)
}

/// All block textures as one 2D array with mipmaps (tiling quads and mipmaps
/// bleeding are why this is an array and not an atlas).
pub fn create_texture_array(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    images: &[TextureImage],
    max_layers: u32,
) -> Result<wgpu::TextureView, RenderError> {
    let fallback = TextureImage {
        width: 1,
        height: 1,
        rgba: vec![255, 0, 255, 255],
    };
    let images = if images.is_empty() {
        std::slice::from_ref(&fallback)
    } else {
        images
    };
    let (w, h) = (images[0].width, images[0].height);
    if images.iter().any(|i| (i.width, i.height) != (w, h)) {
        return Err(RenderError::Textures("textures differ in size".into()));
    }
    if images.iter().any(|i| i.rgba.len() != (w * h * 4) as usize) {
        return Err(RenderError::Textures(
            "pixel data does not match the size".into(),
        ));
    }
    let layers = images.len() as u32;
    if layers > max_layers {
        return Err(RenderError::Textures(format!(
            "{layers} textures exceed the adapter's {max_layers} array layers"
        )));
    }
    let mips = mip_count(w, h);
    // Layer-major data, each layer's mips in order, as `create_texture_with_data` expects.
    let mut data = Vec::new();
    for img in images {
        let (mut level, mut lw, mut lh) = (img.rgba.clone(), w, h);
        for m in 0..mips {
            data.extend_from_slice(&level);
            if m + 1 < mips {
                (level, lw, lh) = downsample(&level, lw, lh);
            }
        }
    }
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("block textures"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: layers,
            },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &data,
    );
    Ok(texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    }))
}
