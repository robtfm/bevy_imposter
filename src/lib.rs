pub mod asset_loader;
pub mod bake;
pub mod oct_coords;
pub mod render;
pub mod util;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridMode {
    Spherical,
    Hemispherical,
    Horizontal,
}

pub const GRID_MASK: u32 = 3;

impl GridMode {
    pub fn as_flags(&self) -> u32 {
        match self {
            GridMode::Spherical => 0,
            GridMode::Hemispherical => 1,
            GridMode::Horizontal => 2,
        }
    }
}

pub use bake::ImposterBakePlugin;
pub use render::ImposterRenderPlugin;
