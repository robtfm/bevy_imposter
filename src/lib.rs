pub mod material;
pub mod bake;
pub mod oct_coords;
pub mod util;
pub mod asset_loader;

use std::collections::VecDeque;

use bevy::{
    asset::LoadState,
    pbr::ExtendedMaterial,
    prelude::*,
    render::{
        camera::{CameraOutputMode, RenderTarget, ScalingMode, Viewport},
        primitives::{Aabb, Sphere},
        render_asset::RenderAssetUsages,
        render_resource::{
            BlendComponent, BlendFactor, BlendOperation, BlendState, Extent3d, TextureDescriptor,
            TextureDimension, TextureFormat, TextureUsages,
        },
        texture::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
        view::{Layer, RenderLayers, VisibilitySystems},
    },
    scene::InstanceId,
};
use material::{Imposter, ImposterData, ImposterMode, StandardMaterialImposterMaker};
use oct_coords::normal_from_uv;
use util::FireEventEx;

#[derive(Clone, Copy, Debug)]
pub enum GridMode {
    Spherical,
    Hemispherical,
}

#[derive(Event, Clone)]
pub struct ImpostGltf {
    pub gltf: Handle<Gltf>,
    pub grid_size: u32,
    pub image_size: UVec2,
    pub grid_mode: GridMode,
    pub imposter_mode: ImposterMode,
}

#[derive(Event)]
pub struct ImpostGltfResult(pub ImpostGltf, pub Result<Imposter, &'static str>);

pub struct GltfImposterPlugin;

pub const IMPOSTER_LAYER: Layer = 7;

impl Plugin for GltfImposterPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<ImpostGltf>()
            .add_plugins(
                (
                    MaterialPlugin::<Imposter>::default(),
                    MaterialPlugin::<
                        ExtendedMaterial<StandardMaterial, StandardMaterialImposterMaker>,
                    >::default(),
                ),
            )
            .add_event::<ImpostGltfResult>()
            .add_systems(Startup, setup)
            .add_systems(
                Update,
                snap_gltfs
                    .after(VisibilitySystems::CalculateBounds)
                    .before(VisibilitySystems::CheckVisibility),
            );
    }
}

#[derive(Resource)]
pub struct GltfImposterSettings {
    root: Entity,
}

fn setup(mut commands: Commands) {
    let root = commands
        .spawn((
            SpatialBundle::default(),
            RenderLayers::layer(IMPOSTER_LAYER),
        ))
        .id();
    commands.insert_resource(GltfImposterSettings { root });
}

#[derive(Clone)]
pub enum ImposterState {
    LoadingScene(InstanceId, bool),
    WaitingToRender {
        camera: Entity,
        image: Handle<Image>,
        image_size: UVec2,
        render_frame: u32,
        sphere: Sphere,
        handles: Vec<UntypedHandle>,
    },
    Rendering {
        camera: Entity,
        image: Handle<Image>,
        image_size: UVec2,
        render_frame: u32,
        sphere: Sphere,
    },
}

