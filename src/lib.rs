pub mod asset_loader;
pub mod bake;
pub mod oct_coords;
pub mod render;
pub mod util;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridMode {
    Spherical,
    Hemispherical,
}

pub use bake::ImposterBakePlugin;
pub use render::ImposterRenderPlugin;
