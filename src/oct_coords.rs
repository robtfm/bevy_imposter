use crate::ImposterMode;
use bevy::prelude::*;

pub fn normal_from_uv(uv: Vec2, mode: ImposterMode) -> (Vec3, Vec3) {
    let n: Vec3 = match mode {
        ImposterMode::Spherical => {
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
        ImposterMode::Hemispherical => {
            let x = uv.x - uv.y;
            let z = -1.0 + uv.x + uv.y;
            let y = 1.0 - x.abs() - z.abs();
            (x, y, z)
        }
    }.into();
    let n = n.normalize();

    let up = if n.y.abs() > 0.5 {
        Vec3::Z
    } else {
        Vec3::Y
    };

    (n, up)
}
