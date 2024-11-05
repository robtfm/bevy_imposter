// spawn a gltf and create an imposter from it

use bevy::{
    asset::LoadState,
    prelude::*,
    render::primitives::{Aabb, Sphere},
    scene::InstanceId,
    window::ExitCondition,
};
use bevy_imposter::{
    bake::{ImposterBakeBundle, ImposterBakeCamera, ImposterBakePlugin},
    GridMode,
};

#[derive(Resource)]
struct BakeSettings {
    mode: GridMode,
    grid_size: u32,
    image_size: u32,
}

fn main() {
    App::new()
        .insert_resource(AmbientLight {
            color: Color::WHITE,
            brightness: 0.0,
        })
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..Default::default()
            }),
            ImposterBakePlugin,
        ))
        .add_systems(Startup, setup)
        .add_systems(PreUpdate, setup_scene_after_load)
        .add_systems(Update, scene_load_check)
        .run();
}

fn parse_scene(scene_path: String) -> (String, usize) {
    if scene_path.contains('#') {
        let gltf_and_scene = scene_path.split('#').collect::<Vec<_>>();
        if let Some((last, path)) = gltf_and_scene.split_last() {
            if let Some(index) = last
                .strip_prefix("Scene")
                .and_then(|index| index.parse::<usize>().ok())
            {
                return (path.join("#"), index);
            }
        }
    }
    (scene_path, 0)
}

#[derive(Resource, Debug)]
pub struct SceneHandle {
    pub gltf_handle: Handle<Gltf>,
    scene_index: usize,
    instance_id: Option<InstanceId>,
    pub is_loaded: bool,
    pub has_light: bool,
    pub sphere: Sphere,
}

impl SceneHandle {
    pub fn new(gltf_handle: Handle<Gltf>, scene_index: usize) -> Self {
        Self {
            gltf_handle,
            scene_index,
            instance_id: None,
            is_loaded: false,
            has_light: false,
            sphere: Sphere::default(),
        }
    }
}

fn setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    let mut args = pico_args::Arguments::from_env();
    let grid_size = args.value_from_str("--grid").unwrap_or(8);
    let image_size = args.value_from_str("--image").unwrap_or(1024);
    let mode = match args
        .value_from_str("--mode")
        .unwrap_or("h".to_owned())
        .chars()
        .next()
        .unwrap()
    {
        'h' => GridMode::Hemispherical,
        'H' => GridMode::Horizontal,
        's' => GridMode::Spherical,
        _ => {
            warn!("unrecognized mode, use [h]emispherical or [s]pherical. defaulting to Hemispherical");
            GridMode::Hemispherical
        }
    };
    let scene_path = args
        .value_from_str("--source")
        .unwrap_or_else(|_| "models/FlightHelmet/FlightHelmet.gltf".to_string());

    let unused = args.finish();
    if !unused.is_empty() {
        println!("unrecognized arguments: {unused:?}");
        println!("args: \n--mode [h]emispherical or [s]pherical\n--grid n (grid size, default 8)\n--image n (image size, default 1024)\n--source path (asset to load, default flight helmet)");
        std::process::exit(1);
    }

    info!("settings: grid: {grid_size}, image: {image_size}, mode: {mode:?}");
    info!("Loading {}", scene_path);
    let (file_path, scene_index) = parse_scene(scene_path);

    commands.insert_resource(SceneHandle::new(asset_server.load(file_path), scene_index));
    commands.insert_resource(BakeSettings {
        mode,
        grid_size,
        image_size,
    });
}

