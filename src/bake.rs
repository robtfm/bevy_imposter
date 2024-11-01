use std::{
    ffi::OsStr,
    hash::Hash,
    marker::PhantomData,
    ops::Range,
    path::Path,
    sync::{Arc, Mutex},
};

use bevy::{
    core_pipeline::{
        core_3d::{AlphaMask3d, Opaque3d, Opaque3dBinKey, Transparent3d},
        prepass::OpaqueNoLightmap3dBinKey,
    },
    ecs::{entity::EntityHashSet, query::QueryFilter, system::lifetimeless::SRes},
    pbr::{
        alpha_mode_pipeline_key, graph::NodePbr, prepare_preprocess_bind_groups, DrawMesh,
        GpuPreprocessNode, MaterialPipelineKey, MeshPipeline, MeshPipelineKey, PreparedMaterial,
        PrepassPipeline, PreprocessBindGroup, RenderMaterialInstances, RenderMeshInstances,
        SetMaterialBindGroup, SetMeshBindGroup, SetPrepassViewBindGroup, SkipGpuPreprocess,
    },
    prelude::*,
    render::{
        camera::{
            CameraOutputMode, CameraProjection, CameraRenderGraph, ExtractedCamera, ScalingMode,
        },
        mesh::GpuMesh,
        primitives::{Aabb, Sphere},
        render_asset::{prepare_assets, RenderAssetUsages, RenderAssets},
        render_graph::{RenderGraphApp, RenderLabel, RenderSubGraph, ViewNode, ViewNodeRunner},
        render_phase::{
            AddRenderCommand, BinnedPhaseItem, BinnedRenderPhasePlugin, BinnedRenderPhaseType,
            CachedRenderPipelinePhaseItem, DrawFunctionId, DrawFunctions, PhaseItem,
            PhaseItemExtraIndex, RenderCommand, SetItemPipeline, SortedPhaseItem,
            SortedRenderPhasePlugin, TrackedRenderPass, ViewBinnedRenderPhases,
            ViewSortedRenderPhases,
        },
        render_resource::{
            Buffer, BufferDescriptor, CachedRenderPipelineId, ColorTargetState, ColorWrites,
            CommandEncoderDescriptor, Extent3d, FragmentState, PipelineCache, RenderPassDescriptor,
            ShaderDefVal, ShaderRef, SpecializedMeshPipeline, SpecializedMeshPipelines, StoreOp,
            Texture, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
        },
        renderer::RenderDevice,
        texture::{ColorAttachment, GpuImage, ImageSampler, TextureCache, TextureFormatPixelInfo},
        view::{
            ColorGrading, ExtractedView, NoFrustumCulling, RenderLayers, ViewDepthTexture,
            ViewUniformOffset, VisibilitySystems, VisibleEntities, WithMesh,
        },
        Extract, Render, RenderApp, RenderSet,
    },
    tasks::AsyncComputeTaskPool,
    utils::Parallel,
};
use wgpu::{BufferUsages, ImageCopyBuffer, ImageDataLayout};

use crate::{asset_loader::write_asset, oct_coords::normal_from_uv, GridMode};

pub struct ImposterBakePlugin;

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderSubGraph)]
pub struct ImposterBakeGraph;

