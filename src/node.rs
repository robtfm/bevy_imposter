use std::{hash::Hash, marker::PhantomData, ops::Range};

use bevy::{
    core_pipeline::prepass::OpaqueNoLightmap3dBinKey,
    ecs::{entity::EntityHashSet, query::QueryFilter},
    pbr::{
        alpha_mode_pipeline_key, graph::NodePbr, prepare_preprocess_bind_groups, DrawMesh,
        GpuPreprocessNode, MaterialPipelineKey, MeshPipeline, MeshPipelineKey, PreparedMaterial,
        PrepassPipeline, PreprocessBindGroup, RenderMaterialInstances, RenderMeshInstances,
        SetMaterialBindGroup, SetMeshBindGroup, SetPrepassViewBindGroup,
    },
    prelude::*,
    render::{
        camera::{
            CameraOutputMode, CameraProjection, CameraRenderGraph, ExtractedCamera,
            NormalizedRenderTarget, ScalingMode,
        },
        mesh::GpuMesh,
        primitives::{Aabb, Sphere},
        render_asset::{prepare_assets, RenderAssetUsages, RenderAssets},
        render_graph::{RenderGraphApp, RenderLabel, RenderSubGraph, ViewNode, ViewNodeRunner},
        render_phase::{
            AddRenderCommand, BinnedPhaseItem, BinnedRenderPhasePlugin, BinnedRenderPhaseType,
            CachedRenderPipelinePhaseItem, DrawFunctionId, DrawFunctions, PhaseItem,
            PhaseItemExtraIndex, RenderCommand, SetItemPipeline, TrackedRenderPass,
            ViewBinnedRenderPhases,
        },
        render_resource::{
            CachedRenderPipelineId, ColorTargetState, ColorWrites, CommandEncoderDescriptor,
            Extent3d, FragmentState, PipelineCache, RenderPassDescriptor, ShaderDefVal, ShaderRef,
            SpecializedMeshPipeline, SpecializedMeshPipelines, StoreOp, Texture, TextureDescriptor,
            TextureDimension, TextureFormat, TextureUsages,
        },
        renderer::RenderDevice,
        texture::{ColorAttachment, GpuImage, ImageSampler, TextureCache},
        view::{
            ColorGrading, ExtractedView, NoFrustumCulling, RenderLayers, ViewDepthTexture,
            ViewUniformOffset, VisibilitySystems, VisibleEntities, WithMesh,
        },
        Extract, Render, RenderApp, RenderSet,
    },
    utils::Parallel,
};

use crate::{oct_coords::normal_from_uv, GridMode};

pub struct ImposterBakePlugin;

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderSubGraph)]
pub struct ImposterBakeGraph;

impl Plugin for ImposterBakePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(BinnedRenderPhasePlugin::<OpaqueImposter, MeshPipeline>::default());
        app.add_systems(
            PostUpdate,
            check_imposter_visibility::<WithMesh>.in_set(VisibilitySystems::CheckVisibility),
        );

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app
            .init_resource::<DrawFunctions<OpaqueImposter>>()
            .init_resource::<ViewBinnedRenderPhases<OpaqueImposter>>()
            .add_systems(ExtractSchedule, extract_imposter_cameras)
            .add_systems(
                Render,
                (
                    prepare_imposter_textures.in_set(RenderSet::PrepareResources),
                    copy_preprocess_bindgroups
                        .in_set(RenderSet::PrepareBindGroups)
                        .after(prepare_preprocess_bind_groups),
                ),
            )
            .add_render_sub_graph(ImposterBakeGraph)
            .add_render_graph_node::<ViewNodeRunner<ImposterBakeNode>>(
                ImposterBakeGraph,
                ImposterBakeNode,
            )
            .add_render_graph_node::<GpuPreprocessNode>(ImposterBakeGraph, NodePbr::GpuPreprocess)
            .add_render_graph_edges(
                ImposterBakeGraph,
                (NodePbr::GpuPreprocess, ImposterBakeNode),
            );

        app.add_plugins(ImposterMaterialPlugin::<StandardMaterial>::default());
    }
}

pub trait ImposterBakeMaterial: Material {
    fn imposter_fragment_shader() -> ShaderRef;
}

impl ImposterBakeMaterial for StandardMaterial {
    fn imposter_fragment_shader() -> ShaderRef {
        "shaders/standard_material_imposter_baker.wgsl".into()
    }
}

