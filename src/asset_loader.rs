use core::str;
use std::io::{Cursor, Read};

use anyhow::anyhow;
use bevy::{
    asset::{AssetLoader, AsyncReadExt},
    math::Vec3,
    prelude::Image,
    render::render_asset::RenderAssetUsages,
};
use wgpu::{Extent3d, TextureFormat};

use crate::render::{Imposter, ImposterData};

pub struct ImposterLoader;

impl AssetLoader for ImposterLoader {
    type Asset = Imposter;

    type Settings = ();

    type Error = anyhow::Error;

    fn load<'a>(
        &'a self,
        reader: &'a mut bevy::asset::io::Reader,
        _: &'a Self::Settings,
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

            let flags = match mode {
                "spherical" => 0,
                "hemispherical" => 1,
                _ => anyhow::bail!("bad mode `{}`", mode),
            };

            Ok(Imposter {
                data: ImposterData {
                    center_and_scale: Vec3::ZERO.extend(scale),
                    grid_size,
                    flags,
                },
                material: Some(image),
                mode: crate::render::ImposterMode::Material,
            })
        })
    }

    fn extensions(&self) -> &[&str] {
        &["boimp"]
    }
}