impl Plugin for ImposterBakePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(BinnedRenderPhasePlugin::<
            ImposterPhaseItem<Opaque3d>,
            MeshPipeline,
        >::default());
        app.add_plugins(BinnedRenderPhasePlugin::<
            ImposterPhaseItem<AlphaMask3d>,
            MeshPipeline,
        >::default());
        app.add_plugins(SortedRenderPhasePlugin::<
            ImposterPhaseItem<Transparent3d>,
            MeshPipeline,
        >::default());
        app.add_systems(
            PostUpdate,
            (
                check_imposter_visibility::<WithMesh>.in_set(VisibilitySystems::CheckVisibility),
                check_finished_cameras,
            ),
        );

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app
            .init_resource::<DrawFunctions<ImposterPhaseItem<Opaque3d>>>()
            .init_resource::<DrawFunctions<ImposterPhaseItem<AlphaMask3d>>>()
            .init_resource::<DrawFunctions<ImposterPhaseItem<Transparent3d>>>()
            .init_resource::<ViewBinnedRenderPhases<ImposterPhaseItem<Opaque3d>>>()
            .init_resource::<ViewBinnedRenderPhases<ImposterPhaseItem<AlphaMask3d>>>()
            .init_resource::<ViewSortedRenderPhases<ImposterPhaseItem<Transparent3d>>>()
            .init_resource::<ImposterActualRenderCount>()
            .init_resource::<ImpostersBaked>()
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
            .add_systems(
                Render,
                copy_back
                    .in_set(RenderSet::Cleanup)
                    .before(World::clear_entities),
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
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            count_expected_imposter_materials::<M>.after(check_imposter_visibility::<WithMesh>),
        );
    }

    fn finish(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app
            .init_resource::<ImposterBakePipeline<M>>()
            .init_resource::<SpecializedMeshPipelines<ImposterBakePipeline<M>>>()
            .add_render_command::<ImposterPhaseItem<Opaque3d>, DrawImposter<M>>()
            .add_render_command::<ImposterPhaseItem<AlphaMask3d>, DrawImposter<M>>()
            .add_render_command::<ImposterPhaseItem<Transparent3d>, DrawImposter<M>>()
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
    pub target: Option<Handle<Image>>,
    pub order: isize,
    pub continuous: bool,
    pub is_finished: bool,
    pub callback: Option<ImageCallback>,
}

impl Default for ImposterBakeCamera {
    fn default() -> Self {
        Self {
            radius: 1.0,
            grid_size: 8,
            image_size: 512,
            grid_mode: GridMode::Spherical,
            target: None,
            order: -99,
            continuous: false,
            is_finished: false,
            callback: None,
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
        self.target = Some(images.add(image));
    }

    pub fn set_callback(&mut self, callback: impl FnOnce(Image) + Send + Sync + 'static) {
        self.callback = Some(Arc::new(Mutex::new(Some(Box::new(callback)))));
    }

    // Returns an async fn that can be set as the callback to save the asset once baked
    // warning: uses the current camera state - changes after this call will not be reflected
    pub fn save_asset_callback(
        &mut self,
        path: impl AsRef<Path>,
    ) -> impl FnOnce(bevy::prelude::Image) + Send + Sync + 'static {
        let mut path = path.as_ref().to_owned();
        if path.extension() != Some(OsStr::new("boimp")) {
            path.set_extension("boimp");
        }

        let grid_size = self.grid_size;
        let radius = self.radius;
        let mode = self.grid_mode;
        move |image| {
            if let Err(e) = write_asset(&path, radius, grid_size, mode, image) {
                error!("error writing imposter asset: {e}");
            } else {
                info!("imposter saved");
            }
        }
    }
}

#[derive(Component)]
pub struct ImposterBakeCompleteChannel {
    sender: crossbeam_channel::Sender<()>,
    receiver: Option<crossbeam_channel::Receiver<()>>,
}

impl Default for ImposterBakeCompleteChannel {
    fn default() -> Self {
        let (sender, receiver) = crossbeam_channel::bounded(5); // make sure we don't block rendering
        Self {
            sender,
            receiver: Some(receiver),
        }
    }
}

#[derive(Bundle)]
pub struct ImposterBakeBundle {
    pub camera: ImposterBakeCamera,
    pub graph: CameraRenderGraph,
    pub visible_entities: VisibleEntities,
    pub expected_count: ImposterExpectedRenderCount,
    pub transform: Transform,
    pub global_transform: GlobalTransform,
    pub complete: ImposterBakeCompleteChannel,
}

impl Default for ImposterBakeBundle {
    fn default() -> Self {
        Self {
            camera: Default::default(),
            graph: CameraRenderGraph::new(ImposterBakeGraph),
            expected_count: Default::default(),
            visible_entities: Default::default(),
            transform: Default::default(),
            global_transform: Default::default(),
            complete: Default::default(),
        }
    }
}