pub fn snap_gltfs(
    mut commands: Commands,
    mut impost: EventReader<ImpostGltf>,
    mut scene_spawner: ResMut<SceneSpawner>,
    settings: Res<GltfImposterSettings>,
    gltfs: Res<Assets<Gltf>>,
    mut images: ResMut<Assets<Image>>,
    mut pending: Local<VecDeque<ImpostGltf>>,
    mut in_progress: Local<Option<(ImpostGltf, ImposterState)>>,
    aabbs: Query<(&GlobalTransform, &Aabb)>,
    material_handles: Query<&Handle<StandardMaterial>>,
    materials: Res<Assets<StandardMaterial>>,
    mut replacement_materials: ResMut<
        Assets<ExtendedMaterial<StandardMaterial, StandardMaterialImposterMaker>>,
    >,
    mut cam: Query<(&mut Camera, &mut Transform)>,
    asset_server: Res<AssetServer>,
) {
    for ev in impost.read() {
        pending.push_back(ev.clone());
    }

    if in_progress.is_none() {
        if let Some(ev) = pending.pop_front() {
            let Some(gltf) = gltfs.get(ev.gltf.id()) else {
                commands.fire_event(ImpostGltfResult(ev, Err("Gltf not found")));
                return;
            };
            let Some(scene) = gltf.default_scene.clone() else {
                commands.fire_event(ImpostGltfResult(ev, Err("Gltf has no scene")));
                return;
            };

            let thing_root = commands
                .spawn(SpatialBundle {
                    // transform: Transform::from_scale(Vec3::splat(5.0)),
                    ..Default::default()
                })
                .id();
            commands.entity(settings.root).push_children(&[thing_root]);
            let instance = scene_spawner.spawn_as_child(scene, thing_root);

            *in_progress = Some((ev, ImposterState::LoadingScene(instance, false)));
        }
    }

    if let Some((ev, state)) = in_progress.take() {
        let state = match state {
            ImposterState::LoadingScene(instance_id, false) => ImposterState::LoadingScene(
                instance_id,
                scene_spawner.instance_is_ready(instance_id),
            ),
            ImposterState::LoadingScene(instance_id, true) => {
                let mut handles = Vec::default();
                let mut points = Vec::default();
                for entity in scene_spawner.iter_instance_entities(instance_id) {
                    commands
                        .entity(entity)
                        .insert(RenderLayers::layer(IMPOSTER_LAYER));
                    if let Ok((gt, mesh_aabb)) = aabbs.get(entity) {
                        //point calc
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
                            gt.transform_point(
                                Vec3::from(mesh_aabb.center)
                                    + (Vec3::from(mesh_aabb.half_extents) * *c),
                            )
                        }));
                    }

                    if let Some(mat) = material_handles
                        .get(entity)
                        .ok()
                        .and_then(|h_mat| materials.get(h_mat))
                    {
                        handles.extend(
                            [
                                &mat.base_color_texture,
                                &mat.normal_map_texture,
                                &mat.metallic_roughness_texture,
                                &mat.emissive_texture,
                            ]
                            .iter()
                            .copied()
                            .flatten()
                            .map(|h| h.clone().untyped()),
                        );

                        if ev.imposter_mode == ImposterMode::Material {
                            commands
                                .entity(entity)
                                .remove::<Handle<StandardMaterial>>()
                                .insert(replacement_materials.add(ExtendedMaterial {
                                    base: mat.clone(),
                                    extension: StandardMaterialImposterMaker {},
                                }));
                        }
                    }
                }

                let Some(aabb) = Aabb::enclosing(points) else {
                    commands.fire_event(ImpostGltfResult(ev, Err("Gltf aabb is zero")));
                    return;
                };
                println!("got aabb {aabb:?}");
                let sphere = Sphere {
                    center: aabb.center,
                    radius: aabb.half_extents.length(),
                };
                println!("got sphere2 {sphere:?}");

                let roundup = |sz: u32, to: u32| -> u32 { sz + (to - sz % to) % to };

                let image_size = UVec2::new(
                    roundup(ev.image_size.x, ev.grid_size),
                    roundup(ev.image_size.y, ev.grid_size),
                );

                let size = Extent3d {
                    width: image_size.x,
                    height: image_size.y,
                    depth_or_array_layers: 1,
                };

                let image_format = match ev.imposter_mode {
                    ImposterMode::Image => TextureFormat::Bgra8UnormSrgb,
                    ImposterMode::Material => TextureFormat::Rg32Uint,
                };

                let mut image = Image {
                    texture_descriptor: TextureDescriptor {
                        label: None,
                        size,
                        dimension: TextureDimension::D2,
                        format: image_format,
                        mip_level_count: 1,
                        sample_count: 1,
                        usage: TextureUsages::TEXTURE_BINDING
                            | TextureUsages::COPY_DST
                            | TextureUsages::RENDER_ATTACHMENT,
                        view_formats: &[],
                    },
                    asset_usage: RenderAssetUsages::all(),
                    sampler: ImageSampler::Descriptor(ImageSamplerDescriptor {
                        address_mode_u: ImageAddressMode::Repeat,
                        address_mode_v: ImageAddressMode::Repeat,
                        mag_filter: ImageFilterMode::Linear,
                        min_filter: ImageFilterMode::Linear,
                        mipmap_filter: ImageFilterMode::Linear,
                        ..Default::default()
                    }),
                    ..default()
                };
                image.resize(size);
                let image = images.add(image);

                let (n, up) = normal_from_uv(Vec2::ZERO, ev.grid_mode);

                let camera = commands
                    .spawn((
                        Camera3dBundle {
                            camera: Camera {
                                viewport: Some(Viewport {
                                    physical_position: UVec2::ONE,
                                    physical_size: image_size / ev.grid_size - 2,
                                    ..Default::default()
                                }),
                                target: RenderTarget::Image(image.clone()),
                                output_mode: CameraOutputMode::Skip,
                                clear_color: ClearColorConfig::None,
                                ..Default::default()
                            },
                            projection: OrthographicProjection {
                                far: sphere.radius * 2.0,
                                scaling_mode: ScalingMode::Fixed {
                                    width: sphere.radius * 2.0,
                                    height: sphere.radius * 2.0,
                                },
                                ..Default::default()
                            }
                            .into(),
                            transform: Transform::from_translation(
                                Vec3::from(sphere.center) + n * sphere.radius,
                            )
                            .looking_at(sphere.center.into(), up),
                            ..Default::default()
                        },
                        RenderLayers::layer(IMPOSTER_LAYER),
                    ))
                    .id();

                commands.entity(settings.root).push_children(&[camera]);

                ImposterState::WaitingToRender {
                    camera,
                    image,
                    image_size,
                    sphere,
                    render_frame: u32::MAX,
                    handles,
                }
            }
            ImposterState::WaitingToRender {
                camera,
                image,
                image_size,
                render_frame,
                sphere,
                mut handles,
            } => {
                handles.retain(|h| asset_server.load_state(h.id()) == LoadState::Loading);

                if handles.is_empty() {
                    ImposterState::Rendering {
                        camera,
                        image,
                        image_size,
                        sphere,
                        render_frame: 0,
                    }
                } else {
                    println!("waiting");
                    ImposterState::WaitingToRender {
                        camera,
                        image,
                        image_size,
                        render_frame,
                        sphere,
                        handles,
                    }
                }
            }
            ImposterState::Rendering {
                camera,
                image,
                image_size,
                sphere,
                render_frame,
            } => {
                println!("{}", render_frame);
                if render_frame == ev.grid_size * ev.grid_size - 1 {
                    println!("done");
                    let imposter = Imposter {
                        image,
                        data: ImposterData {
                            center_and_scale: Vec3::from(sphere.center).extend(sphere.radius),
                            grid_size: ev.grid_size,
                            flags: match ev.grid_mode {
                                GridMode::Spherical => 0,
                                GridMode::Hemispherical => 1,
                            },
                        },
                        mode: ev.imposter_mode,
                    };
                    commands.entity(settings.root).despawn_descendants();
                    commands.fire_event(ImpostGltfResult(ev, Ok(imposter)));
                    return;
                }

                let Ok((mut cam, mut transform)) = cam.get_mut(camera) else {
                    commands.fire_event(ImpostGltfResult(ev, Err("Camera lost")));
                    return;
                };

                let next_frame = render_frame + 1;
                let x = next_frame / ev.grid_size;
                let y = next_frame % ev.grid_size;
                cam.viewport = Some(Viewport {
                    physical_position: image_size / ev.grid_size * UVec2::new(x, y) + 1,
                    physical_size: image_size / ev.grid_size - 2,
                    ..Default::default()
                });
                let uv = UVec2::new(x, y).as_vec2() / (ev.grid_size - 1) as f32;
                println!("rect: {:?}", cam.viewport);
                let (n, up) = normal_from_uv(uv, ev.grid_mode);
                *transform =
                    Transform::from_translation(Vec3::from(sphere.center) + n * sphere.radius)
                        .looking_at(sphere.center.into(), up);
                println!(
                    "translation: {:?}, from uv {:?}, normal/up {:?}",
                    transform.translation,
                    uv,
                    normal_from_uv(uv, ev.grid_mode)
                );

                if next_frame == ev.grid_size * ev.grid_size - 1 {
                    cam.output_mode = CameraOutputMode::Write {
                        blend_state: Some(BlendState {
                            color: BlendComponent {
                                src_factor: BlendFactor::One,
                                dst_factor: BlendFactor::One,
                                operation: BlendOperation::Add,
                            },
                            alpha: BlendComponent {
                                src_factor: BlendFactor::One,
                                dst_factor: BlendFactor::One,
                                operation: BlendOperation::Add,
                            },
                        }),
                        clear_color: ClearColorConfig::None,
                    };
                }

                ImposterState::Rendering {
                    camera,
                    image,
                    image_size,
                    render_frame: next_frame,
                    sphere,
                }
            }
        };

        *in_progress = Some((ev, state));
    }
}
