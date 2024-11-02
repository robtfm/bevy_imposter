use bevy::{asset::LoadState, prelude::*};
use bevy_imposter::{asset_loader::ImposterLoaderSettings, render::Imposter, ImposterRenderPlugin};

pub fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(ImposterRenderPlugin)
        .add_systems(Startup, setup)
        .add_systems(Update, (set_camera_pos, rotate))
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

    commands.spawn((
        MaterialMeshBundle::<Imposter> {
            mesh: meshes.add(Rectangle::default()),
            material: asset_server
                .load_with_settings::<_, ImposterLoaderSettings>(source, move |s| {
                    s.multisample = multisample
                }),
            ..default()
        },
        Rotate,
    ));

    commands.spawn(Camera3dBundle {
        transform: Transform::from_translation(Vec3::ONE).looking_at(Vec3::ZERO, Vec3::Y),
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

fn rotate(mut q: Query<&mut Transform, With<Rotate>>, time: Res<Time>) {
    for mut t in q.iter_mut() {
        t.rotation = Quat::from_rotation_y(time.elapsed_seconds() * 0.2);
    }
}
