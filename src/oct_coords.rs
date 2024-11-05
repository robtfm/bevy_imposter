use std::f32::consts::PI;

use crate::GridMode;
use bevy::prelude::*;

pub fn normal_from_grid(grid_pos: UVec2, mode: GridMode, grid_size: u32) -> (Vec3, Vec3) {
    let n: Vec3 = match mode {
        GridMode::Spherical => {
            let uv = UVec2::new(grid_pos.x, grid_pos.y).as_vec2() / (grid_size - 1) as f32;

            let x = uv.x * 2.0 - 1.0;
            let z = uv.y * 2.0 - 1.0;
            let y = 1.0 - x.abs() - z.abs();

            if y < 0.0 {
                (
                    x.signum() * (1.0 - z.abs()),
                    y,
                    z.signum() * (1.0 - x.abs()),
                )
            } else {
                (x, y, z)
            }
        }
        GridMode::Hemispherical => {
            let uv = UVec2::new(grid_pos.x, grid_pos.y).as_vec2() / (grid_size - 1) as f32;

            let x = uv.x - uv.y;
            let z = -1.0 + uv.x + uv.y;
            let y = 1.0 - x.abs() - z.abs();
            (x, y, z)
        }
        GridMode::Horizontal => {
            let index = grid_pos.y * grid_size + grid_pos.x;
            let angle = PI * 2.0 * index as f32 / (grid_size * grid_size) as f32;
            let (x, z) = angle.sin_cos();
            (x, 0.0, z)
        }
    }
    .into();
    let n = n.normalize();

    let up = if n.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };

    (n, up)
}