#[derive(Default)]
pub struct ImposterMaterialPlugin<M: ImposterBakeMaterial> {
    _p: PhantomData<fn() -> M>,
}

impl<M: ImposterBakeMaterial> Plugin for ImposterMaterialPlugin<M>
where
    M::Data: PartialEq + Eq + Hash + Clone,
{
    fn build(&self, _app: &mut App) {}

    fn finish(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app
            .init_resource::<ImposterBakePipeline<M>>()
            .init_resource::<SpecializedMeshPipelines<ImposterBakePipeline<M>>>()
            .add_render_command::<OpaqueImposter, DrawImposter<M>>()
            .add_systems(
                Render,
                queue_imposter_material_meshes::<M>
                    .in_set(RenderSet::QueueMeshes)
                    .after(prepare_assets::<PreparedMaterial<M>>),
            );
    }
}

#[derive(Component, Clone)]
pub struct ImposterBakeCamera {
    pub radius: f32,
    pub grid_size: u32,
    pub image_size: u32,
    pub grid_mode: GridMode,
    pub target: Handle<Image>,
    pub order: isize,
}

impl Default for ImposterBakeCamera {
    fn default() -> Self {
        Self {
            radius: 1.0,
            grid_size: 8,
            image_size: 512,
            grid_mode: GridMode::Spherical,
            target: Default::default(),
            order: -99,
        }
    }
}

impl ImposterBakeCamera {
    pub fn init_target(&mut self, images: &mut Assets<Image>) {
        let size = Extent3d {
            width: self.image_size,
            height: self.image_size,
            depth_or_array_layers: 1,
        };

        let mut image = Image {
            texture_descriptor: TextureDescriptor {
                label: None,
                size,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rg32Uint,
                mip_level_count: 1,
                sample_count: 1,
                usage: TextureUsages::TEXTURE_BINDING
                    | TextureUsages::COPY_DST
                    | TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            },
            asset_usage: RenderAssetUsages::all(),
            sampler: ImageSampler::nearest(),
            ..default()
        };
        image.resize(size);
        self.target = images.add(image);
    }
}

#[derive(Bundle)]
pub struct ImposterBakeBundle {
    pub camera: ImposterBakeCamera,
    pub graph: CameraRenderGraph,
    pub visible_entities: VisibleEntities,
    pub transform: Transform,
    pub global_transform: GlobalTransform,
}

impl Default for ImposterBakeBundle {
    fn default() -> Self {
        Self {
            camera: Default::default(),
            graph: CameraRenderGraph::new(ImposterBakeGraph),
            visible_entities: Default::default(),
            transform: Default::default(),
            global_transform: Default::default(),
        }
    }
}

pub fn check_imposter_visibility<QF>(
    mut thread_queues: Local<Parallel<Vec<Entity>>>,
    mut view_query: Query<(
        Entity,
        &GlobalTransform,
        &mut VisibleEntities,
        Option<&RenderLayers>,
        &ImposterBakeCamera,
        Has<NoFrustumCulling>,
    )>,
    mut visible_aabb_query: Query<
        (
            Entity,
            &InheritedVisibility,
            &mut ViewVisibility,
            Option<&RenderLayers>,
            Option<&Aabb>,
            &GlobalTransform,
            Has<NoFrustumCulling>,
        ),
        QF,
    >,
) where
    QF: QueryFilter + 'static,
{
    for (_view, gt, mut visible_entities, maybe_view_mask, camera, no_cpu_culling) in
        &mut view_query
    {
        let view_mask = maybe_view_mask.unwrap_or_default();

        visible_aabb_query.par_iter_mut().for_each_init(
            || thread_queues.borrow_local_mut(),
            |queue, query_item| {
                let (
                    entity,
                    inherited_visibility,
                    mut view_visibility,
                    maybe_entity_mask,
                    maybe_model_aabb,
                    transform,
                    no_frustum_culling,
                ) = query_item;

                // Skip computing visibility for entities that are configured to be hidden.
                // ViewVisibility has already been reset in `reset_view_visibility`.
                if !inherited_visibility.get() {
                    return;
                }

                let entity_mask = maybe_entity_mask.unwrap_or_default();
                if !view_mask.intersects(entity_mask) {
                    return;
                }

                // If we have an aabb, do sphere culling
                if !no_frustum_culling && !no_cpu_culling {
                    if let Some(model_aabb) = maybe_model_aabb {
                        let world_from_local = transform.affine();
                        let model_sphere = Sphere {
                            center: world_from_local.transform_point3a(model_aabb.center),
                            radius: transform.radius_vec3a(model_aabb.half_extents),
                        };
                        if (Vec3::from(model_sphere.center) - gt.translation()).length()
                            > model_sphere.radius + camera.radius
                        {
                            return;
                        }
                    }
                }
                view_visibility.set();
                queue.push(entity);
            },
        );

        visible_entities.clear::<QF>();
        thread_queues.drain_into(visible_entities.get_mut::<QF>());
    }
}

