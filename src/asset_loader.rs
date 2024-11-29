use core::str;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Read, Write},
    path::PathBuf,
};

use anyhow::anyhow;
use bevy::{
    asset::{AssetLoader, AsyncReadExt},
    log::{debug, info},
    math::{UVec2, Vec3},
    prelude::{AlphaMode, Image},
    render::render_asset::RenderAssetUsages,
};
use image::{DynamicImage, ImageBuffer};
use serde::{Deserialize, Serialize};
use wgpu::{Extent3d, TextureFormat};

use crate::{
    oct_coords::GridMode,
    render::{Imposter, ImposterData, INDEXED_FLAG, RENDER_MULTISAMPLE_FLAG, USE_SOURCE_UV_Y_FLAG},
};

pub struct ImposterLoader;

#[derive(Serialize, Deserialize, Default)]
pub enum ImposterVertexMode {
    // should be used with a Rectangle / Plane3d(normal: Vec3::Z) mesh
    #[default]
    Billboard,
    // can be used with any mesh
    NoBillboard,
}

#[derive(Serialize, Deserialize)]
pub struct ImposterLoaderSettings {
    // billboard / no billboard
    pub vertex_mode: ImposterVertexMode,
    // smooth sample the material texture
    pub multisample: bool,
    // take uv y coords from the source mesh
    pub use_source_uv_y: bool,
    // additional multiplier
    pub alpha: f32,
    // roughly alpha mode. 0 -> Blend, 1 -> Opaque, (0-1) -> Mask
    // if you need more control you can modify the loaded asset (we can't put actual alpha mode here because it doesn't serialize)
    pub alpha_blend: f32,
}

impl Default for ImposterLoaderSettings {
    fn default() -> Self {
        Self {
            vertex_mode: Default::default(),
            multisample: Default::default(),
            use_source_uv_y: Default::default(),
            alpha: 1.0,
            alpha_blend: 0.0,
        }
    }
}

impl AssetLoader for ImposterLoader {
    type Asset = Imposter;

    type Settings = ImposterLoaderSettings;

    type Error = anyhow::Error;