#[allow(clippy::type_complexity)]
pub fn check_imposter_visibility<QF>(
    mut thread_queues: Local<Parallel<Vec<Entity>>>,
    mut view_query: Query<(
        Entity,
        &GlobalTransform,
        &mut VisibleEntities,
        Option<&RenderLayers>,
        &ImposterBakeCamera,
        &mut ImposterExpectedRenderCount,
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
    for (
        _view,
        gt,
        mut visible_entities,
        maybe_view_mask,
        camera,
        mut expected_count,
        no_cpu_culling,
    ) in &mut view_query
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
        expected_count.0 = 0;
    }
}

fn count_expected_imposter_materials<M: ImposterBakeMaterial>(
    mut q: Query<(&mut ImposterExpectedRenderCount, &VisibleEntities), With<ImposterBakeCamera>>,
    materials: Query<(), With<Handle<M>>>,
) {
    for (mut count, visible_entities) in q.iter_mut() {
        let material_count = visible_entities
            .iter::<WithMesh>()
            .filter(|e| materials.get(**e).is_ok())
            .count();
        count.0 += material_count;
    }
}

#[derive(Component)]
pub struct ExtractedImposterBakeCamera {
    pub grid_size: u32,
    pub image_size: u32,
    pub target: Option<Handle<Image>>,
    pub subviews: Vec<(u32, u32, Entity)>,
    pub expected_count: usize,
    pub channel: crossbeam_channel::Sender<()>,
    pub callback: Option<ImageCallback>,
}

#[derive(PartialEq, Eq, Hash)]
pub struct ImposterPhaseItem<T: 'static> {
    inner: T,
}

impl<T: SortedPhaseItem> SortedPhaseItem for ImposterPhaseItem<T> {
    type SortKey = T::SortKey;

    fn sort_key(&self) -> Self::SortKey {
        self.inner.sort_key()
    }
}

impl<T: PhaseItem> PhaseItem for ImposterPhaseItem<T> {
    #[inline]
    fn entity(&self) -> Entity {
        self.inner.entity()
    }

    #[inline]
    fn draw_function(&self) -> DrawFunctionId {
        self.inner.draw_function()
    }

    #[inline]
    fn batch_range(&self) -> &Range<u32> {
        self.inner.batch_range()
    }

    #[inline]
    fn batch_range_mut(&mut self) -> &mut Range<u32> {
        self.inner.batch_range_mut()
    }

    #[inline]
    fn extra_index(&self) -> PhaseItemExtraIndex {
        self.inner.extra_index()
    }

    #[inline]
    fn batch_range_and_extra_index_mut(&mut self) -> (&mut Range<u32>, &mut PhaseItemExtraIndex) {
        self.inner.batch_range_and_extra_index_mut()
    }
}

impl<T: BinnedPhaseItem> BinnedPhaseItem for ImposterPhaseItem<T> {
    type BinKey = T::BinKey;

    #[inline]
    fn new(
        key: Self::BinKey,
        representative_entity: Entity,
        batch_range: Range<u32>,
        extra_index: PhaseItemExtraIndex,
    ) -> Self {
        Self {
            inner: T::new(key, representative_entity, batch_range, extra_index),
        }
    }
}

impl<T: CachedRenderPipelinePhaseItem> CachedRenderPipelinePhaseItem for ImposterPhaseItem<T> {
    #[inline]
    fn cached_pipeline(&self) -> CachedRenderPipelineId {
        self.inner.cached_pipeline()
    }
}

fn check_finished_cameras(
    mut commands: Commands,
    mut q: Query<(
        Entity,
        &mut ImposterBakeCamera,
        &ImposterBakeCompleteChannel,
    )>,
) {
    for (ent, mut cam, receiver) in q.iter_mut() {
        while receiver
            .receiver
            .as_ref()
            .and_then(|r| r.try_recv().ok())
            .is_some()
        {
            if !cam.continuous {
                debug!("recv success");
                cam.is_finished = true;
                commands.entity(ent).remove::<ImposterBakeCompleteChannel>();
            }
        }
    }
}

