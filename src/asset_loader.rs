use core::str;
use std::{
    io::{Cursor, Read, Write},
    mem::ManuallyDrop,
    path::PathBuf,
};

use ::exr::prelude::WritableImage;
use anyhow::anyhow;
use bevy::{
    asset::{AssetLoader, AsyncReadExt},
    math::{UVec2, Vec3},
    prelude::Image,
    render::render_asset::RenderAssetUsages,
};
use bytemuck::{cast_slice, Pod, Zeroable};
use exr::prelude as exrp;
use exr::{
    image::pixel_vec::PixelVec,
    prelude::{ReadChannels, ReadLayers, ReadSpecificChannel},
};
use serde::{Deserialize, Serialize};
use wgpu::{Extent3d, TextureFormat};

use crate::{
    oct_coords::GridMode,
    render::{Imposter, ImposterData, RENDER_MULTISAMPLE_FLAG, USE_SOURCE_UV_Y_FLAG},
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
    // additional alpha multiplier
    pub alpha: f32,
}

impl Default for ImposterLoaderSettings {
    fn default() -> Self {
        Self {
            vertex_mode: Default::default(),
            multisample: Default::default(),
            use_source_uv_y: Default::default(),
            alpha: 1.0,
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

            let raw = zip
                .by_name("texture.exr")?
                .bytes()
                .collect::<Result<Vec<_>, _>>()?;
            let cursor = Cursor::new(raw);

            let mut image = exrp::read()
                .no_deep_data()
                .largest_resolution_level()
                .specific_channels()
                .required("R")
                .required("G")
                .collect_pixels(
                    PixelVec::<PodU32Pair>::constructor,
                    |vec: &mut PixelVec<PodU32Pair>,
                     pos: exr::math::Vec2<usize>,
                     (r, g): (u32, u32)| vec.set_pixel(pos, PodU32Pair(r, g)),
                )
                .all_layers()
                .all_attributes()
                .from_buffered(cursor)?;

            let data = std::mem::take(&mut image.layer_data[0].channel_data.pixels.pixels);
            let size: UVec2 = packed_tile_size * grid_size;

            let raw_u8 = convert_vec(data);

            let image = Image::new(
                Extent3d {
                    width: size.x,
                    height: size.y,
                    depth_or_array_layers: 1,
                },
                wgpu::TextureDimension::D2,
                raw_u8,
                TextureFormat::Rg32Uint,
                RenderAssetUsages::RENDER_WORLD,
            );

            let image = load_context.add_labeled_asset("texture".to_owned(), image);

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
            .as_flags();

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
                material: Some(image),
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

    let data: &[u32] = cast_slice(&image.data);

    for grid_x in 0..grid_size {
        for grid_y in 0..grid_size {
            for pix_x in 0..pixels_per_tile {
                for pix_y in 0..pixels_per_tile {
                    let y = grid_y * pixels_per_tile + pix_y;
                    let x = grid_x * pixels_per_tile + pix_x;
                    if data[(y * width + x) * 2] != 0 {
                        used_x[pix_x] = true;
                        used_y[pix_y] = true;
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
    println!("ratio: {total_ratio} ({x_ratio} * {y_ratio})");
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

    let new_data_u8 = new_data.into_iter().map(|v| v.to_le_bytes()).flatten().collect::<Vec<_>>();

    let new_image = Image::new(
        Extent3d {
            width: new_width as u32,
            height: (y_count * grid_size) as u32,
            depth_or_array_layers: 1,
        },
        wgpu::TextureDimension::D2,
        // convert_vec(new_data),
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

#[repr(C)]
#[derive(Clone, Copy, Zeroable, Default)]
struct PodU32Pair(u32, u32);

unsafe impl Pod for PodU32Pair {}

impl exrp::recursive::IntoRecursive for PodU32Pair {
    type Recursive = <(u32, u32) as exrp::recursive::IntoRecursive>::Recursive;

    fn into_recursive(self) -> Self::Recursive {
        <(u32, u32)>::into_recursive((self.0, self.1))
    }
}

pub fn write_asset(
    path: &PathBuf,
    scale: f32,
    grid_size: u32,
    tile_size: u32,
    mode: GridMode,
    image: Image,
    pack: bool,
) -> Result<(), anyhow::Error> {
    let (image, packed_offset, packed_size) = if pack {
        pack_asset(grid_size as usize, &image)
    } else {
        (image, UVec2::ZERO, UVec2::splat(tile_size))
    };

    let file = std::fs::File::create(path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

    let size = (image.width() as usize, image.height() as usize);
    // let raw_u32_pairs: Vec<PodU32Pair> = convert_vec(image.data);
    let raw_u32_pairs = image.data.chunks_exact(8).map(|chunk| (u32::from_le_bytes(chunk[0..4].try_into().unwrap()), u32::from_le_bytes(chunk[4..8].try_into().unwrap()))).collect::<Vec<_>>();

    if raw_u32_pairs.len() == usize::MAX {
        panic!()
    }

    let exr_image: exrp::Image<exrp::Layer<exrp::SpecificChannels<_, _>>> =
        exrp::Image::from_layer(exrp::Layer::new(
            size,
            exrp::LayerAttributes::default(),
            exrp::Encoding::SMALL_FAST_LOSSLESS,
            exrp::SpecificChannels::new(
                (
                    exrp::ChannelDescription::named("R", exrp::SampleType::U32),
                    exrp::ChannelDescription::named("G", exrp::SampleType::U32),
                ),
                PixelVec::new(size, raw_u32_pairs),
            ),
        ));

    let mut cursor = Cursor::new(Vec::default());

    // note: we use non_parallel as the pooled version seems to cause massive memory leaks
    exr_image.write().non_parallel().to_buffered(&mut cursor)?;
    zip.start_file("texture.exr", options)?;
    zip.write_all(&cursor.into_inner())?;

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
    Ok(())
}

pub fn convert_vec<T: Pod, U: Pod>(mut vec: Vec<T>) -> Vec<U> {
    vec.shrink_to_fit();
    let mut raw = ManuallyDrop::new(vec);
    let (ptr, len, capacity) = (raw.as_mut_ptr(), raw.len(), raw.capacity());

    let size_t = std::mem::size_of::<T>();
    let size_u = std::mem::size_of::<U>();

    let target_len = len * size_t / size_u;
    let target_capacity = capacity * size_t / size_u;

    if len * size_t != target_len * size_u || capacity * size_t != target_capacity * size_u {
        panic!("invalid vec size for conversion");
    }

    unsafe { Vec::from_raw_parts(ptr as *mut U, target_len, target_capacity) }
}
