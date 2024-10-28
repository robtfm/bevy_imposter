use bevy::{
    asset::LoadState,
    math::Vec3A,
    prelude::*,
    render::{primitives::{Aabb, Sphere}, view::RenderLayers},
    scene::InstanceId,
};
use bevy_imposter::{material::Imposter, GltfImposterPlugin, ImpostGltf, ImpostGltfResult, ImposterMode, IMPOSTER_LAYER};
use camera_controller::{CameraController, CameraControllerPlugin};

#[path = "helpers/camera_controller.rs"]
mod camera_controller;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window { present_mode: bevy::window::PresentMode::Immediate, ..Default::default() }),
            ..Default::default()
        }), 
            CameraControllerPlugin, 
            GltfImposterPlugin
        ))
        .add_systems(Startup, setup)
        .add_systems(PreUpdate, setup_scene_after_load)
        .add_systems(Update, (scene_load_check, update, impost))
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
}

impl SceneHandle {
    pub fn new(gltf_handle: Handle<Gltf>, scene_index: usize) -> Self {
        Self {
            gltf_handle,
            scene_index,
            instance_id: None,
            is_loaded: false,
            has_light: false,
        }
    }
}

fn setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    let scene_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "models/FlightHelmet/FlightHelmet.gltf".to_string());
    info!("Loading {}", scene_path);
    let (file_path, scene_index) = parse_scene(scene_path);

    commands.insert_resource(SceneHandle::new(asset_server.load(file_path), scene_index));
}

fn scene_load_check(
    asset_server: Res<AssetServer>,
    mut scenes: ResMut<Assets<Scene>>,
    gltf_assets: Res<Assets<Gltf>>,
    mut scene_handle: ResMut<SceneHandle>,
    mut scene_spawner: ResMut<SceneSpawner>,
) {
    match scene_handle.instance_id {
        None => {
            if asset_server.load_state(&scene_handle.gltf_handle) == LoadState::Loaded {
                let gltf = gltf_assets.get(&scene_handle.gltf_handle).unwrap();
                if gltf.scenes.len() > 1 {
                    info!(
                        "Displaying scene {} out of {}",
                        scene_handle.scene_index,
                        gltf.scenes.len()
                    );
                    info!("You can select the scene by adding '#Scene' followed by a number to the end of the file path (e.g '#Scene1' to load the second scene).");
                }

                let gltf_scene_handle =
                    gltf.scenes
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
                scene_handle.has_light =
                    query
                        .iter(&scene.world)
                        .any(|(maybe_directional_light, maybe_point_light)| {
                            maybe_directional_light.is_some() || maybe_point_light.is_some()
                        });

                scene_handle.instance_id =
                    Some(scene_spawner.spawn(gltf_scene_handle.clone_weak()));

                info!("Spawning scene...");
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
) {
    if scene_handle.is_loaded && !*setup {
        *setup = true;
        // Find an approximate bounding box of the scene from its meshes
        if meshes.iter().any(|(_, maybe_aabb)| maybe_aabb.is_none()) {
            return;
        }

        let mut min = Vec3A::splat(f32::MAX);
        let mut max = Vec3A::splat(f32::MIN);
        for (transform, maybe_aabb) in &meshes {
            let aabb = maybe_aabb.unwrap();
            // If the Aabb had not been rotated, applying the non-uniform scale would produce the
            // correct bounds. However, it could very well be rotated and so we first convert to
            // a Sphere, and then back to an Aabb to find the conservative min and max points.
            let sphere = Sphere {
                center: Vec3A::from(transform.transform_point(Vec3::from(aabb.center))),
                radius: transform.radius_vec3a(aabb.half_extents),
            };
            let aabb = Aabb::from(sphere);
            min = min.min(aabb.min());
            max = max.max(aabb.max());
        }

        let size = (max - min).length();
        let aabb = Aabb::from_min_max(Vec3::from(min), Vec3::from(max));

        info!("Spawning a controllable 3D perspective camera [{aabb:?}]");
        let mut projection = PerspectiveProjection::default();
        projection.far = projection.far.max(size * 10.0);

        let walk_speed = size * 3.0;
        let camera_controller = CameraController {
            walk_speed,
            run_speed: 3.0 * walk_speed,
            ..default()
        };

        // Display the controls of the scene viewer
        info!("{}", camera_controller);
        info!("{:?}", *scene_handle);

        commands.spawn((
            Camera3dBundle {
                projection: projection.into(),
                transform: Transform::from_translation(
                    Vec3::from(aabb.center) + size * Vec3::new(0.5, 0.25, 0.5),
                )
                .looking_at(Vec3::from(aabb.center), Vec3::Y),
                camera: Camera {
                    is_active: true,
                    ..default()
                },
                ..default()
            },
            camera_controller,
        ));

        // Spawn a default light if the scene does not have one
        if !scene_handle.has_light {
            info!("Spawning a directional light");
            commands.spawn((
                DirectionalLightBundle {
                    transform: Transform::from_xyz(1.0, 1.0, 0.0).looking_at(Vec3::ZERO, Vec3::Y),
                    ..default()
                },
                RenderLayers::from_layers(&[0, IMPOSTER_LAYER])
            ));

            scene_handle.has_light = true;
        }
    }
}

pub fn impost(
    k: Res<ButtonInput<KeyCode>>,
    scene_handle: Res<SceneHandle>,
    mut req: EventWriter<ImpostGltf>,
) {
    if k.just_pressed(KeyCode::KeyI) {
        // request an imposter
        info!("requesting imposter");
        req.send(ImpostGltf {
            gltf: scene_handle.gltf_handle.clone(),
            grid_size: 6,
            image_size: UVec2::splat(256),
            mode: ImposterMode::Hemispherical,
        });
    }
}

fn update(
    mut commands: Commands,
    mut res: EventReader<ImpostGltfResult>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut imposters: ResMut<Assets<Imposter>>,
) {
    if let Some(res) = res.read().last() {
        match &res.1 {
            Ok(imposter) => {
                info!("result ok! {:?}", imposter);

                commands.spawn(PbrBundle {
                    mesh: meshes.add(Cuboid::default().mesh().build()),
                    transform: Transform::from_translation(Vec3::ONE * 2.0),
                    material: materials.add(StandardMaterial {
                        base_color_texture: Some(imposter.image.clone()),
                        ..Default::default()
                    }),
                    ..Default::default()
                });

                commands.spawn(MaterialMeshBundle {
                    mesh: meshes.add(Rectangle::default()),
                    transform: Transform::from_translation(Vec3::X * -0.4),
                    material: imposters.add(imposter.clone()),
                    ..Default::default()
                });
            },
            Err(e) => {
                error!("{e}");
            },
        }
    }
}