pub type ImageCallback = Arc<Mutex<Option<Box<dyn FnOnce(Image) + Send + Sync + 'static>>>>;

#[derive(Resource)]
pub struct ImpostersBaked {
    sender: crossbeam_channel::Sender<(u32, ImageCallback, Buffer)>,
    receiver: crossbeam_channel::Receiver<(u32, ImageCallback, Buffer)>,
}

impl Default for ImpostersBaked {
    fn default() -> Self {
        let (sender, receiver) = crossbeam_channel::unbounded();
        Self { sender, receiver }
    }
}

#[allow(clippy::type_complexity)]
pub fn extract_imposter_cameras(
    mut commands: Commands,
    mut opaque: ResMut<ViewBinnedRenderPhases<ImposterPhaseItem<Opaque3d>>>,
    mut alphamask: ResMut<ViewBinnedRenderPhases<ImposterPhaseItem<AlphaMask3d>>>,
    mut transparent: ResMut<ViewSortedRenderPhases<ImposterPhaseItem<Transparent3d>>>,
    cameras: Extract<
        Query<(
            Entity,
            &ImposterBakeCamera,
            &ImposterBakeCompleteChannel,
            &ImposterExpectedRenderCount,
            &GlobalTransform,
            &VisibleEntities,
        )>,
    >,
) {
    let mut entities = EntityHashSet::default();

    for (entity, camera, channel, expected_count, gt, visible_entities) in cameras.iter() {
        if camera.is_finished || !channel.receiver.as_ref().map_or(true, |r| r.is_empty()) {
            continue;
        }
        debug!("extract");
        opaque.insert_or_clear(entity);
        alphamask.insert_or_clear(entity);
        transparent.insert_or_clear(entity);
        entities.insert(entity);

        let center = gt.translation();
        let mut subviews = Vec::default();
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

                subviews.push((x, y, id));
            }
        }

        commands.get_or_spawn(entity).insert((
            ExtractedImposterBakeCamera {
                grid_size: camera.grid_size,
                image_size: camera.image_size,
                target: camera.target.clone(),
                subviews,
                expected_count: expected_count.0,
                channel: channel.sender.clone(),
                callback: camera.callback.clone(),
            },
            ExtractedCamera {
                target: None,
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
            // we must add this to get the gpu mesh uniform system to pick up the view and generate mesh uniforms for us
            // value doesn't matter as we won't render using this view
            ViewUniformOffset { offset: u32::MAX },
        ));
    }

    opaque.retain(|entity, _| entities.contains(entity));
    alphamask.retain(|entity, _| entities.contains(entity));
    transparent.retain(|entity, _| entities.contains(entity));
}

