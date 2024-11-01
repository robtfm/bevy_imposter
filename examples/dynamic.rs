use std::f32::consts::{FRAC_PI_4, PI};

use bevy::{
    asset::LoadState,
    diagnostic::{FrameTimeDiagnosticsPlugin, LogDiagnosticsPlugin},
    prelude::*,
    render::primitives::{Aabb, Sphere},
    scene::InstanceId,
};
use bevy_imposter::{
    material::{Imposter, ImposterData, ImposterMode},
    bake::{ImposterBakeBundle, ImposterBakeCamera, ImposterBakePlugin},
};
use camera_controller::{CameraController, CameraControllerPlugin};
use rand::{thread_rng, Rng};

#[path = "helpers/camera_controller.rs"]
mod camera_controller;

fn main() {
    println!("specify a glb/gltf via args (or you get a flight helmet)");
    println!("press I to start baking every frame and spawn some imposters. press O to stop baking.");

    App::new()
        .insert_resource(AmbientLight {
            color: Color::WHITE,
            brightness: 0.0,
        })
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    present_mode: bevy::window::PresentMode::Immediate,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            CameraControllerPlugin,
            // GltfImposterPlugin,
            ImposterBakePlugin,
            MaterialPlugin::<Imposter>::default(),
        ))
        .add_plugins((FrameTimeDiagnosticsPlugin, LogDiagnosticsPlugin::default()))
        .add_systems(Startup, setup)
        .add_systems(PreUpdate, setup_scene_after_load)
        .add_systems(
            Update,
            (scene_load_check, impost, update_lights, rotate),
        )
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

fn setup(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    let scene_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "models/FlightHelmet/FlightHelmet.gltf".to_string());
    info!("Loading {}", scene_path);
    let (file_path, scene_index) = parse_scene(scene_path);

    commands.insert_resource(SceneHandle::new(asset_server.load(file_path), scene_index));

    commands.spawn(PbrBundle {
        transform: Transform::from_xyz(2.0, 0.0, 0.0).with_scale(Vec3::splat(0.1)),
        mesh: meshes.add(Cuboid::default().mesh()),
        material: mats.add(Color::BLACK),
        ..Default::default()
    });
    commands.spawn(PbrBundle {
        transform: Transform::from_xyz(2.0, 0.0, 1.0).with_scale(Vec3::splat(0.1)),
        mesh: meshes.add(Cuboid::default().mesh()),
        material: mats.add(Color::srgba(0.0, 0.0, 1.0, 1.0)),
        ..Default::default()
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
            println!("loaded mesh entity: {entity:?}");

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
        let size = aabb.half_extents.length() * 2.0;
        let sphere = Sphere {
            center: aabb.center,
            radius: aabb.half_extents.length(),
        };
        info!("sphere: {:?}", sphere);
        scene_handle.sphere = sphere;

        info!("Spawning a controllable 3D perspective camera");
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
            commands.spawn((DirectionalLightBundle {
                transform: Transform::from_xyz(1.0, 1.0, 0.0).looking_at(Vec3::ZERO, Vec3::Y),
                ..default()
            },));

            scene_handle.has_light = true;
        }
    }
}

pub fn impost(
    mut commands: Commands,
    k: Res<ButtonInput<KeyCode>>,
    scene_handle: Res<SceneHandle>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<Imposter>>,
    cams: Query<Entity, With<ImposterBakeCamera>>,
) {
    if k.just_pressed(KeyCode::KeyO) {
        for entity in cams.iter() {
            println!("stopping imposter baking");
            commands.entity(entity).despawn_recursive();
        }
    }

    if k.just_pressed(KeyCode::KeyI) {
        println!("running imposter baking (every frame)");
        let grid_size = 8;
        let mut camera = ImposterBakeCamera {
            radius: scene_handle.sphere.radius,
            grid_size,
            image_size: 1000,
            grid_mode: bevy_imposter::GridMode::Hemispherical,
            continuous: false,
            ..Default::default()
        };
        camera.init_target(&mut images);

        let mut rng = thread_rng();
        let count = 1;
        let range = -1400.0..=1400.0;
        let offset = Vec3::X * 0.5;
        let rotate_range = 0.0..=(PI * 2.0);
        println!("spawning {count} imposters");
        for _ in 0..count {
            let translation = Vec3::new(
                rng.gen_range(range.clone()),
                rng.gen_range(range.clone()),
                rng.gen_range(range.clone()),
            ) + offset;
            let rotation = Vec3::new(
                rng.gen_range(rotate_range.clone()),
                rng.gen_range(rotate_range.clone()),
                rng.gen_range(rotate_range.clone()),
            );
            let spawned = commands
                .spawn((
                    MaterialMeshBundle {
                        mesh: meshes.add(Rectangle::default().mesh().build()),
                        transform: Transform::from_translation(
                            translation + Vec3::from(scene_handle.sphere.center),
                        )
                        .with_rotation(Quat::from_euler(
                            EulerRot::XYZ,
                            rotation.x,
                            rotation.y,
                            rotation.z,
                        )),
                        material: materials.add(Imposter {
                            data: ImposterData {
                                center_and_scale: Vec3::ZERO.extend(scene_handle.sphere.radius),
                                grid_size,
                                flags: 1,
                            },
                            image: camera.target.clone().unwrap(),
                            mode: ImposterMode::Material,
                        }),
                        ..Default::default()
                    },
                    // Rotate,
                ))
                .id();

            println!("added {spawned:?}");
        }

        commands.spawn(ImposterBakeBundle {
            camera,
            transform: Transform::from_translation(scene_handle.sphere.center.into()),
            ..Default::default()
        });
    }
}

fn update_lights(
    key_input: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut query: Query<(&mut Transform, &mut DirectionalLight)>,
    mut animate_directional_light: Local<bool>,
) {
    for (_, mut light) in &mut query {
        if key_input.just_pressed(KeyCode::KeyU) {
            light.shadows_enabled = !light.shadows_enabled;
        }
    }

    if key_input.just_pressed(KeyCode::KeyL) {
        *animate_directional_light = !*animate_directional_light;
    }
    if *animate_directional_light {
        for (mut transform, _) in &mut query {
            transform.rotation = Quat::from_euler(
                EulerRot::ZYX,
                0.0,
                time.elapsed_seconds() * PI / 15.0,
                -FRAC_PI_4,
            );
        }
    }
}

#[derive(Component)]
pub struct Rotate;

// fix issue with rotated imposters
fn rotate(mut q: Query<&mut Transform, With<Rotate>>, time: Res<Time>) {
    for mut t in q.iter_mut() {
        t.rotation = Quat::from_rotation_y(time.elapsed_seconds());
    }
}