#[derive(PartialEq, Eq, Hash)]
pub struct OpaqueImposter {
    pub key: OpaqueNoLightmap3dBinKey,
    pub representative_entity: Entity,
    pub batch_range: Range<u32>,
    pub extra_index: PhaseItemExtraIndex,
}

impl PhaseItem for OpaqueImposter {
    #[inline]
    fn entity(&self) -> Entity {
        self.representative_entity
    }

    #[inline]
    fn draw_function(&self) -> DrawFunctionId {
        self.key.draw_function
    }

    #[inline]
    fn batch_range(&self) -> &Range<u32> {
        &self.batch_range
    }

    #[inline]
    fn batch_range_mut(&mut self) -> &mut Range<u32> {
        &mut self.batch_range
    }

    #[inline]
    fn extra_index(&self) -> PhaseItemExtraIndex {
        self.extra_index
    }

    #[inline]
    fn batch_range_and_extra_index_mut(&mut self) -> (&mut Range<u32>, &mut PhaseItemExtraIndex) {
        (&mut self.batch_range, &mut self.extra_index)
    }
}

impl BinnedPhaseItem for OpaqueImposter {
    type BinKey = OpaqueNoLightmap3dBinKey;

    #[inline]
    fn new(
        key: Self::BinKey,
        representative_entity: Entity,
        batch_range: Range<u32>,
        extra_index: PhaseItemExtraIndex,
    ) -> Self {
        Self {
            key,
            representative_entity,
            batch_range,
            extra_index,
        }
    }
}

impl CachedRenderPipelinePhaseItem for OpaqueImposter {
    #[inline]
    fn cached_pipeline(&self) -> CachedRenderPipelineId {
        self.key.pipeline
    }
}

#[derive(Component)]
pub struct ImposterViews(Vec<(u32, u32, Entity)>);

pub fn extract_imposter_cameras(
    mut commands: Commands,
    mut opaque: ResMut<ViewBinnedRenderPhases<OpaqueImposter>>,
    cameras: Extract<
        Query<(
            Entity,
            &ImposterBakeCamera,
            &GlobalTransform,
            &VisibleEntities,
        )>,
    >,
) {
    let mut entities = EntityHashSet::default();

    for (entity, camera, gt, visible_entities) in cameras.iter() {
        opaque.insert_or_clear(entity);
        entities.insert(entity);

        let center = gt.translation();
        let mut views = Vec::default();
        let mut projection = OrthographicProjection {
            far: camera.radius * 2.0,
            scaling_mode: ScalingMode::Fixed {
                width: camera.radius * 2.0,
                height: camera.radius * 2.0,
            },
            ..Default::default()
        };
        projection.update(0.0, 0.0);
        let clip_from_view = projection.get_clip_from_view();
        for x in 0..camera.grid_size {
            for y in 0..camera.grid_size {
                let uv = UVec2::new(x, y).as_vec2() / (camera.grid_size - 1) as f32;

                let (normal, up) = normal_from_uv(uv, camera.grid_mode);
                let camera_transform = GlobalTransform::from(
                    Transform::from_translation(center + normal * camera.radius)
                        .looking_at(center, up),
                );

                let view = ExtractedView {
                    clip_from_view,
                    world_from_view: camera_transform,
                    clip_from_world: None,
                    hdr: false,
                    viewport: UVec4::new(0, 0, camera.image_size, camera.image_size),
                    color_grading: ColorGrading::default(),
                };

                let id = commands.spawn(view).id();

                views.push((x, y, id));
            }
        }

        commands.get_or_spawn(entity).insert((
            camera.clone(),
            ExtractedCamera {
                target: Some(NormalizedRenderTarget::Image(camera.target.clone())),
                physical_viewport_size: Some(UVec2::splat(camera.image_size)),
                physical_target_size: Some(UVec2::splat(camera.image_size)),
                viewport: None,
                render_graph: ImposterBakeGraph.intern(),
                order: camera.order,
                output_mode: CameraOutputMode::Skip,
                msaa_writeback: false,
                clear_color: ClearColorConfig::None,
                sorted_camera_index_for_target: 0,
                exposure: 0.0,
                hdr: false,
            },
            visible_entities.clone(),
            ImposterViews(views),
            // we must add this to get the gpu mesh uniform system to pick up the view and generate mesh uniforms for us
            // value doesn't matter as we won't render using this view
            ViewUniformOffset { offset: u32::MAX },
        ));
    }

    opaque.retain(|entity, _| entities.contains(entity));
}

