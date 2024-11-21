// spawn a gltf and bake dynamic imposters every frame
// the gltf can be animated or moved, and the changes are reflected in every imposter.
// scene mgmt copied wholesale from bevy

use std::f32::consts::{FRAC_PI_4, PI};

use bevy::{
    animation::AnimationTarget,
    asset::LoadState,
    diagnostic::{FrameTimeDiagnosticsPlugin, LogDiagnosticsPlugin},
    ecs::entity::EntityHashMap,
    math::FloatOrd,
    prelude::*,
    render::{
        primitives::{Aabb, Sphere},
        view::RenderLayers,
    },
    scene::InstanceId,
    utils::hashbrown::HashMap,
};
use boimp::{
    render::DummyIndicesImage, GridMode, Imposter, ImposterBakeBundle, ImposterBakeCamera,
    ImposterBakePlugin, ImposterData,
};
use camera_controller::{CameraController, CameraControllerPlugin};
use rand::{thread_rng, Rng};

#[path = "helpers/camera_controller.rs"]
mod camera_controller;

#[derive(Resource)]
struct BakeSettings {
    mode: GridMode,
    grid_size: u32,
    tile_size: u32,
    count: usize,
    multisample_source: u32,
    multisample_target: bool,
}

fn main() {
    println!(
        "press I to start baking every frame and spawn some imposters. press O to stop baking."
    );

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
            ImposterBakePlugin,
        ))
        .add_plugins((FrameTimeDiagnosticsPlugin, LogDiagnosticsPlugin::default()))
        .add_systems(Startup, setup)
        .add_systems(PreUpdate, setup_scene_after_load)
        .add_systems(
            Update,
            (
                scene_load_check,
                impost,
                update_lights,
                rotate,
                swap_old,
                setup_anim_after_load,
            ),
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

fn setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    let mut args = pico_args::Arguments::from_env();
    let grid_size = args.value_from_str("--grid").unwrap_or(15);
    let tile_size = args.value_from_str("--tile").unwrap_or(128);
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
            warn!("unrecognized mode, use [h]emispherical or [s]pherical. defaulting to hemispherical");
            GridMode::Hemispherical
        }
    };
    let count = args.value_from_str("--count").unwrap_or(1000);
    let scene_path = args
        .value_from_str("--source")
        .unwrap_or_else(|_| "models/FlightHelmet/FlightHelmet.gltf".to_string());
    let multisample_target = args.contains("--multisample-target");
    let multisample_source = args.value_from_str("--multisample-source").unwrap_or(1);

    let unused = args.finish();
    if !unused.is_empty() {
        println!("unrecognized arguments: {unused:?}");
        println!("args: \n--mode [h]emispherical or [s]pherical\n--grid n (grid size, default 15)\n--image n (image size, default 1024)\n--count n (number of imposters to spawn)\n--multisample-source <n> (to multisample when generating the imposter, try 8)\n--multisample-target (to multisample when rendering imposters)\n--source <path> (asset to load, default flight helmet)");
        std::process::exit(1);
    }

    info!("settings: grid: {grid_size}, tile: {tile_size}, mode: {mode:?}");
    info!("Loading {}", scene_path);
    let (file_path, scene_index) = parse_scene(scene_path);

    commands.insert_resource(SceneHandle::new(asset_server.load(file_path), scene_index));
    commands.insert_resource(BakeSettings {
        mode,
        grid_size,
        tile_size,
        count,
        multisample_source,
        multisample_target,
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

fn setup_anim_after_load(
    mut setup: Local<bool>,
    mut players: Query<&mut AnimationPlayer>,
    targets: Query<(Entity, &AnimationTarget)>,
    parents: Query<&Parent>,
    scene_handle: Res<SceneHandle>,
    clips: Res<Assets<AnimationClip>>,
    gltf_assets: Res<Assets<Gltf>>,
    asset_server: Res<AssetServer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut commands: Commands,
) {
    if scene_handle.is_loaded && !*setup {
        *setup = true;
    } else {
        return;
    }

    let gltf = gltf_assets.get(&scene_handle.gltf_handle).unwrap();
    let animations = &gltf.animations;
    if animations.is_empty() {
        return;
    }

    // copied wholesale from animation_plugin
    let animation_target_id_to_entity: HashMap<_, _> = targets
        .iter()
        .map(|(entity, target)| (target.id, entity))
        .collect();

    let mut player_to_graph: EntityHashMap<(AnimationGraph, Vec<AnimationNodeIndex>)> =
        EntityHashMap::default();

    for (clip_id, clip) in clips.iter() {
        let mut ancestor_player = None;
        for target_id in clip.curves().keys() {
            // If the animation clip refers to entities that aren't present in
            // the scene, bail.
            let Some(&target) = animation_target_id_to_entity.get(target_id) else {
                continue;
            };

            // Find the nearest ancestor animation player.
            let mut current = Some(target);
            while let Some(entity) = current {
                if players.contains(entity) {
                    match ancestor_player {
                        None => {
                            // If we haven't found a player yet, record the one
                            // we found.
                            ancestor_player = Some(entity);
                        }
                        Some(ancestor) => {
                            // If we have found a player, then make sure it's
                            // the same player we located before.
                            if ancestor != entity {
                                // It's a different player. Bail.
                                ancestor_player = None;
                                break;
                            }
                        }
                    }
                }

                // Go to the next parent.
                current = parents.get(entity).ok().map(|parent| parent.get());
            }
        }

        let Some(ancestor_player) = ancestor_player else {
            warn!(
                "Unexpected animation hierarchy for animation clip {:?}; ignoring.",
                clip_id
            );
            continue;
        };

        let Some(clip_handle) = asset_server.get_id_handle(clip_id) else {
            warn!("Clip {:?} wasn't loaded.", clip_id);
            continue;
        };

        let &mut (ref mut graph, ref mut clip_indices) =
            player_to_graph.entry(ancestor_player).or_default();
        let node_index = graph.add_clip(clip_handle, 1.0, graph.root);
        clip_indices.push(node_index);
    }

    for (player_entity, (graph, clips)) in player_to_graph {
        let Ok(mut player) = players.get_mut(player_entity) else {
            warn!("Animation targets referenced a nonexistent player. This shouldn't happen.");
            continue;
        };
        let graph = graphs.add(graph);
        player.play(clips[0]).repeat();
        commands.entity(player_entity).insert(graph);
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

        let aabb = Aabb::enclosing(&points).unwrap();
        let radius = points
            .iter()
            .map(|p| FloatOrd((*p - Vec3::from(aabb.center)).length()))
            .max()
            .unwrap()
            .0;
        let size = radius * 2.0;
        let sphere = Sphere {
            center: aabb.center,
            radius,
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
            RenderLayers::default().with(1), // we keep imposters off the primary renderlayer to avoid imposterception
        ));

        // Spawn a default light if the scene does not have one
        if !scene_handle.has_light {
            info!("Spawning a directional light");
            commands.spawn((
                DirectionalLightBundle {
                    transform: Transform::from_xyz(1.0, 1.0, 0.0).looking_at(Vec3::ZERO, Vec3::Y),
                    ..default()
                },
                RenderLayers::default().with(1),
            ));

            scene_handle.has_light = true;
        }
    }
}

fn impost(
    mut commands: Commands,
    k: Res<ButtonInput<KeyCode>>,
    scene_handle: Res<SceneHandle>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<Imposter>>,
    cams: Query<Entity, With<ImposterBakeCamera>>,
    settings: Res<BakeSettings>,
    dummy_indices: Res<DummyIndicesImage>,
) {
    if k.just_pressed(KeyCode::KeyO) {
        for entity in cams.iter() {
            println!("stopping imposter baking");
            commands.entity(entity).despawn_recursive();
        }
    }

    if k.just_pressed(KeyCode::KeyI) {
        println!("running imposter baking (every frame)");
        let mut camera = ImposterBakeCamera {
            radius: scene_handle.sphere.radius,
            grid_size: settings.grid_size,
            tile_size: settings.tile_size,
            grid_mode: settings.mode,
            continuous: true,
            multisample: settings.multisample_source,
            ..Default::default()
        };
        camera.init_target(&mut images);

        let mut rng = thread_rng();
        let range = scene_handle.sphere.radius * (settings.count as f32).sqrt();
        let range = -range..=range;
        let offset = Vec3::X * 0.5;
        let rotate_range = 0.0..=(PI * 2.0);
        println!("spawning {} imposters", settings.count);
        let hemi_mult = if settings.mode != GridMode::Spherical {
            0.0
        } else {
            1.0
        };
        for _ in 0..settings.count {
            let translation = Vec3::new(
                rng.gen_range(range.clone()),
                rng.gen_range(range.clone()) * hemi_mult,
                rng.gen_range(range.clone()),
            ) + offset;
            let rotation = Vec3::new(
                rng.gen_range(rotate_range.clone()) * hemi_mult,
                rng.gen_range(rotate_range.clone()),
                rng.gen_range(rotate_range.clone()) * hemi_mult,
            );
            commands.spawn((
                MaterialMeshBundle {
                    mesh: meshes.add(Plane3d::new(Vec3::Z, Vec2::splat(0.5))),
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
                        data: ImposterData::new(
                            Vec3::ZERO,
                            scene_handle.sphere.radius,
                            settings.grid_size,
                            settings.tile_size,
                            UVec2::ZERO,
                            UVec2::splat(settings.tile_size),
                            settings.mode,
                            true,
                            settings.multisample_target,
                            false,
                            false,
                            1.0,
                        ),
                        pixels: camera.target.clone().unwrap(),
                        indices: dummy_indices.0.clone(),
                        alpha_mode: AlphaMode::Blend,
                    }),
                    ..Default::default()
                },
                RenderLayers::layer(1),
            ));
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

fn rotate(mut q: Query<&mut Transform, With<Rotate>>, time: Res<Time>) {
    for mut t in q.iter_mut() {
        t.rotation = Quat::from_rotation_y(time.elapsed_seconds());
    }
}

fn swap_old(key_input: Res<ButtonInput<KeyCode>>, mut imps: ResMut<Assets<Imposter>>) {
    if key_input.just_pressed(KeyCode::KeyP) {
        for a in imps.iter_mut() {
            a.1.data.flags ^= 2;
        }
    }
}