fn copy_preprocess_bindgroups(
    mut commands: Commands,
    source: Query<(&ExtractedImposterBakeCamera, &PreprocessBindGroup)>,
) {
    for (views, bindgroup) in source.iter() {
        for (_, _, view) in views.subviews.iter() {
            commands
                .entity(*view)
                .insert((bindgroup.clone(), SkipGpuPreprocess));
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
            ShaderDefVal::Bool(key, _) => !matches!(
                key.as_str(),
                "DEPTH_PREPASS" | "NORMAL_PREPASS" | "MOTION_VECTOR_PREPASS"
            ),
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
            targets: vec![Some(ColorTargetState {
                format: TextureFormat::Rg32Uint,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
        });

        Ok(descriptor)
    }
}

#[derive(Component)]
pub struct ImposterTextures {
    pub output: ColorAttachment,
    pub depth: ViewDepthTexture,
    pub target: Option<Texture>,
}

pub fn prepare_imposter_textures(
    mut commands: Commands,
    mut texture_cache: ResMut<TextureCache>,
    render_device: Res<RenderDevice>,
    opaque_phases: Res<ViewBinnedRenderPhases<ImposterPhaseItem<Opaque3d>>>,
    images: Res<RenderAssets<GpuImage>>,
    views: Query<(Entity, &ExtractedImposterBakeCamera)>,
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

        commands.entity(entity).insert(ImposterTextures {
            output: ColorAttachment::new(texture, None, Some(LinearRgba::BLACK)),
            depth: ViewDepthTexture::new(depth_texture, Some(0.0)),
            target: camera
                .target
                .as_ref()
                .and_then(|target| images.get(target.id()))
                .map(|image| image.texture.clone()),
        });
    }
}

#[allow(clippy::too_many_arguments)]
pub fn queue_imposter_material_meshes<M: ImposterBakeMaterial>(
    opaque_draw_functions: Res<DrawFunctions<ImposterPhaseItem<Opaque3d>>>,
    alphamask_draw_functions: Res<DrawFunctions<ImposterPhaseItem<AlphaMask3d>>>,
    transparent_draw_functions: Res<DrawFunctions<ImposterPhaseItem<Transparent3d>>>,
    mut views: Query<(Entity, &VisibleEntities), With<ExtractedImposterBakeCamera>>,
    mut opaque_render_phases: ResMut<ViewBinnedRenderPhases<ImposterPhaseItem<Opaque3d>>>,
    mut alphamask_render_phases: ResMut<ViewBinnedRenderPhases<ImposterPhaseItem<AlphaMask3d>>>,
    mut transparent_render_phases: ResMut<ViewSortedRenderPhases<ImposterPhaseItem<Transparent3d>>>,
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
    let alphamask_draw = alphamask_draw_functions
        .read()
        .get_id::<DrawImposter<M>>()
        .unwrap();
    let transparent_draw = transparent_draw_functions
        .read()
        .get_id::<DrawImposter<M>>()
        .unwrap();

    for (view, visible_entities) in &mut views {
        let (Some(opaque_phase), Some(alphamask_phase), Some(transparent_phase)) = (
            opaque_render_phases.get_mut(&view),
            alphamask_render_phases.get_mut(&view),
            transparent_render_phases.get_mut(&view),
        ) else {
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
                        Opaque3dBinKey {
                            draw_function: opaque_draw,
                            pipeline: pipeline_id,
                            asset_id: mesh_instance.mesh_asset_id.into(),
                            material_bind_group_id: material.get_bind_group_id().0,
                            lightmap_image: None, // can't check the mesh bit
                        },
                        *visible_entity,
                        BinnedRenderPhaseType::mesh(mesh_instance.should_batch()),
                    );
                }
                // Alpha mask
                MeshPipelineKey::MAY_DISCARD => {
                    alphamask_phase.add(
                        OpaqueNoLightmap3dBinKey {
                            draw_function: alphamask_draw,
                            pipeline: pipeline_id,
                            asset_id: mesh_instance.mesh_asset_id.into(),
                            material_bind_group_id: material.get_bind_group_id().0,
                        },
                        *visible_entity,
                        BinnedRenderPhaseType::mesh(mesh_instance.should_batch()),
                    );
                }
                _ => {
                    transparent_phase.add(ImposterPhaseItem {
                        inner: Transparent3d {
                            entity: *visible_entity,
                            draw_function: transparent_draw,
                            pipeline: pipeline_id,
                            distance: 0.0, // this will be wrong for some view whatever we use...
                            batch_range: 0..1,
                            extra_index: PhaseItemExtraIndex::NONE,
                        },
                    });
                }
            }
        }
    }
}

#[derive(Default, RenderLabel, Hash, Debug, PartialEq, Eq, Clone)]
pub struct ImposterBakeNode;

impl ViewNode for ImposterBakeNode {
    type ViewQuery = (
        &'static ExtractedImposterBakeCamera,
        &'static ImposterTextures,
    );