    fn load<'a>(
        &'a self,
        reader: &'a mut bevy::asset::io::Reader,
        load_settings: &'a Self::Settings,
        load_context: &'a mut bevy::asset::LoadContext,
    ) -> impl bevy::utils::ConditionalSendFuture<Output = Result<Self::Asset, Self::Error>> {
        Box::pin(async move {
            let mut bytes = Vec::new();
            reader
                .read_to_end(&mut bytes)
                .await
                .map_err(|_| anyhow!("read failed"))?;
            let cursor = Cursor::new(&bytes[..]);
            let mut zip = zip::ZipArchive::new(cursor)?;
            let settings = zip
                .by_name("settings.txt")?
                .bytes()
                .collect::<Result<Vec<_>, _>>()?;
            let mut parts = str::from_utf8(&settings)?.split(' ');
            let (
                Some(grid_size),
                Some(scale),
                Some(mode),
                Some(base_tile_size),
                Some(packed_offset_x),
                Some(packed_offset_y),
                Some(packed_size_x),
                Some(packed_size_y),
            ) = (
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
            )
            else {
                anyhow::bail!("bad format for settings: `{:?}`", settings);
            };
            let grid_size = grid_size.parse()?;
            let scale = scale.parse()?;
            let base_tile_size = base_tile_size.parse()?;
            let packed_tile_offset = UVec2::new(packed_offset_x.parse()?, packed_offset_y.parse()?);
            let packed_tile_size = UVec2::new(packed_size_x.parse()?, packed_size_y.parse()?);

            let is_indexed = zip.file_names().any(|n| n == "pixels.png");
            let (pixels_image, indices_image, vram_bytes) = if is_indexed {
                let raw_pixels = zip
                    .by_name("pixels.png")?
                    .bytes()
                    .collect::<Result<Vec<_>, _>>()?;
                let mut reader = image::ImageReader::new(std::io::Cursor::new(raw_pixels));
                reader.set_format(image::ImageFormat::Png);
                reader.no_limits();
                let pixels_bytes = reader.decode()?.into_bytes();
                let pixels_x = (pixels_bytes.len() as f32 / 8.0).sqrt().ceil() as u32;
                let pixels_y = (pixels_bytes.len() as f32 / (8 * pixels_x) as f32).ceil() as u32;
                let pixels_image = Image::new(
                    Extent3d {
                        width: pixels_x,
                        height: pixels_y,
                        depth_or_array_layers: 1,
                    },
                    wgpu::TextureDimension::D2,
                    pixels_bytes,
                    TextureFormat::Rg32Uint,
                    RenderAssetUsages::RENDER_WORLD,
                );
                let pixels_image =
                    load_context.add_labeled_asset("pixels".to_owned(), pixels_image);

                let raw_indices = zip
                    .by_name("indices.png")?
                    .bytes()
                    .collect::<Result<Vec<_>, _>>()?;
                let mut reader = image::ImageReader::new(std::io::Cursor::new(raw_indices));
                reader.set_format(image::ImageFormat::Png);
                reader.no_limits();
                let indices_bytes = reader.decode()?.into_bytes();

                let use_u16 = pixels_x * pixels_y < 65536;

                let size: UVec2 = packed_tile_size * grid_size;
                let width = if use_u16 { (size.x + 1) / 2 } else { size.x };
                debug!(
                    "load use_u16? {use_u16}, base size: {}, use size: {}, height: {}, total pix: {}",
                    size.x,
                    width,
                    size.y,
                    indices_bytes.len()
                );
                let indices_image = Image::new(
                    Extent3d {
                        width,
                        height: size.y,
                        depth_or_array_layers: 1,
                    },
                    wgpu::TextureDimension::D2,
                    indices_bytes,
                    TextureFormat::R32Uint,
                    RenderAssetUsages::RENDER_WORLD,
                );
                let indices_image =
                    load_context.add_labeled_asset("indices".to_owned(), indices_image);
                (
                    pixels_image,
                    indices_image,
                    pixels_x * pixels_y * 8 + width * size.y * 4,
                )
            } else {
                let raw_image = zip
                    .by_name("texture.png")?
                    .bytes()
                    .collect::<Result<Vec<_>, _>>()?;
                let mut reader = image::ImageReader::new(std::io::Cursor::new(raw_image));
                reader.set_format(image::ImageFormat::Png);
                reader.no_limits();
                let pixels_bytes = reader.decode()?.into_bytes();
                let size: UVec2 = packed_tile_size * grid_size;
                let pixels_image = Image::new(
                    Extent3d {
                        width: size.x,
                        height: size.y,
                        depth_or_array_layers: 1,
                    },
                    wgpu::TextureDimension::D2,
                    pixels_bytes,
                    TextureFormat::Rg32Uint,
                    RenderAssetUsages::RENDER_WORLD,
                );
                let pixels_image =
                    load_context.add_labeled_asset("texture".to_owned(), pixels_image);

                let indices_image = load_context.add_labeled_asset(
                    "dummy_indices".to_owned(),
                    Image::new(
                        Extent3d {
                            width: 1,
                            height: 1,
                            depth_or_array_layers: 1,
                        },
                        wgpu::TextureDimension::D2,
                        vec![0, 0, 0, 0],
                        TextureFormat::R32Uint,
                        RenderAssetUsages::RENDER_WORLD,
                    ),
                );

                (pixels_image, indices_image, size.x * size.y * 8)
            };

            let flags = match load_settings.vertex_mode {
                ImposterVertexMode::Billboard => 4,
                ImposterVertexMode::NoBillboard => 0,
            } + match load_settings.multisample {
                true => RENDER_MULTISAMPLE_FLAG,
                false => 0,
            } + match load_settings.use_source_uv_y {
                true => USE_SOURCE_UV_Y_FLAG,
                false => 0,
            } + match mode {
                "spherical" => GridMode::Spherical,
                "hemispherical" => GridMode::Hemispherical,
                "Horizontal" => GridMode::Horizontal,
                _ => anyhow::bail!("bad mode `{}`", mode),
            }
            .as_flags()
                + if is_indexed { INDEXED_FLAG } else { 0 };

            let alpha_mode = if load_settings.alpha_blend == 0.0 {
                AlphaMode::Blend
            } else if load_settings.alpha_blend == 1.0 {
                AlphaMode::Opaque
            } else {
                AlphaMode::Mask(load_settings.alpha_blend)
            };

            Ok(Imposter {
                data: ImposterData {
                    center_and_scale: Vec3::ZERO.extend(scale),
                    grid_size,
                    flags,
                    alpha: load_settings.alpha,
                    base_tile_size,
                    packed_tile_offset,
                    packed_tile_size,
                },
                pixels: pixels_image,
                indices: indices_image,
                // base_size: size.x * size.y * 8,
                // compressed_size: size.x * size.y * if use_u16 { 1 } else { 2 }
                //     + pixels_x * pixels_y * 8,
                alpha_mode,
                vram_bytes: vram_bytes as usize,
            })
        })
    }

    fn extensions(&self) -> &[&str] {
        &["boimp"]
    }
}

