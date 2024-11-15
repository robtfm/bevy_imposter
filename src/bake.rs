use std::{
    ffi::OsStr,
    hash::Hash,
    marker::PhantomData,
    ops::Range,
    path::Path,
    sync::{Arc, Mutex},
};

use bevy::{
    asset::load_internal_asset,
    core_pipeline::{
        core_3d::{AlphaMask3d, Opaque3d, Opaque3dBinKey, Transparent3d},
        fullscreen_vertex_shader::fullscreen_shader_vertex_state,
        prepass::OpaqueNoLightmap3dBinKey,
    },
    ecs::{entity::EntityHashSet, query::QueryFilter, system::lifetimeless::SRes},
    pbr::{
        alpha_mode_pipeline_key, graph::NodePbr, prepare_preprocess_bind_groups, DrawMesh,
        ExtendedMaterial, GpuPreprocessNode, MaterialExtension, MaterialPipelineKey, MeshPipeline,
        MeshPipelineKey, PreparedMaterial, PrepassPipeline, PreprocessBindGroup,
        RenderMaterialInstances, RenderMeshInstances, SetMaterialBindGroup, SetMeshBindGroup,
        SetPrepassViewBindGroup, SkipGpuPreprocess,
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
            binding_types::{texture_2d, uniform_buffer},
            BindGroup, BindGroupEntries, BindGroupLayout, BindGroupLayoutEntries, Buffer,
            BufferDescriptor, CachedRenderPipelineId, ColorTargetState, ColorWrites,
            CommandEncoderDescriptor, Extent3d, FragmentState, PipelineCache, RenderPassDescriptor,
            RenderPipelineDescriptor, ShaderDefVal, ShaderRef, ShaderType, SpecializedMeshPipeline,
            SpecializedMeshPipelines, StoreOp, Texture, TextureDescriptor, TextureDimension,
            TextureFormat, TextureUsages, UniformBuffer,
        },
        renderer::{RenderDevice, RenderQueue},
        texture::{ColorAttachment, GpuImage, ImageSampler, TextureCache, TextureFormatPixelInfo},
        view::{
            ColorGrading, ExtractedView, NoFrustumCulling, RenderLayers, ViewDepthTexture,
            ViewUniformOffset, VisibilitySystems, VisibleEntities, WithMesh,
        },
        Extract, Render, RenderApp, RenderSet,
    },
    tasks::AsyncComputeTaskPool,
    utils::{HashMap, Parallel},
};
use wgpu::{BufferUsages, ImageCopyBuffer, ImageDataLayout, ShaderStages};

use crate::{
    asset_loader::write_asset,
    oct_coords::{normal_from_grid, GridMode},
    ImposterRenderPlugin,
};

pub struct ImposterBakePlugin;

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderSubGraph)]
pub struct ImposterBakeGraph;

pub const STANDARD_BAKE_HANDLE: Handle<Shader> = Handle::weak_from_u128(72833264206534166);
pub const IMPOSTER_BAKE_HANDLE: Handle<Shader> = Handle::weak_from_u128(28332642065341667);
pub const SHARED_HANDLE: Handle<Shader> = Handle::weak_from_u128(699899997614446892);
pub const IMPOSTER_BLIT_HANDLE: Handle<Shader> = Handle::weak_from_u128(269989999761444689);

impl Plugin for ImposterBakePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ImposterRenderPlugin);

        load_internal_asset!(
            app,
            STANDARD_BAKE_HANDLE,
            "shaders/standard_material_imposter_baker.wgsl",
            Shader::from_wgsl
        );
        load_internal_asset!(
            app,
            IMPOSTER_BAKE_HANDLE,
            "shaders/imposter_imposter_baker.wgsl",
            Shader::from_wgsl
        );
        load_internal_asset!(app, SHARED_HANDLE, "shaders/shared.wgsl", Shader::from_wgsl);
        load_internal_asset!(
            app,
            IMPOSTER_BLIT_HANDLE,
            "shaders/imposter_blit.wgsl",
            Shader::from_wgsl
        );

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
            .init_resource::<PartBaked>()
            .add_systems(ExtractSchedule, extract_imposter_cameras)
            .add_systems(
                Render,
                (
                    prepare_imposter_textures.in_set(RenderSet::PrepareResources),
                    prepare_imposter_bindgroups.in_set(RenderSet::PrepareBindGroups),
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
        app.add_plugins(ImposterMaterialPlugin::<crate::Imposter>::default()); // imposterception
    }

    fn finish(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app.init_resource::<ImposterBlitPipeline>();
    }
}