    fn run<'w>(
        &self,
        graph: &mut bevy::render::render_graph::RenderGraphContext,
        render_context: &mut bevy::render::renderer::RenderContext<'w>,
        (camera, textures): bevy::ecs::query::QueryItem<'w, Self::ViewQuery>,
        world: &'w World,
    ) -> Result<(), bevy::render::render_graph::NodeRunError> {
        let view = graph.view_entity();

        let (Some(opaque_phase), Some(alphamask_phase), Some(transparent_phase)) = (
            world
                .get_resource::<ViewBinnedRenderPhases<ImposterPhaseItem<Opaque3d>>>()
                .and_then(|phases| phases.get(&view)),
            world
                .get_resource::<ViewBinnedRenderPhases<ImposterPhaseItem<AlphaMask3d>>>()
                .and_then(|phases| phases.get(&view)),
            world
                .get_resource::<ViewSortedRenderPhases<ImposterPhaseItem<Transparent3d>>>()
                .and_then(|phases| phases.get(&view)),
        ) else {
            return Ok(());
        };

        let actual = world.resource::<ImposterActualRenderCount>();
        *actual.0.lock().unwrap() = 0;

        render_context.add_command_buffer_generation_task(move |render_device| {
            let mut command_encoder =
                render_device.create_command_encoder(&CommandEncoderDescriptor {
                    label: Some("imposter_command_encoder"),
                });

            // Render pass setup
            let render_pass = command_encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("imposter_bake"),
                color_attachments: &[Some(textures.output.get_attachment())],
                depth_stencil_attachment: Some(textures.depth.get_attachment(StoreOp::Store)),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            let mut render_pass = TrackedRenderPass::new(&render_device, render_pass);

            // Opaque draws
            // we use the batch from the dummy main view, which means opaque will be rendered potentially out of order
            // TODO: see if it's worth binning for every individual view separately. since this is baking, probably not.
            // if we use it for dynamic imposters in future there'd only be a single view being rendered anyway

            let tile_size = 1.0 / camera.grid_size as f32 * camera.image_size as f32;
            // run once to check if all the items are ready and rendering

            render_pass.set_viewport(0.0, 0.0, tile_size, tile_size, 0.0, 1.0);
            opaque_phase.render(&mut render_pass, world, camera.subviews[0].2);
            alphamask_phase.render(&mut render_pass, world, camera.subviews[0].2);
            transparent_phase.render(&mut render_pass, world, camera.subviews[0].2);

            let actual = *actual.0.lock().unwrap();
            let success = actual == camera.expected_count;

            if success {
                for (x, y, view) in camera.subviews.iter().skip(1) {
                    render_pass.set_viewport(
                        *x as f32 * tile_size,
                        *y as f32 * tile_size,
                        tile_size,
                        tile_size,
                        0.0,
                        1.0,
                    );
                    opaque_phase.render(&mut render_pass, world, *view);
                    alphamask_phase.render(&mut render_pass, world, *view);
                    transparent_phase.render(&mut render_pass, world, *view);
                }
            }

            drop(render_pass);

            if success {
                if let Some(callback) = camera.callback.as_ref() {
                    debug!("send callback buffer");
                    let render_device = world.resource::<RenderDevice>();

                    let buffer = render_device.create_buffer(&BufferDescriptor {
                        label: Some("imposter transfer buffer"),
                        size: get_aligned_size(
                            camera.image_size,
                            camera.image_size,
                            TextureFormat::Rg32Uint.pixel_size() as u32,
                        ) as u64,
                        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });

                    command_encoder.copy_texture_to_buffer(
                        textures.output.texture.texture.as_image_copy(),
                        ImageCopyBuffer {
                            buffer: &buffer,
                            layout: ImageDataLayout {
                                bytes_per_row: Some(get_aligned_size(
                                    camera.image_size,
                                    1,
                                    TextureFormat::Rg32Uint.pixel_size() as u32,
                                )),
                                ..Default::default()
                            },
                        },
                        Extent3d {
                            width: camera.image_size,
                            height: camera.image_size,
                            depth_or_array_layers: 1,
                        },
                    );

                    let _ = world.resource::<ImpostersBaked>().sender.send((
                        camera.image_size,
                        callback.clone(),
                        buffer,
                    ));
                }

                // copy it to the output
                if let Some(target) = textures.target.as_ref() {
                    command_encoder.copy_texture_to_texture(
                        textures.output.texture.texture.as_image_copy(),
                        target.as_image_copy(),
                        Extent3d {
                            width: camera.image_size,
                            height: camera.image_size,
                            depth_or_array_layers: 1,
                        },
                    );
                }

                // report back
                debug!("send success");
                let _ = camera.channel.send(());
            }

            command_encoder.finish()
        });

        Ok(())
    }
}