fn copy_preprocess_bindgroups(
    mut commands: Commands,
    source: Query<(&ImposterViews, &PreprocessBindGroup)>,
) {
    for (views, bindgroup) in source.iter() {
        for (_, _, view) in views.0.iter() {
            commands.entity(*view).insert(bindgroup.clone());
        }
    }
}

#[derive(Resource)]
pub struct ImposterBakePipeline<M: ImposterBakeMaterial> {
    prepass_pipeline: PrepassPipeline<M>,
    frag_shader: Handle<Shader>,
}

impl<M: ImposterBakeMaterial> FromWorld for ImposterBakePipeline<M> {
    fn from_world(world: &mut World) -> Self {
        Self {
            prepass_pipeline: PrepassPipeline::from_world(world),
            frag_shader: match M::imposter_fragment_shader() {
                ShaderRef::Default => panic!(),
                ShaderRef::Handle(handle) => handle,
                ShaderRef::Path(path) => world.resource::<AssetServer>().load(path),
            },
        }
    }
}

impl<M: ImposterBakeMaterial> SpecializedMeshPipeline for ImposterBakePipeline<M>
where
    M::Data: PartialEq + Eq + Hash + Clone,
{
    type Key = MaterialPipelineKey<M>;

    fn specialize(
        &self,
        key: Self::Key,
        layout: &bevy::render::mesh::MeshVertexBufferLayoutRef,
    ) -> Result<
        bevy::render::render_resource::RenderPipelineDescriptor,
        bevy::render::render_resource::SpecializedMeshPipelineError,
    > {
        // pretty similar to a prepass, so let's start there.
        // would be glorious if this was abstracted so we could avoid cheating like this, or copy/pasting 250 lines
        let mut descriptor = self.prepass_pipeline.specialize(key, layout)?;
        descriptor.label = Some("imposter_bake_pipeline".into());

        // modify defs
        let defs = &mut descriptor.vertex.shader_defs;
        defs.retain(|d| match d {
            ShaderDefVal::Bool(key, _) => match key.as_str() {
                "DEPTH_PREPASS" | "NORMAL_PREPASS" | "MOTION_VECTOR_PREPASS" => false,
                _ => true,
            },
            _ => true,
        });
        defs.extend([
            "IMPOSTER_BAKE_PIPELINE".into(),
            "PREPASS_FRAGMENT".into(),
            "DEPTH_CLAMP_ORTHO".into(),
            "DEFERRED_PREPASS".into(),
            "NORMAL_PREPASS_OR_DEFERRED_PREPASS".into(),
        ]);

        // force inclusion of the vertex normals/tangents
        let mut vertex_attributes = vec![Mesh::ATTRIBUTE_NORMAL.at_shader_location(3)];
        if layout.0.contains(Mesh::ATTRIBUTE_TANGENT) {
            defs.push("VERTEX_TANGENTS".into());
            vertex_attributes.push(Mesh::ATTRIBUTE_TANGENT.at_shader_location(4));
        }
        let buffer_layout = layout.0.get_layout(&vertex_attributes)?;
        descriptor.vertex.buffers[0]
            .attributes
            .extend(buffer_layout.attributes);

        // replace frag state
        descriptor.fragment = Some(FragmentState {
            shader: self.frag_shader.clone(),
            shader_defs: defs.clone(),
            entry_point: "fragment".into(),
            targets: vec![
                None,
                None,
                None,
                None,
                None,
                None,
                Some(ColorTargetState {
                    format: TextureFormat::Rg32Uint,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                }),
            ],
        });

        Ok(descriptor)
    }
}

#[derive(Component)]
pub struct ImposterTextures {
    pub output: ColorAttachment,
    pub depth: ViewDepthTexture,
    pub target: Texture,
}