pub fn pack_asset(grid_size: usize, image: &Image) -> (Image, UVec2, UVec2) {
    let width = image.width() as usize;
    let pixels_per_tile = width / grid_size;
    let mut used_x = std::iter::repeat(false)
        .take(pixels_per_tile)
        .collect::<Vec<_>>();
    let mut used_y = std::iter::repeat(false)
        .take(pixels_per_tile)
        .collect::<Vec<_>>();

    let data: &[u32] = bytemuck::cast_slice(&image.data);

    for grid_x in 0..grid_size {
        for grid_y in 0..grid_size {
            for (pix_x, used_x) in used_x.iter_mut().enumerate() {
                for (pix_y, used_y) in used_y.iter_mut().enumerate() {
                    let y = grid_y * pixels_per_tile + pix_y;
                    let x = grid_x * pixels_per_tile + pix_x;
                    if data[(y * width + x) * 2] != 0 {
                        *used_x = true;
                        *used_y = true;
                    }
                }
            }
        }
    }

    let x_start = used_x
        .iter()
        .enumerate()
        .find(|(_, b)| **b)
        .unwrap_or((0, &true))
        .0;
    let x_end = used_x
        .iter()
        .enumerate()
        .rev()
        .find(|(_, b)| **b)
        .unwrap_or((0, &true))
        .0;
    let y_start = used_y
        .iter()
        .enumerate()
        .find(|(_, b)| **b)
        .unwrap_or((0, &true))
        .0;
    let y_end = used_y
        .iter()
        .enumerate()
        .rev()
        .find(|(_, b)| **b)
        .unwrap_or((0, &true))
        .0;
    let x_count = x_end - x_start + 1;
    let y_count = y_end - y_start + 1;
    let new_width = x_count * grid_size;
    let x_ratio = x_count as f32 / pixels_per_tile as f32;
    let y_ratio = y_count as f32 / pixels_per_tile as f32;
    let total_ratio = x_ratio * y_ratio;
    debug!("ratio: {total_ratio} ({x_ratio} * {y_ratio})");
    if total_ratio == 0.0 {
        std::process::exit(1);
    }

    let mut new_data =
        Vec::from_iter(std::iter::repeat(0u32).take(x_count * y_count * 2 * grid_size * grid_size));
    for grid_y in 0..grid_size {
        for grid_x in 0..grid_size {
            for pix_y in 0..y_count {
                let source_x = grid_x * pixels_per_tile + x_start;
                let source_y = grid_y * pixels_per_tile + y_start + pix_y;
                let target_x = grid_x * x_count;
                let target_y = grid_y * y_count + pix_y;

                new_data[(target_y * new_width + target_x) * 2
                    ..(target_y * new_width + target_x + x_count) * 2]
                    .copy_from_slice(
                        &data[(source_y * width + source_x) * 2
                            ..(source_y * width + source_x + x_count) * 2],
                    );
            }
        }
    }

    let new_data_u8 = new_data
        .into_iter()
        .flat_map(|v| v.to_le_bytes())
        .collect::<Vec<_>>();

    let new_image = Image::new(
        Extent3d {
            width: new_width as u32,
            height: (y_count * grid_size) as u32,
            depth_or_array_layers: 1,
        },
        wgpu::TextureDimension::D2,
        new_data_u8,
        wgpu::TextureFormat::Rg32Uint,
        Default::default(),
    );
    (
        new_image,
        UVec2::new(x_start as u32, y_start as u32),
        UVec2::new(x_count as u32, y_count as u32),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn write_asset(
    path: &PathBuf,
    scale: f32,
    grid_size: u32,
    tile_size: u32,
    mode: GridMode,
    image: Image,
    pack: bool,
    index: bool,
) -> Result<(), anyhow::Error> {
    std::fs::create_dir_all(path.parent().unwrap())?;
    let file = std::fs::File::create(path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

    //trim blank edges
    let (image, packed_offset, packed_size) = if pack {
        pack_asset(grid_size as usize, &image)
    } else {
        (image, UVec2::ZERO, UVec2::splat(tile_size))
    };

    let mut wrote_indexed = false;
    if index {
        // gather unique pixel pairs
        let mut pixels = BTreeSet::<[u8; 8]>::default();
        for chunk in image.data.chunks_exact(8) {
            pixels.insert(chunk.try_into().unwrap());
        }

        let pixels_x = (pixels.len() as f32).sqrt().ceil() as u32;
        let pixels_y = (pixels.len() as f32 / pixels_x as f32).ceil() as u32;

        let unique_pixel_count = pixels_x * pixels_y;
        let use_u16 = unique_pixel_count < 65536;

        let base_pixel_count = image.width() * image.height();
        let total_index_size_bytes =
            unique_pixel_count * 8 + base_pixel_count * if use_u16 { 2 } else { 4 };
        let base_size = base_pixel_count * 8;
        if total_index_size_bytes < base_size {
            wrote_indexed = true;

            // write unique pixels to an image
            let mut pixel_data = pixels.iter().copied().flatten().collect::<Vec<_>>();
            // pad to square
            pixel_data.extend(
                std::iter::repeat(0u8)
                    .take(((pixels_x * pixels_y * 8) as usize).saturating_sub(pixel_data.len())),
            );
            let pixels_image = Image::new(
                Extent3d {
                    width: pixels_x,
                    height: pixels_y,
                    depth_or_array_layers: 1,
                },
                wgpu::TextureDimension::D2,
                pixel_data,
                TextureFormat::Rg32Uint,
                Default::default(),
            );

            // write pixels to zip
            let dyn_image = DynamicImage::ImageRgba8(
                ImageBuffer::from_raw(
                    pixels_image.width() * 2,
                    pixels_image.height(),
                    pixels_image.data,
                )
                .unwrap(),
            );
            let mut cursor = Cursor::new(Vec::default());
            dyn_image
                .write_to(&mut cursor, image::ImageFormat::Png)
                .unwrap();
            zip.start_file("pixels.png", options)?;
            zip.write_all(&cursor.into_inner())?;

            // write indices to another image
            debug!(
                "use u16? {}*{}={} < 65536 - {}",
                pixels_x,
                pixels_y,
                pixels_x * pixels_y,
                use_u16
            );
            let pixel_lookup = pixels
                .into_iter()
                .enumerate()
                .map(|(ix, p)| (p, ix))
                .collect::<BTreeMap<_, _>>();
            let mut pixel_indices = image
                .data
                .chunks_exact(8)
                .flat_map(|chunk| {
                    let chunk: [u8; 8] = chunk.try_into().unwrap();
                    let index = *pixel_lookup.get(&chunk).unwrap();
                    if use_u16 {
                        (index as u16).to_le_bytes().to_vec()
                    } else {
                        (index as u32).to_le_bytes().to_vec()
                    }
                })
                .collect::<Vec<_>>();

            let width = if use_u16 {
                if image.width() & 1 == 1 {
                    // pad each line to u32 boundary
                    for i in 0..image.height() {
                        pixel_indices.insert(
                            (image.width() * 2 + i * (image.width() * 2 + 2)) as usize,
                            0,
                        );
                        pixel_indices.insert(
                            (image.width() * 2 + i * (image.width() * 2 + 2)) as usize,
                            0,
                        );
                    }
                    image.width() / 2 + 1
                } else {
                    image.width() / 2
                }
            } else {
                image.width()
            };
            let indices_image = Image::new(
                Extent3d {
                    width,
                    height: image.height(),
                    depth_or_array_layers: 1,
                },
                wgpu::TextureDimension::D2,
                pixel_indices,
                TextureFormat::R32Uint,
                Default::default(),
            );

            // write indices to zip
            let dyn_image = DynamicImage::ImageRgba8(
                ImageBuffer::from_raw(
                    indices_image.width(),
                    indices_image.height(),
                    indices_image.data,
                )
                .unwrap(),
            );
            let mut cursor = Cursor::new(Vec::default());
            dyn_image
                .write_to(&mut cursor, image::ImageFormat::Png)
                .unwrap();
            zip.start_file("indices.png", options)?;
            zip.write_all(&cursor.into_inner())?;
        }
    }

    if !wrote_indexed {
        // write image directly
        let dyn_image = DynamicImage::ImageRgba8(
            ImageBuffer::from_raw(image.width() * 2, image.height(), image.data).unwrap(),
        );
        let mut cursor = Cursor::new(Vec::default());
        dyn_image
            .write_to(&mut cursor, image::ImageFormat::Png)
            .unwrap();
        zip.start_file("texture.png", options)?;
        zip.write_all(&cursor.into_inner())?;
    }

    // write settings
    zip.start_file("settings.txt", options)?;
    let mode = match mode {
        GridMode::Spherical => "spherical",
        GridMode::Hemispherical => "hemispherical",
        GridMode::Horizontal => "Horizontal",
    };
    zip.write_all(
        format!(
            "{grid_size} {scale} {mode} {tile_size} {} {} {} {}",
            packed_offset.x, packed_offset.y, packed_size.x, packed_size.y
        )
        .as_bytes(),
    )?;
    zip.finish()?;
    info!("saved imposter to `{}`", path.to_string_lossy());
    Ok(())
}
