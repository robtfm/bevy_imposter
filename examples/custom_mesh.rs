// niche use case: providing a custom mesh allows for explicit control of the sampled y coords for horizontal billboards

use bevy::{asset::LoadState, prelude::*};
use bevy_imposter::{
    asset_loader::{ImposterLoaderSettings, ImposterVertexMode},
    render::Imposter,
    ImposterRenderPlugin,
};
use camera_controller::{CameraController, CameraControllerPlugin};

#[path = "helpers/camera_controller.rs"]
mod camera_controller;

pub fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(ImposterRenderPlugin)
        .add_plugins(CameraControllerPlugin)
        .add_systems(Startup, setup)
        .add_systems(Update, (set_camera_pos, rotate))
        .run();
}

fn setup(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    let mut args = pico_args::Arguments::from_env();

    let source = args
        .value_from_str("--source")
        .unwrap_or("boimps/output.boimp".to_owned());
    let multisample = args.contains("--multisample");

    if !args.finish().is_empty() {
        println!("args: --source <file>\n--multisample (to multisample)");
        std::process::exit(1);
    };

    let mut mesh = Cuboid::default().mesh().build();
    let Some(bevy::render::mesh::VertexAttributeValues::Float32x2(uvs)) =
        mesh.attribute_mut(Mesh::ATTRIBUTE_UV_0)
    else {
        panic!()
    };
    for ix in [0, 1, 6, 7, 8, 11, 12, 15, 20, 21, 22, 23] {
        uvs[ix][1] = 0.0;
    }
    for ix in [2, 3, 4, 5, 9, 10, 13, 14, 16, 17, 18, 19] {
        uvs[ix][1] = 1.0;
    }
    let mesh = meshes.add(mesh);
    commands.spawn((
        MaterialMeshBundle::<Imposter> {
            mesh,
            transform: Transform::from_translation(Vec3::Y * 23.219)
                .with_scale(Vec3::new(32.0, 46.42, 32.0)),
            material: asset_server.load_with_settings::<_, ImposterLoaderSettings>(
                source,
                move |s| {
                    s.vertex_mode = ImposterVertexMode::NoBillboard;
                    s.use_source_uv_y = true;
                    s.multisample = multisample;
                },
            ),
            ..default()
        },
        Rotate,
    ));

    commands
        .spawn(Camera3dBundle {
            transform: Transform::from_translation(Vec3::ONE).looking_at(Vec3::ZERO, Vec3::Y),
            ..Default::default()
        })
        .insert(CameraController {
            walk_speed: 15.0,
            run_speed: 45.0,
            ..default()
        });

    commands.spawn(PbrBundle {
        mesh: meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(16.0))),
        material: mats.add(StandardMaterial::from(Color::srgb(0.0, 1.0, 0.0))),
        ..Default::default()
    });

    commands.spawn(PbrBundle {
        mesh: meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(32.0))),
        material: mats.add(StandardMaterial::from(Color::srgb(0.0, 0.0, 1.0))),
        transform: Transform::from_translation(Vec3::Y * -0.01),
        ..Default::default()
    });

    commands.spawn(DirectionalLightBundle::default());
}

fn set_camera_pos(
    handles: Query<&Handle<Imposter>>,
    imposters: Res<Assets<Imposter>>,
    mut cam: Query<&mut Transform, With<Camera>>,
    asset_server: Res<AssetServer>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }

    for handle in handles.iter() {
        if let Some(imposter) = imposters.get(handle.id()) {
            *done = true;
            let scale = imposter.data.center_and_scale.w;
            for mut transform in cam.iter_mut() {
                *transform = Transform::from_translation(Vec3::X * scale * 4.0)
                    .looking_at(Vec3::ZERO, Vec3::Y);
            }
            info!("loaded (scale {scale})");
            println!("{imposter:?}");
        }
        if let LoadState::Failed(_) = asset_server.load_state(handle.id()) {
            error!("make sure to save an asset first with the `save_asset` example");
            std::process::exit(1);
        }
    }
}

#[derive(Component)]
pub struct Rotate;

fn rotate(
    mut q: Query<&mut Transform, With<Rotate>>,
    time: Res<Time>,
    k: Res<ButtonInput<KeyCode>>,
    mut rot: Local<bool>,
    mut accrued: Local<f32>,
) {
    if k.just_pressed(KeyCode::KeyR) {
        *rot = !*rot;
    }

    if *rot {
        *accrued += time.delta_seconds();
        for mut t in q.iter_mut() {
            t.rotation = Quat::from_rotation_y(*accrued * 0.2);
        }
    }
}