fn scene_load_check(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut scenes: ResMut<Assets<Scene>>,
    gltf_assets: Res<Assets<Gltf>>,
    mut scene_handle: ResMut<SceneHandle>,
    mut scene_spawner: ResMut<SceneSpawner>,
) {
    match scene_handle.instance_id {
        None => {
            match asset_server.load_state(&scene_handle.gltf_handle) {
                LoadState::Loaded => {
                    let gltf = gltf_assets.get(&scene_handle.gltf_handle).unwrap();
                    if gltf.scenes.len() > 1 {
                        info!(
                            "Displaying scene {} out of {}",
                            scene_handle.scene_index,
                            gltf.scenes.len()
                        );
                        info!("You can select the scene by adding '#Scene' followed by a number to the end of the file path (e.g '#Scene1' to load the second scene).");
                    }

                    let gltf_scene_handle = gltf
                        .scenes
                        .get(scene_handle.scene_index)
                        .unwrap_or_else(|| {
                            panic!(
                                "glTF file doesn't contain scene {}!",
                                scene_handle.scene_index
                            )
                        });
                    let scene = scenes.get_mut(gltf_scene_handle).unwrap();

                    let mut query = scene
                        .world
                        .query::<(Option<&DirectionalLight>, Option<&PointLight>)>();
                    scene_handle.has_light = query.iter(&scene.world).any(
                        |(maybe_directional_light, maybe_point_light)| {
                            maybe_directional_light.is_some() || maybe_point_light.is_some()
                        },
                    );

                    let root = commands
                        .spawn(SpatialBundle {
                            transform: Transform::from_scale(Vec3::splat(1.0)),
                            ..Default::default()
                        })
                        // .insert(Rotate)
                        .id();
                    scene_handle.instance_id =
                        Some(scene_spawner.spawn_as_child(gltf_scene_handle.clone_weak(), root));

                    info!("Spawning scene...");
                }
                LoadState::Failed(_) => {
                    error!("failed to load");
                    std::process::exit(1);
                }
                _ => (),
            }
        }
        Some(instance_id) if !scene_handle.is_loaded => {
            if scene_spawner.instance_is_ready(instance_id) {
                info!("...done!");
                scene_handle.is_loaded = true;
            }
        }
        Some(_) => {}
    }
}

fn setup_scene_after_load(
    mut commands: Commands,
    mut setup: Local<bool>,
    mut scene_handle: ResMut<SceneHandle>,
    meshes: Query<(&GlobalTransform, Option<&Aabb>), With<Handle<Mesh>>>,
    scene_spawner: Res<SceneSpawner>,
    settings: Res<BakeSettings>,
) {
    if scene_handle.is_loaded && !*setup {
        *setup = true;
        // Find an approximate bounding box of the scene from its meshes
        if meshes.iter().any(|(_, maybe_aabb)| maybe_aabb.is_none()) {
            return;
        }

        let mut points = Vec::default();
        for entity in scene_spawner.iter_instance_entities(scene_handle.instance_id.unwrap()) {
            let Ok((transform, maybe_aabb)) = meshes.get(entity) else {
                continue;
            };

            let aabb = maybe_aabb.unwrap();
            let corners = [
                Vec3::new(-1.0, -1.0, -1.0),
                Vec3::new(-1.0, -1.0, 1.0),
                Vec3::new(-1.0, 1.0, -1.0),
                Vec3::new(-1.0, 1.0, 1.0),
                Vec3::new(1.0, -1.0, -1.0),
                Vec3::new(1.0, -1.0, 1.0),
                Vec3::new(1.0, 1.0, -1.0),
                Vec3::new(1.0, 1.0, 1.0),
            ];
            points.extend(corners.iter().map(|c| {
                transform
                    .transform_point(Vec3::from(aabb.center) + (Vec3::from(aabb.half_extents) * *c))
            }));
        }

        let aabb = Aabb::enclosing(points).unwrap();
        let sphere = Sphere {
            center: aabb.center,
            radius: aabb.half_extents.length(),
        };
        info!("sphere: {:?}", sphere);
        scene_handle.sphere = sphere;

        info!("running imposter baking");
        let mut camera = ImposterBakeCamera {
            radius: scene_handle.sphere.radius,
            grid_size: settings.grid_size,
            image_size: settings.image_size,
            grid_mode: settings.mode,
            continuous: false,
            ..Default::default()
        };
        let save_callback = camera.save_asset_callback("assets/boimps/output.boimp");
        camera.set_callback(|image| {
            info!("saving imposter to `assets/boimps/output.boimp`");
            save_callback(image);
            std::process::exit(0);
        });

        commands.spawn(ImposterBakeBundle {
            camera,
            transform: Transform::from_translation(scene_handle.sphere.center.into()),
            ..Default::default()
        });
    }
}