pub trait ImposterBakeMaterial: Material {
    fn imposter_fragment_shader() -> ShaderRef;
}

impl ImposterBakeMaterial for StandardMaterial {
    fn imposter_fragment_shader() -> ShaderRef {
        STANDARD_BAKE_HANDLE.into()
    }
}

impl ImposterBakeMaterial for crate::Imposter {
    fn imposter_fragment_shader() -> ShaderRef {
        IMPOSTER_BAKE_HANDLE.into()
    }
}

pub trait ImposterBakeMaterialExtension: MaterialExtension {
    fn imposter_fragment_shader() -> ShaderRef;
}

pub struct ImposterMaterialPlugin<M: ImposterBakeMaterial> {
    _p: PhantomData<fn() -> M>,
}

impl<M: ImposterBakeMaterial> Default for ImposterMaterialPlugin<M> {
    fn default() -> Self {
        Self {
            _p: Default::default(),
        }
    }
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

impl<B: Material, E: MaterialExtension + ImposterBakeMaterialExtension> ImposterBakeMaterial
    for ExtendedMaterial<B, E>
{
    fn imposter_fragment_shader() -> ShaderRef {
        E::imposter_fragment_shader()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BakeState {
    Rendering,
    RunningCallback,
    Finished,
}

#[derive(Component, Clone)]
pub struct ImposterBakeCamera {
    // area to capture
    pub radius: f32,
    // number of snapshots to pack
    pub grid_size: u32,
    // image size per tile
    pub tile_size: u32,
    // number of samples to average over (power of 1,2,4,8,etc)
    pub multisample: u32,
    // camera angles to use for snapshots
    pub grid_mode: GridMode,
    // optional output, can be used in a material for dynamic imposters or previews
    pub target: Option<Handle<Image>>,
    // camera order, for dynamic should be less than your 3d camera
    pub order: isize,
    // whether to snapshot every frame or stop after a single successful snapshot
    pub continuous: bool,
    // whether to wait for all visible entities to be renderable (pipelines compiled, mesh/material data transferred to gpu)
    pub wait_for_render: bool,
    // max number of tiles to render in a single frame
    pub max_tiles_per_frame: usize,
    // signal for completion (if not continuous) - written by the library
    pub state: BakeState,
    // optional callback for completion
    pub callback: Option<ImageCallback>,
    // optional custom camera positions, for using the baking infrastructure to generate your own layouts
    // needs to be combined with a custom frag shader
    pub manual_camera_transforms: Option<Vec<GlobalTransform>>,
}

impl Default for ImposterBakeCamera {
    fn default() -> Self {
        Self {
            radius: 1.0,
            grid_size: 8,
            tile_size: 64,
            multisample: 8,
            grid_mode: GridMode::Spherical,
            target: None,
            order: -99,
            continuous: false,
            wait_for_render: true,
            max_tiles_per_frame: usize::MAX,
            state: BakeState::Rendering,
            callback: None,
            manual_camera_transforms: None,
        }
    }
}

impl ImposterBakeCamera {
    // create a target image of the right format and size
    pub fn init_target(&mut self, images: &mut Assets<Image>) {
        let size = Extent3d {
            width: self.tile_size * self.grid_size,
            height: self.tile_size * self.grid_size,
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

    // add a callback to be run on completion
    pub fn set_callback(&mut self, callback: impl FnOnce(Image) + Send + Sync + 'static) {
        self.callback = Some(Arc::new(Mutex::new(Some(Box::new(callback)))));
    }

    // Returns an async fn that can be set as the callback to save the asset once baked.
    // warning: uses the current camera state - changes after this call will not be reflected
    // shrink_asset will pack the texture more tightly saving vram, but is slower.
    pub fn save_asset_callback(
        &self,
        path: impl AsRef<Path>,
        shrink_asset: bool,
    ) -> impl FnOnce(bevy::prelude::Image) + Send + Sync + 'static {
        let mut path = path.as_ref().to_owned();
        if path.extension() != Some(OsStr::new("boimp")) {
            path.set_extension("boimp");
        }

        let grid_size = self.grid_size;
        let tile_size = self.tile_size;
        let radius = self.radius;
        let mode = self.grid_mode;
        move |image| {
            if let Err(e) = write_asset(
                &path,
                radius,
                grid_size,
                tile_size,
                mode,
                image,
                shrink_asset,
            ) {
                error!("error writing imposter asset: {e}");
            } else {
                info!("imposter saved");
            }
        }
    }
}

#[derive(Component)]
pub struct ImposterBakeCompleteChannel {
    sender: crossbeam_channel::Sender<BakeState>,
    receiver: Option<crossbeam_channel::Receiver<BakeState>>,
}

impl Default for ImposterBakeCompleteChannel {
    fn default() -> Self {
        let (sender, receiver) = crossbeam_channel::bounded(2); // make sure we don't block rendering
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

#[derive(Resource, Default)]
pub struct PartBaked(Arc<Mutex<HashMap<Entity, usize>>>);

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
        visible_entities.clear::<QF>();

        if !camera.continuous && camera.state == BakeState::Finished {
            return;
        }

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

        thread_queues.drain_into(visible_entities.get_mut::<QF>());
        expected_count.0 = 0;
    }
}

#[allow(clippy::type_complexity)]
fn count_expected_imposter_materials<M: ImposterBakeMaterial>(
    mut q: Query<(&mut ImposterExpectedRenderCount, &VisibleEntities), With<ImposterBakeCamera>>,
    materials: Query<(), (With<Handle<M>>, With<Handle<Mesh>>)>,
) {
    for (mut count, visible_entities) in q.iter_mut() {
        let material_count = visible_entities
            .iter::<WithMesh>()
            .filter(|e| materials.get(**e).is_ok())
            .count();
        count.0 += material_count;
        debug!(
            "bake entities {}: {}",
            std::any::type_name::<M>(),
            material_count
        );
    }
}

#[derive(Component)]
pub struct ExtractedImposterBakeCamera {
    pub grid_size: u32,
    pub tile_size: u32,
    pub multisample: u32,
    pub target: Option<Handle<Image>>,
    pub subviews: Vec<(u32, u32, Entity)>,
    pub expected_count: usize,
    pub wait_for_render: bool,
    pub max_tiles_per_frame: usize,
    pub channel: crossbeam_channel::Sender<BakeState>,
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
        while let Some(new_state) = receiver.receiver.as_ref().and_then(|r| r.try_recv().ok()) {
            if !cam.continuous {
                debug!("recv state: {new_state:?}");
                cam.state = new_state;

                if new_state == BakeState::Finished {
                    commands.entity(ent).remove::<ImposterBakeCompleteChannel>();
                }
            }
        }
    }
}

pub type ImageCallback = Arc<Mutex<Option<Box<dyn FnOnce(Image) + Send + Sync + 'static>>>>;

#[derive(Resource)]
pub struct ImpostersBaked {
    sender: crossbeam_channel::Sender<(
        u32,
        ImageCallback,
        crossbeam_channel::Sender<BakeState>,
        Buffer,
    )>,
    receiver: crossbeam_channel::Receiver<(
        u32,
        ImageCallback,
        crossbeam_channel::Sender<BakeState>,
        Buffer,
    )>,
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
    part_baked: Res<PartBaked>,
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
        if camera.state != BakeState::Rendering
            || !channel.receiver.as_ref().map_or(true, |r| r.is_empty())
        {
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
        for y in 0..camera.grid_size {
            for x in 0..camera.grid_size {
                let camera_transform =
                    if let Some(camera_transforms) = camera.manual_camera_transforms.as_ref() {
                        *camera_transforms
                            .get((y * camera.grid_size + x) as usize)
                            .expect("not enough manual camera transforms")
                    } else {
                        let (normal, up) =
                            normal_from_grid(UVec2::new(x, y), camera.grid_mode, camera.grid_size);
                        GlobalTransform::from(
                            Transform::from_translation(center + normal * camera.radius)
                                .looking_at(center, up),
                        )
                    };

                let view = ExtractedView {
                    clip_from_view,
                    world_from_view: camera_transform,
                    clip_from_world: None,
                    hdr: false,
                    viewport: UVec4::new(
                        0,
                        0,
                        camera.tile_size * camera.grid_size,
                        camera.tile_size * camera.grid_size,
                    ),
                    color_grading: ColorGrading::default(),
                };

                let id = commands.spawn(view).id();

                subviews.push((x, y, id));
            }
        }

        commands.get_or_spawn(entity).insert((
            ExtractedImposterBakeCamera {
                grid_size: camera.grid_size,
                tile_size: camera.tile_size,
                target: camera.target.clone(),
                multisample: camera.multisample,
                subviews,
                expected_count: expected_count.0,
                wait_for_render: camera.wait_for_render,
                max_tiles_per_frame: camera.max_tiles_per_frame,
                channel: channel.sender.clone(),
                callback: camera.callback.clone(),
            },
            ExtractedCamera {
                target: None,
                physical_viewport_size: Some(UVec2::splat(camera.tile_size * camera.grid_size)),
                physical_target_size: Some(UVec2::splat(camera.tile_size * camera.grid_size)),
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
    part_baked
        .0
        .lock()
        .unwrap()
        .retain(|entity, _| entities.contains(entity));
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

        // add DEPTH_CLAMP_ORTHO to force fragment shader
        let key = MaterialPipelineKey {
            mesh_key: key.mesh_key.union(MeshPipelineKey::DEPTH_CLAMP_ORTHO),
            bind_group_data: key.bind_group_data,
        };

        let mut descriptor = self.prepass_pipeline.specialize(key, layout)?;
        descriptor.label =
            Some(format!("imposter_bake_pipeline {}", std::any::type_name::<M>()).into());

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
            "VIEW_PROJECTION_ORTHOGRAPHIC".into(),
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

        let mut frag_defs = descriptor
            .fragment
            .map(|f| f.shader_defs)
            .clone()
            .unwrap_or_default();
        frag_defs.retain(|d| match d {
            ShaderDefVal::Bool(key, _) => !matches!(
                key.as_str(),
                "DEPTH_PREPASS" | "NORMAL_PREPASS" | "MOTION_VECTOR_PREPASS"
            ),
            _ => true,
        });
        frag_defs.extend([
            "IMPOSTER_BAKE_PIPELINE".into(),
            "PREPASS_FRAGMENT".into(),
            "DEPTH_CLAMP_ORTHO".into(),
            "DEFERRED_PREPASS".into(),
            "NORMAL_PREPASS_OR_DEFERRED_PREPASS".into(),
            "VIEW_PROJECTION_ORTHOGRAPHIC".into(),
        ]);

        // replace frag state
        descriptor.fragment = Some(FragmentState {
            shader: self.frag_shader.clone(),
            shader_defs: frag_defs,
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

#[derive(ShaderType)]
pub struct BlitUniform {
    samples: u32,
}

#[derive(Resource)]
pub struct ImposterBlitPipeline {
    layout: BindGroupLayout,
    pipeline: CachedRenderPipelineId,
}

impl FromWorld for ImposterBlitPipeline {
    fn from_world(world: &mut World) -> Self {
        let device = world.resource::<RenderDevice>();
        let pipeline_cache = world.resource::<PipelineCache>();

        let layout = device.create_bind_group_layout(
            "imposter_blit_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    texture_2d(wgpu::TextureSampleType::Uint),
                    uniform_buffer::<BlitUniform>(false),
                ),
            ),
        );

        let pipeline = pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some("imposter_blit_render_pipeline".into()),
            layout: vec![layout.clone()],
            vertex: fullscreen_shader_vertex_state(),
            fragment: Some(FragmentState {
                shader: IMPOSTER_BLIT_HANDLE,
                shader_defs: Vec::default(),
                entry_point: "blend_materials".into(),
                targets: vec![Some(ColorTargetState {
                    format: TextureFormat::Rg32Uint,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
            }),
            depth_stencil: None,
            push_constant_ranges: Default::default(),
            primitive: Default::default(),
            multisample: Default::default(),
        });

        Self { layout, pipeline }
    }
}

#[derive(Component)]
pub struct ImposterResources {
    pub output: ColorAttachment,
    pub intermediate: Option<ColorAttachment>,
    pub depth: ViewDepthTexture,
    pub target: Option<Texture>,
    pub blit_buffer: Option<UniformBuffer<BlitUniform>>,
    pub blit_bindgroup: Option<BindGroup>,
}

#[allow(clippy::too_many_arguments)]
pub fn prepare_imposter_textures(
    mut commands: Commands,
    mut texture_cache: ResMut<TextureCache>,
    render_device: Res<RenderDevice>,
    opaque_phases: Res<ViewBinnedRenderPhases<ImposterPhaseItem<Opaque3d>>>,
    images: Res<RenderAssets<GpuImage>>,
    views: Query<(Entity, &ExtractedImposterBakeCamera)>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    for (entity, camera) in views.iter() {
        if !opaque_phases.contains_key(&entity) {
            continue;
        }

        let final_size = Extent3d {
            width: camera.tile_size * camera.grid_size,
            height: camera.tile_size * camera.grid_size,
            depth_or_array_layers: 1,
        };
        let intermediate_size = camera.tile_size * camera.multisample;
        let intermediate_size = match camera.multisample {
            1 => final_size,
            _ => Extent3d {
                width: intermediate_size,
                height: intermediate_size,
                depth_or_array_layers: 1,
            },
        };

        let descriptor = TextureDescriptor {
            label: Some("imposter_texture"),
            size: final_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rg32Uint,
            usage: TextureUsages::COPY_SRC
                | TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        };
        let texture = texture_cache.get(&render_device, descriptor);

        let (intermediate, blit_buffer) = match camera.multisample {
            1 => (None, None),
            _ => {
                let descriptor = TextureDescriptor {
                    label: Some("imposter_texture"),
                    size: intermediate_size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: TextureFormat::Rg32Uint,
                    usage: TextureUsages::COPY_SRC
                        | TextureUsages::RENDER_ATTACHMENT
                        | TextureUsages::TEXTURE_BINDING
                        | TextureUsages::STORAGE_BINDING,
                    view_formats: &[],
                };
                let mut buffer: UniformBuffer<BlitUniform> = UniformBuffer::from(BlitUniform {
                    samples: camera.multisample,
                });
                buffer.write_buffer(&device, &queue);

                (
                    Some(texture_cache.get(&render_device, descriptor)),
                    Some(buffer),
                )
            }
        };

        let depth_descriptor = TextureDescriptor {
            label: Some("imposter_depth"),
            size: intermediate_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Depth32Float,
            usage: TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        };
        let depth_texture = texture_cache.get(&render_device, depth_descriptor);

        commands.entity(entity).insert(ImposterResources {
            output: ColorAttachment::new(texture, None, Some(LinearRgba::BLACK)),
            intermediate: intermediate
                .map(|i| ColorAttachment::new(i, None, Some(LinearRgba::BLACK))),
            depth: ViewDepthTexture::new(depth_texture, Some(0.0)),
            target: camera
                .target
                .as_ref()
                .and_then(|target| images.get(target.id()))
                .map(|image| image.texture.clone()),
            blit_buffer,
            blit_bindgroup: None,
        });
    }
}

pub fn prepare_imposter_bindgroups(
    mut q: Query<(&mut ImposterResources, &ExtractedImposterBakeCamera)>,
    device: Res<RenderDevice>,
    pipeline: Res<ImposterBlitPipeline>,
) {
    for (mut res, camera) in q.iter_mut() {
        if camera.multisample > 1 {
            let bindgroup = device.create_bind_group(
                "imposter_blit_group",
                &pipeline.layout,
                &BindGroupEntries::sequential((
                    &res.intermediate.as_ref().unwrap().texture.default_view,
                    res.blit_buffer.as_ref().unwrap().binding().unwrap().clone(),
                )),
            );

            res.blit_bindgroup = Some(bindgroup);
        }
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

            // todo: investigate using A2C?
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
                MeshPipelineKey::BLEND_OPAQUE | MeshPipelineKey::BLEND_ALPHA_TO_COVERAGE => {
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
                            // since we share the mesh bindgroup this will be wrong for some views whatever we use.
                            // todo: use oit?
                            distance: 0.0,
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
        &'static ImposterResources,
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

        let blit_pipeline = world.resource::<ImposterBlitPipeline>();
        let pipeline_cache = world.resource::<PipelineCache>();
        let Some(pipeline) = pipeline_cache.get_render_pipeline(blit_pipeline.pipeline) else {
            return Ok(());
        };

        let actual = world.resource::<ImposterActualRenderCount>();

        let part_baked = world.resource::<PartBaked>();

        render_context.add_command_buffer_generation_task(move |render_device| {
            // we are counting on a shared resource, so have to take a unique lock within the task to ensure it
            // doesn't fail when multiple bake cameras exist.
            // probably a better way to do this
            let _parallel_lock = actual.1.lock().unwrap();
            let mut part_baked = part_baked.0.lock().unwrap();
            *actual.0.lock().unwrap() = 0;

            let mut command_encoder =
                render_device.create_command_encoder(&CommandEncoderDescriptor {
                    label: Some("imposter_command_encoder"),
                });

            let mut rendered = part_baked.get(&view).copied().unwrap_or_default();

            if camera.multisample == 1 {
                if rendered > 0 {
                    // grab the attachments once to disable clearing
                    textures.output.get_attachment();
                    textures.depth.get_attachment(StoreOp::Store);
                }

                // use a single renderpass
                // Render pass setup
                let render_pass = command_encoder.begin_render_pass(&RenderPassDescriptor {
                    label: Some("imposter_bake"),
                    color_attachments: &[Some(textures.output.get_attachment())],
                    depth_stencil_attachment: Some(textures.depth.get_attachment(StoreOp::Store)),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                let mut render_pass = TrackedRenderPass::new(&render_device, render_pass);

                if rendered == 0 {
                    // run once to check if all the items are ready and rendering

                    render_pass.set_viewport(
                        0.0,
                        0.0,
                        camera.tile_size as f32,
                        camera.tile_size as f32,
                        0.0,
                        1.0,
                    );
                    // we use the batch from the dummy main view, which means items will be rendered potentially out of order
                    // TODO: see if it's worth binning for every individual view separately. since this is baking, probably not for opaque.
                    // if we use it for dynamic imposters in future there'd only be a single view being rendered anyway
                    opaque_phase.render(&mut render_pass, world, camera.subviews[0].2);
                    alphamask_phase.render(&mut render_pass, world, camera.subviews[0].2);
                    transparent_phase.render(&mut render_pass, world, camera.subviews[0].2);

                    let actual = *actual.0.lock().unwrap();

                    if actual != camera.expected_count && camera.wait_for_render {
                        debug!("not ready: {}/{}", actual, camera.expected_count);
                    } else {
                        rendered += 1;
                    }
                }

                if rendered > 0 {
                    for (x, y, view) in camera
                        .subviews
                        .iter()
                        .skip(rendered)
                        .take(camera.max_tiles_per_frame)
                    {
                        render_pass.set_viewport(
                            (*x * camera.tile_size) as f32,
                            (*y * camera.tile_size) as f32,
                            camera.tile_size as f32,
                            camera.tile_size as f32,
                            0.0,
                            1.0,
                        );
                        opaque_phase.render(&mut render_pass, world, *view);
                        alphamask_phase.render(&mut render_pass, world, *view);
                        transparent_phase.render(&mut render_pass, world, *view);
                        rendered += 1;
                    }
                }

                drop(render_pass);
            } else {
                // manual multisample resolve requires multiple passes
                let should_clear = rendered == 0;

                // store the attachments so we keep the initial clears
                let color_attachments = [Some(
                    textures.intermediate.as_ref().unwrap().get_attachment(),
                )];
                let depth_attachment = Some(textures.depth.get_attachment(StoreOp::Store));

                for (x, y, view) in camera
                    .subviews
                    .iter()
                    .skip(rendered)
                    .take(camera.max_tiles_per_frame)
                {
                    // Render pass setup
                    let render_pass = command_encoder.begin_render_pass(&RenderPassDescriptor {
                        label: Some("imposter_bake"),
                        color_attachments: &color_attachments,
                        depth_stencil_attachment: depth_attachment.clone(),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                    let mut render_pass = TrackedRenderPass::new(&render_device, render_pass);
                    opaque_phase.render(&mut render_pass, world, *view);
                    alphamask_phase.render(&mut render_pass, world, *view);
                    transparent_phase.render(&mut render_pass, world, *view);

                    if rendered == 0 {
                        let actual = *actual.0.lock().unwrap();
                        let success = actual == camera.expected_count;

                        if !success {
                            debug!("not ready: {}/{}", actual, camera.expected_count);
                            if camera.wait_for_render {
                                break;
                            }
                        }
                    }

                    drop(render_pass);
                    rendered += 1;

                    // copy it
                    if !should_clear {
                        // grab the attachments once to disable clearing
                        textures.output.get_attachment();
                    }
                    let mut pass = command_encoder.begin_render_pass(&RenderPassDescriptor {
                        label: Some("imposter_blit"),
                        color_attachments: &[Some(textures.output.get_attachment())],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });

                    pass.set_viewport(
                        (*x * camera.tile_size) as f32,
                        (*y * camera.tile_size) as f32,
                        camera.tile_size as f32,
                        camera.tile_size as f32,
                        0.0,
                        1.0,
                    );

                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(0, textures.blit_bindgroup.as_ref().unwrap(), &[]);
                    pass.draw(0..3, 0..1);
                }
            }

            part_baked.insert(view, rendered);
            debug!(
                "{:?} -> {}/{}",
                view,
                rendered,
                camera.grid_size * camera.grid_size
            );
            if rendered as u32 == camera.grid_size * camera.grid_size {
                part_baked.remove(&view);
                if let Some(callback) = camera.callback.as_ref() {
                    debug!("send callback buffer");
                    let render_device = world.resource::<RenderDevice>();

                    let buffer = render_device.create_buffer(&BufferDescriptor {
                        label: Some("imposter transfer buffer"),
                        size: get_aligned_size(
                            camera.tile_size * camera.grid_size,
                            camera.tile_size * camera.grid_size,
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
                                    camera.tile_size * camera.grid_size,
                                    1,
                                    TextureFormat::Rg32Uint.pixel_size() as u32,
                                )),
                                ..Default::default()
                            },
                        },
                        Extent3d {
                            width: camera.tile_size * camera.grid_size,
                            height: camera.tile_size * camera.grid_size,
                            depth_or_array_layers: 1,
                        },
                    );

                    // report back
                    debug!("send state::callback");
                    if let Err(e) = camera.channel.send(BakeState::RunningCallback) {
                        warn!("error sending state: {e}");
                    }

                    let _ = world.resource::<ImpostersBaked>().sender.send((
                        camera.tile_size * camera.grid_size,
                        callback.clone(),
                        camera.channel.clone(),
                        buffer,
                    ));
                } else {
                    // report back
                    debug!("no callback, send success");
                    if let Err(e) = camera.channel.send(BakeState::Finished) {
                        warn!("error sending state: {e}");
                    }
                }

                // copy it to the output
                if let Some(target) = textures.target.as_ref() {
                    command_encoder.copy_texture_to_texture(
                        textures.output.texture.texture.as_image_copy(),
                        target.as_image_copy(),
                        Extent3d {
                            width: camera.tile_size * camera.grid_size,
                            height: camera.tile_size * camera.grid_size,
                            depth_or_array_layers: 1,
                        },
                    );
                }
            }

            command_encoder.finish()
        });

        Ok(())
    }
}

pub fn copy_back(baked: Res<ImpostersBaked>) {
    while let Ok((image_size, callback, success_channel, buffer)) = baked.receiver.try_recv() {
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
            (callback)(image);

            debug!("post-callback send success");
            if let Err(e) = success_channel.send(BakeState::Finished) {
                warn!("error sending state: {e}");
            }
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
pub struct ImposterActualRenderCount(Arc<Mutex<usize>>, Arc<Mutex<()>>);

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
