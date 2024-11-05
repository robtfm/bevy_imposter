use core::str;
use std::{
    io::{Cursor, Read, Write},
    path::PathBuf,
};

use anyhow::anyhow;
use bevy::{
    asset::{AssetLoader, AsyncReadExt},
    math::Vec3,
    prelude::Image,
    render::render_asset::RenderAssetUsages,
};
use serde::{Deserialize, Serialize};
use wgpu::{Extent3d, TextureFormat};

use crate::{
    render::{Imposter, ImposterData, RENDER_MULTISAMPLE_FLAG, USE_SOURCE_UV_Y_FLAG},
    GridMode,
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

#[derive(Serialize, Deserialize, Default)]
pub struct ImposterLoaderSettings {
    // billboard / no billboard
    pub vertex_mode: ImposterVertexMode,
    // smooth sample the material texture
    pub multisample: bool,
    // take uv y coords from the source mesh
    pub use_source_uv_y: bool,
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
            let (Some(grid_size), Some(scale), Some(mode)) =
                (parts.next(), parts.next(), parts.next())
            else {
                anyhow::bail!("bad format for settings: `{:?}`", settings);
            };
            let grid_size = grid_size.parse::<u32>()?;
            let scale = scale.parse::<f32>()?;

            let texture = zip
                .by_name("texture.raw")?
                .bytes()
                .collect::<Result<Vec<_>, _>>()?;
            let pixels = texture.len() / 8;
            let image_size = (pixels as f32).sqrt() as u32;

            let image = Image::new(
                Extent3d {
                    width: image_size,
                    height: image_size,
                    depth_or_array_layers: 1,
                },
                wgpu::TextureDimension::D2,
                texture,
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
                },
                material: Some(image),
            })
        })
    }

    fn extensions(&self) -> &[&str] {
        &["boimp"]
    }
}

pub fn write_asset(
    path: &PathBuf,
    scale: f32,
    grid_size: u32,
    mode: GridMode,
    image: Image,
) -> Result<(), anyhow::Error> {
    let file = std::fs::File::create(path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Zstd)
        .compression_level(Some(-10));
    zip.start_file("texture.raw", options)?;
    zip.write_all(&image.data)?;
    zip.start_file("settings.txt", options)?;
    let mode = match mode {
        GridMode::Spherical => "spherical",
        GridMode::Hemispherical => "hemispherical",
        GridMode::Horizontal => "Horizontal",
    };
    zip.write_all(format!("{grid_size} {scale} {mode}").as_bytes())?;
    zip.finish()?;
    Ok(())
}