pub fn prepare_imposter_textures(
    mut commands: Commands,
    mut texture_cache: ResMut<TextureCache>,
    render_device: Res<RenderDevice>,
    opaque_phases: Res<ViewBinnedRenderPhases<OpaqueImposter>>,
    images: Res<RenderAssets<GpuImage>>,
    views: Query<(Entity, &ImposterBakeCamera)>,
) {
    for (entity, camera) in views.iter() {
        if !opaque_phases.contains_key(&entity) {
            continue;
        }

        let size = Extent3d {
            width: camera.image_size,
            height: camera.image_size,
            depth_or_array_layers: 1,
        };

        let descriptor = TextureDescriptor {
            label: Some("imposter_texture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rg32Uint,
            usage: TextureUsages::COPY_SRC
                | TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        };
        let texture = texture_cache.get(&render_device, descriptor);

        let depth_descriptor = TextureDescriptor {
            label: Some("imposter_depth"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Depth32Float,
            usage: TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        };
        let depth_texture = texture_cache.get(&render_device, depth_descriptor);

        let Some(target) = images.get(camera.target.id()) else {
            continue;
        };

        commands.entity(entity).insert(ImposterTextures {
            output: ColorAttachment::new(texture, None, Some(LinearRgba::BLACK)),
            depth: ViewDepthTexture::new(depth_texture, Some(0.0)),
            target: target.texture.clone(),
        });
    }
}

pub fn queue_imposter_material_meshes<M: ImposterBakeMaterial>(
    opaque_draw_functions: Res<DrawFunctions<OpaqueImposter>>,
    mut views: Query<(Entity, &VisibleEntities), With<ImposterBakeCamera>>,
    mut opaque_render_phases: ResMut<ViewBinnedRenderPhases<OpaqueImposter>>,
    imposter_pipeline: Res<ImposterBakePipeline<M>>,
    mut pipelines: ResMut<SpecializedMeshPipelines<ImposterBakePipeline<M>>>,
    pipeline_cache: Res<PipelineCache>,
    render_meshes: Res<RenderAssets<GpuMesh>>,
    render_mesh_instances: Res<RenderMeshInstances>,
    render_materials: Res<RenderAssets<PreparedMaterial<M>>>,
    render_material_instances: Res<RenderMaterialInstances<M>>,
    // render_lightmaps: Res<RenderLightmaps>,
) where
    M::Data: PartialEq + Eq + Hash + Clone,
{
    let opaque_draw = opaque_draw_functions
        .read()
        .get_id::<DrawImposter<M>>()
        .unwrap();

    for (view, visible_entities) in &mut views {
        let Some(opaque_phase) = opaque_render_phases.get_mut(&view) else {
            continue;
        };

        let view_key = MeshPipelineKey::from_msaa_samples(1);

        for visible_entity in visible_entities.iter::<WithMesh>() {
            let Some(material_asset_id) = render_material_instances.get(visible_entity) else {
                continue;
            };
            let Some(mesh_instance) = render_mesh_instances.render_mesh_queue_data(*visible_entity)
            else {
                continue;
            };
            let Some(material) = render_materials.get(*material_asset_id) else {
                continue;
            };
            let Some(mesh) = render_meshes.get(mesh_instance.mesh_asset_id) else {
                continue;
            };

            let mut mesh_key = view_key | MeshPipelineKey::from_bits_retain(mesh.key_bits.bits());

            mesh_key |= alpha_mode_pipeline_key(material.properties.alpha_mode, &Msaa::Off);

            // Even though we don't use the lightmap in the prepass, the
            // `SetMeshBindGroup` render command will bind the data for it. So
            // we need to include the appropriate flag in the mesh pipeline key
            // to ensure that the necessary bind group layout entries are
            // present.
            // unfortunately it's not accessible...
            // if render_lightmaps
            //     .render_lightmaps
            //     .contains_key(visible_entity)
            // {
            //     mesh_key |= MeshPipelineKey::LIGHTMAPPED;
            // }

            let pipeline_id = pipelines.specialize(
                &pipeline_cache,
                &imposter_pipeline,
                MaterialPipelineKey {
                    mesh_key,
                    bind_group_data: material.key.clone(),
                },
                &mesh.layout,
            );
            let pipeline_id = match pipeline_id {
                Ok(id) => id,
                Err(err) => {
                    error!("{}", err);
                    continue;
                }
            };

            match mesh_key
                .intersection(MeshPipelineKey::BLEND_RESERVED_BITS | MeshPipelineKey::MAY_DISCARD)
            {
                MeshPipelineKey::BLEND_OPAQUE => {
                    opaque_phase.add(
                        OpaqueNoLightmap3dBinKey {
                            draw_function: opaque_draw,
                            pipeline: pipeline_id,
                            asset_id: mesh_instance.mesh_asset_id.into(),
                            material_bind_group_id: material.get_bind_group_id().0,
                        },
                        *visible_entity,
                        BinnedRenderPhaseType::mesh(mesh_instance.should_batch()),
                    );
                }
                // Alpha mask
                MeshPipelineKey::MAY_DISCARD => {
                    // todo
                }
                _ => {}
            }
        }
    }
}

#[derive(Default, RenderLabel, Hash, Debug, PartialEq, Eq, Clone)]
pub struct ImposterBakeNode;

impl ViewNode for ImposterBakeNode {
    type ViewQuery = (
        &'static ImposterBakeCamera,
        &'static ImposterTextures,
        &'static ImposterViews,
    );

    fn run<'w>(
        &self,
        graph: &mut bevy::render::render_graph::RenderGraphContext,
        render_context: &mut bevy::render::renderer::RenderContext<'w>,
        (camera, textures, views): bevy::ecs::query::QueryItem<'w, Self::ViewQuery>,
        world: &'w World,
    ) -> Result<(), bevy::render::render_graph::NodeRunError> {
        let view = graph.view_entity();

        let Some(opaque_phase) = world
            .get_resource::<ViewBinnedRenderPhases<OpaqueImposter>>()
            .and_then(|phases| phases.get(&view))
        else {
            return Ok(());
        };

        render_context.add_command_buffer_generation_task(move |render_device| {
            let mut command_encoder =
                render_device.create_command_encoder(&CommandEncoderDescriptor {
                    label: Some("imposter_command_encoder"),
                });

            // Render pass setup
            let render_pass = command_encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("imposter_bake"),
                color_attachments: &[
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(textures.output.get_attachment()),
                ],
                depth_stencil_attachment: Some(textures.depth.get_attachment(StoreOp::Store)),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            let mut render_pass = TrackedRenderPass::new(&render_device, render_pass);

            // if let Some(viewport) = camera.viewport.as_ref() {
            //     render_pass.set_camera_viewport(viewport);
            // }

            // Opaque draws
            for (x, y, view) in &views.0 {
                render_pass.set_viewport(
                    *x as f32 / camera.grid_size as f32 * camera.image_size as f32,
                    *y as f32 / camera.grid_size as f32 * camera.image_size as f32,
                    1.0 / camera.grid_size as f32 * camera.image_size as f32,
                    1.0 / camera.grid_size as f32 * camera.image_size as f32,
                    0.0,
                    1.0,
                );
                opaque_phase.render(&mut render_pass, world, *view);
            }

            drop(render_pass);

            // copy it to the output
            command_encoder.copy_texture_to_texture(
                textures.output.texture.texture.as_image_copy(),
                textures.target.as_image_copy(),
                Extent3d {
                    width: camera.image_size,
                    height: camera.image_size,
                    depth_or_array_layers: 1,
                },
            );

            command_encoder.finish()
        });

        Ok(())
    }
}

pub struct DebugRenderCommand;
impl<P: PhaseItem> RenderCommand<P> for DebugRenderCommand {
    type Param = ();

    type ViewQuery = ();

    type ItemQuery = ();

    fn render<'w>(
        _: &P,
        _: bevy::ecs::query::ROQueryItem<'w, Self::ViewQuery>,
        _: Option<bevy::ecs::query::ROQueryItem<'w, Self::ItemQuery>>,
        _: bevy::ecs::system::SystemParamItem<'w, '_, Self::Param>,
        _: &mut TrackedRenderPass<'w>,
    ) -> bevy::render::render_phase::RenderCommandResult {
        println!("debug render command: {}", std::any::type_name::<P>());
        bevy::render::render_phase::RenderCommandResult::Success
    }
}

pub type DrawImposter<M> = (
    SetItemPipeline,
    SetPrepassViewBindGroup<0>,
    SetMeshBindGroup<1>,
    SetMaterialBindGroup<M, 2>,
    DrawMesh,
    // DebugRenderCommand,
);
