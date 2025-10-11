use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use image::imageops::flip_vertical_in_place;
use image::GenericImageView;
use wgpu::util::{DeviceExt, TextureDataOrder};

use crate::types::{
    ChannelBindings, ChannelSource, ChannelTextureKind, CHANNEL_COUNT, CUBEMAP_FACE_STEMS,
};

use super::context::SurfaceColorSpace;

pub(crate) const KEYBOARD_TEXTURE_WIDTH: u32 = 256;
pub(crate) const KEYBOARD_TEXTURE_HEIGHT: u32 = 3;
pub(crate) const KEYBOARD_BYTES_PER_PIXEL: u32 = 4;

pub(crate) struct ChannelResources {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub resolution: [f32; 4],
    keyboard: bool,
}

impl ChannelResources {
    pub(crate) fn is_keyboard(&self) -> bool {
        self.keyboard
    }

    pub(crate) fn update_keyboard(&self, queue: &wgpu::Queue, data: &[u8]) {
        if !self.keyboard {
            return;
        }

        let expected_len =
            (KEYBOARD_TEXTURE_WIDTH * KEYBOARD_TEXTURE_HEIGHT * KEYBOARD_BYTES_PER_PIXEL) as usize;
        if data.len() != expected_len {
            tracing::warn!(
                expected_len,
                actual_len = data.len(),
                "keyboard texture update ignored due to mismatched payload size"
            );
            return;
        }

        let bytes_per_row = KEYBOARD_TEXTURE_WIDTH * KEYBOARD_BYTES_PER_PIXEL;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(KEYBOARD_TEXTURE_HEIGHT),
            },
            wgpu::Extent3d {
                width: KEYBOARD_TEXTURE_WIDTH,
                height: KEYBOARD_TEXTURE_HEIGHT,
                depth_or_array_layers: 1,
            },
        );
    }
}

pub(crate) fn create_resources(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    bindings: &ChannelBindings,
    kinds: &[ChannelTextureKind; CHANNEL_COUNT],
    color_space: SurfaceColorSpace,
) -> Result<Vec<ChannelResources>> {
    let mut resources = Vec::with_capacity(CHANNEL_COUNT);
    for (index, (binding, kind)) in bindings.slots().iter().zip(kinds.iter()).enumerate() {
        let resource = match (binding, kind) {
            (Some(ChannelSource::Texture { path }), ChannelTextureKind::Texture2d) => {
                match load_texture_channel(device, queue, index, path, color_space) {
                    Ok(resource) => resource,
                    Err(error) => {
                        tracing::warn!(
                            channel = index,
                            path = %path.display(),
                            error = %error,
                            "failed to load texture channel; using placeholder"
                        );
                        create_placeholder_texture(device, queue, index as u32, color_space)?
                    }
                }
            }
            (Some(ChannelSource::Cubemap { directory }), ChannelTextureKind::Cubemap) => {
                match load_cubemap_channel(device, queue, index, directory, color_space) {
                    Ok(resource) => resource,
                    Err(error) => {
                        tracing::warn!(
                            channel = index,
                            dir = %directory.display(),
                            error = %error,
                            "failed to load cubemap channel; using placeholder"
                        );
                        create_placeholder_cubemap(device, queue, index as u32, color_space)?
                    }
                }
            }
            (Some(ChannelSource::Keyboard), ChannelTextureKind::Texture2d) => {
                create_keyboard_channel(device, queue, index as u32, color_space)?
            }
            (None, ChannelTextureKind::Texture2d) => {
                create_placeholder_texture(device, queue, index as u32, color_space)?
            }
            (None, ChannelTextureKind::Cubemap) => {
                create_placeholder_cubemap(device, queue, index as u32, color_space)?
            }
            (Some(ChannelSource::Texture { .. }), ChannelTextureKind::Cubemap)
            | (Some(ChannelSource::Cubemap { .. }), ChannelTextureKind::Texture2d)
            | (Some(ChannelSource::Keyboard), ChannelTextureKind::Cubemap) => {
                tracing::warn!(
                    channel = index,
                    "channel binding kind mismatch; using placeholder resource"
                );
                match kind {
                    ChannelTextureKind::Texture2d => {
                        create_placeholder_texture(device, queue, index as u32, color_space)?
                    }
                    ChannelTextureKind::Cubemap => {
                        create_placeholder_cubemap(device, queue, index as u32, color_space)?
                    }
                }
            }
        };
        resources.push(resource);
    }

    Ok(resources)
}

fn create_placeholder_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    index: u32,
    color_space: SurfaceColorSpace,
) -> Result<ChannelResources> {
    let data = [255u8, 255, 255, 25];
    let texture_format = match color_space {
        SurfaceColorSpace::Gamma => wgpu::TextureFormat::Rgba8Unorm,
        SurfaceColorSpace::Linear => wgpu::TextureFormat::Rgba8UnormSrgb,
    };
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some(&format!("placeholder channel texture #{index}")),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: texture_format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        TextureDataOrder::LayerMajor,
        &data,
    );

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    Ok(ChannelResources {
        texture,
        view,
        sampler,
        resolution: [1.0, 1.0, 1.0, 0.0],
        keyboard: false,
    })
}

