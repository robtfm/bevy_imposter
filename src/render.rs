use bevy::{
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderRef, ShaderType},
};

use crate::asset_loader::ImposterLoader;

pub struct ImposterRenderPlugin;

impl Plugin for ImposterRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<Imposter>::default())
            .register_asset_loader(ImposterLoader);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum ImposterMode {
    Image,
    Material,
}

#[derive(ShaderType, Clone, Copy, PartialEq, Debug)]
pub struct ImposterData {
    pub center_and_scale: Vec4,
    pub grid_size: u32,
    pub flags: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ImposterKey(ImposterMode);

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
#[bind_group_data(ImposterKey)]
pub struct Imposter {
    #[uniform(200)]
    pub data: ImposterData,
    // material mode
    #[texture(201, dimension = "2d", sample_type = "u_int")]
    pub material: Option<Handle<Image>>,
    // image mode
    // #[texture(202, dimension = "2d")]
    // #[sampler(203)]
    // pub image: Option<Handle<Image>>
    pub mode: ImposterMode,
}

impl From<&Imposter> for ImposterKey {
    fn from(value: &Imposter) -> Self {
        Self(value.mode)
    }
}

impl Material for Imposter {
    fn vertex_shader() -> ShaderRef {
        "shaders/vertex.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/fragment.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn specialize(
        _: &bevy::pbr::MaterialPipeline<Self>,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _: &bevy::render::mesh::MeshVertexBufferLayoutRef,
        key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        match key.bind_group_data.0 {
            ImposterMode::Image => descriptor
                .fragment
                .as_mut()
                .unwrap()
                .shader_defs
                .push("IMPOSTER_IMAGE".into()),
            ImposterMode::Material => descriptor
                .fragment
                .as_mut()
                .unwrap()
                .shader_defs
                .push("IMPOSTER_MATERIAL".into()),
        }
        Ok(())
    }
}
