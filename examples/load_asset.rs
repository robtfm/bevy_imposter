// load and display a saved imposter

use bevy::{asset::LoadState, prelude::*};
use boimp::{Imposter, ImposterLoaderSettings, ImposterRenderPlugin};
use camera_controller::{CameraController, CameraControllerPlugin};

#[path = "helpers/camera_controller.rs"]
mod camera_controller;

pub fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(ImposterRenderPlugin)
        .add_plugins(CameraControllerPlugin)
        .add_systems(Startup, setup)
        .add_systems(Update, set_camera_pos)
        .run();
}

fn setup(mut commands: Commands, asset_server: Res<AssetServer>, mut meshes: ResMut<Assets<Mesh>>) {
    let mut args = pico_args::Arguments::from_env();

    let source = args
        .value_from_str("--source")
        .unwrap_or("boimps/output.boimp".to_owned());
    let multisample = args.contains("--multisample");

    if !args.finish().is_empty() {
        println!("args: --source <file>\n--multisample (to multisample)");
        std::process::exit(1);
    };

    commands.spawn(MaterialMeshBundle::<Imposter> {
        mesh: meshes.add(Plane3d::new(Vec3::Z, Vec2::splat(0.5))),
        material: asset_server.load_with_settings::<_, ImposterLoaderSettings>(source, move |s| {
            s.multisample = multisample;
        }),
        ..default()
    });

    commands.spawn(Camera3dBundle {
        transform: Transform::from_translation(Vec3::ONE).looking_at(Vec3::ZERO, Vec3::Y),
        camera: Camera {
            clear_color: ClearColorConfig::Custom(Color::srgb(0.4, 0.0, 0.4)),
            ..Default::default()
        },
        ..Default::default()
    });

    commands.spawn(DirectionalLightBundle::default());
}

fn set_camera_pos(
    mut commands: Commands,
    handles: Query<&Handle<Imposter>>,
    imposters: Res<Assets<Imposter>>,
    mut cam: Query<(Entity, &mut Transform), With<Camera>>,
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
            for (cam_ent, mut transform) in cam.iter_mut() {
                *transform = Transform::from_translation(Vec3::X * scale * 4.0)
                    .looking_at(Vec3::ZERO, Vec3::Y);
                commands.entity(cam_ent).insert(CameraController {
                    walk_speed: scale,
                    run_speed: scale * 3.0,
                    ..default()
                });
            }
            info!("loaded {imposter:?}");
        }
        if let LoadState::Failed(_) = asset_server.load_state(handle.id()) {
            error!("make sure to save an asset first with the `save_asset` example");
            std::process::exit(1);
        }
    }
}