fn create_placeholder_cubemap(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    index: u32,
    color_space: SurfaceColorSpace,
) -> Result<ChannelResources> {
    let face_size = 1;
    let mut data = Vec::with_capacity(6 * 4);
    for face in 0..6 {
        let value = if face % 2 == 0 { 255 } else { 0 };
        data.extend([value, value, value, 255]);
    }
    let texture_format = match color_space {
        SurfaceColorSpace::Gamma => wgpu::TextureFormat::Rgba8Unorm,
        SurfaceColorSpace::Linear => wgpu::TextureFormat::Rgba8UnormSrgb,
    };
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some(&format!("placeholder cubemap texture #{index}")),
            size: wgpu::Extent3d {
                width: face_size,
                height: face_size,
                depth_or_array_layers: 6,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: texture_format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        TextureDataOrder::LayerMajor,
        &data,
    );

    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some(&format!("placeholder cubemap view #{index}")),
        dimension: Some(wgpu::TextureViewDimension::Cube),
        array_layer_count: Some(6),
        ..Default::default()
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    Ok(ChannelResources {
        texture,
        view,
        sampler,
        resolution: [1.0, 1.0, 6.0, 0.0],
        keyboard: false,
    })
}

fn create_keyboard_channel(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    index: u32,
    color_space: SurfaceColorSpace,
) -> Result<ChannelResources> {
    let data = vec![
        0u8;
        (KEYBOARD_TEXTURE_WIDTH * KEYBOARD_TEXTURE_HEIGHT * KEYBOARD_BYTES_PER_PIXEL)
            as usize
    ];
    let texture_format = match color_space {
        SurfaceColorSpace::Gamma => wgpu::TextureFormat::Rgba8Unorm,
        SurfaceColorSpace::Linear => wgpu::TextureFormat::Rgba8UnormSrgb,
    };
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some(&format!("keyboard channel texture #{index}")),
            size: wgpu::Extent3d {
                width: KEYBOARD_TEXTURE_WIDTH,
                height: KEYBOARD_TEXTURE_HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: texture_format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        TextureDataOrder::LayerMajor,
        &data,
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        mipmap_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });

    Ok(ChannelResources {
        texture,
        view,
        sampler,
        resolution: [
            KEYBOARD_TEXTURE_WIDTH as f32,
            KEYBOARD_TEXTURE_HEIGHT as f32,
            1.0,
            0.0,
        ],
        keyboard: true,
    })
}

fn load_texture_channel(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    index: usize,
    path: &Path,
    color_space: SurfaceColorSpace,
) -> Result<ChannelResources> {
    let image = image::open(path).with_context(|| {
        format!(
            "failed to open texture for channel {} at {}",
            index,
            path.display()
        )
    })?;
    let (width, height) = image.dimensions();
    let mut rgba = image.to_rgba8();
    flip_vertical_in_place(&mut rgba);

    let texture_format = match color_space {
        SurfaceColorSpace::Gamma => wgpu::TextureFormat::Rgba8Unorm,
        SurfaceColorSpace::Linear => wgpu::TextureFormat::Rgba8UnormSrgb,
    };
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some(&format!("channel texture #{index}")),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: texture_format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        TextureDataOrder::LayerMajor,
        &rgba,
    );

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    Ok(ChannelResources {
        texture,
        view,
        sampler,
        resolution: [width as f32, height as f32, 1.0, 0.0],
        keyboard: false,
    })
}

fn load_cubemap_channel(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    index: usize,
    directory: &Path,
    color_space: SurfaceColorSpace,
) -> Result<ChannelResources> {
    if !directory.is_dir() {
        anyhow::bail!(
            "cubemap directory {} is missing or not a directory for channel {}",
            directory.display(),
            index
        );
    }

    let mut faces = Vec::with_capacity(CUBEMAP_FACE_STEMS.len());
    for face in CUBEMAP_FACE_STEMS {
        let face_path = find_cubemap_face(directory, face).ok_or_else(|| {
            anyhow!(
                "cubemap face '{face}' missing for channel {index} in {}",
                directory.display()
            )
        })?;
        let image = image::open(&face_path).with_context(|| {
            format!(
                "failed to open cubemap face '{}' for channel {} at {}",
                face,
                index,
                face_path.display()
            )
        })?;
        faces.push((face_path, image));
    }

    let (width, height) = faces[0].1.dimensions();
    if width != height {
        anyhow::bail!(
            "cubemap face {} is not square ({}x{})",
            faces[0].0.display(),
            width,
            height
        );
    }

    let mut data = Vec::with_capacity((width * height * 4 * 6) as usize);
    for (_, face) in faces {
        let mut rgba = face.to_rgba8();
        flip_vertical_in_place(&mut rgba);
        data.extend_from_slice(&rgba);
    }

    let texture_format = match color_space {
        SurfaceColorSpace::Gamma => wgpu::TextureFormat::Rgba8Unorm,
        SurfaceColorSpace::Linear => wgpu::TextureFormat::Rgba8UnormSrgb,
    };

    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some(&format!("cubemap texture #{index}")),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 6,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: texture_format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        TextureDataOrder::LayerMajor,
        &data,
    );

    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some(&format!("cubemap view #{index}")),
        dimension: Some(wgpu::TextureViewDimension::Cube),
        array_layer_count: Some(6),
        ..Default::default()
    });

    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    Ok(ChannelResources {
        texture,
        view,
        sampler,
        resolution: [width as f32, height as f32, 6.0, 0.0],
        keyboard: false,
    })
}

fn find_cubemap_face(directory: &Path, stem: &str) -> Option<PathBuf> {
    const EXTENSIONS: [&str; 4] = ["png", "jpg", "jpeg", "bmp"];
    for ext in EXTENSIONS {
        let candidate = directory.join(format!("{stem}.{ext}"));
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}