pub fn copy_back(baked: Res<ImpostersBaked>) {
    while let Ok((image_size, callback, buffer)) = baked.receiver.try_recv() {
        debug!("begin async process");

        let Some(callback) = callback.lock().unwrap().take() else {
            warn!("imposter callback already taken?!");
            continue;
        };

        let finish = async move {
            let (tx, rx) = async_channel::bounded(1);
            let buffer_slice = buffer.slice(..);
            // The polling for this map call is done every frame when the command queue is submitted.
            buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
                let err = result.err();
                if err.is_some() {
                    panic!("{}", err.unwrap().to_string());
                }
                tx.try_send(()).unwrap();
            });
            rx.recv().await.unwrap();
            let data = buffer_slice.get_mapped_range();
            // we immediately move the data to CPU memory to avoid holding the mapped view for long
            let mut result = Vec::from(&*data);
            drop(data);
            drop(buffer);

            let pixel_size = TextureFormat::Rg32Uint.pixel_size();

            if result.len() != (image_size * image_size) as usize * pixel_size {
                // Our buffer has been padded because we needed to align to a multiple of 256.
                // We remove this padding here
                let initial_row_bytes = image_size as usize * pixel_size;
                let buffered_row_bytes = align_byte_size(image_size * pixel_size as u32) as usize;

                let mut take_offset = buffered_row_bytes;
                let mut place_offset = initial_row_bytes;
                for _ in 1..image_size {
                    result.copy_within(take_offset..take_offset + buffered_row_bytes, place_offset);
                    take_offset += buffered_row_bytes;
                    place_offset += initial_row_bytes;
                }
                result.truncate(initial_row_bytes * image_size as usize);
            }

            let image = Image::new(
                Extent3d {
                    width: image_size,
                    height: image_size,
                    depth_or_array_layers: 1,
                },
                wgpu::TextureDimension::D2,
                result,
                TextureFormat::Rg32Uint,
                RenderAssetUsages::all(),
            );

            debug!("callback");
            (callback)(image)
        };

        AsyncComputeTaskPool::get().spawn(finish).detach();
    }
}

pub fn align_byte_size(value: u32) -> u32 {
    value + (wgpu::COPY_BYTES_PER_ROW_ALIGNMENT - (value % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT))
}

pub fn get_aligned_size(width: u32, height: u32, pixel_size: u32) -> u32 {
    height * align_byte_size(width * pixel_size)
}

#[derive(Component, Default, Clone)]
pub struct ImposterExpectedRenderCount(usize);

#[derive(Resource, Default)]
pub struct ImposterActualRenderCount(Arc<Mutex<usize>>);

pub struct CountRenderCommand;
impl<P: PhaseItem> RenderCommand<P> for CountRenderCommand {
    type Param = SRes<ImposterActualRenderCount>;

    type ViewQuery = ();

    type ItemQuery = ();

    fn render<'w>(
        _: &P,
        _: bevy::ecs::query::ROQueryItem<'w, Self::ViewQuery>,
        _: Option<bevy::ecs::query::ROQueryItem<'w, Self::ItemQuery>>,
        count: bevy::ecs::system::SystemParamItem<'w, '_, Self::Param>,
        _: &mut TrackedRenderPass<'w>,
    ) -> bevy::render::render_phase::RenderCommandResult {
        *count.0.lock().unwrap() += 1;
        bevy::render::render_phase::RenderCommandResult::Success
    }
}

pub type DrawImposter<M> = (
    SetItemPipeline,
    SetPrepassViewBindGroup<0>,
    SetMeshBindGroup<1>,
    SetMaterialBindGroup<M, 2>,
    DrawMesh,
    CountRenderCommand,
);
