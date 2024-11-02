use bevy::{
    asset::load_internal_asset, prelude::*, render::render_resource::{AsBindGroup, ShaderRef, ShaderType}
};

use crate::asset_loader::ImposterLoader;

pub const BINDINGS_HANDLE: Handle<Shader> = Handle::weak_from_u128(659996873659996873);
pub const FRAGMENT_HANDLE: Handle<Shader> = Handle::weak_from_u128(656126482580442360);
pub const SHARED_HANDLE: Handle<Shader> = Handle::weak_from_u128(699899997614446892);
pub const VERTEX_HANDLE: Handle<Shader> = Handle::weak_from_u128(591046068481766317);

pub struct ImposterRenderPlugin;

impl Plugin for ImposterRenderPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(app, BINDINGS_HANDLE, "shaders/bindings.wgsl", Shader::from_wgsl);
        load_internal_asset!(app, FRAGMENT_HANDLE, "shaders/fragment.wgsl", Shader::from_wgsl);
        load_internal_asset!(app, SHARED_HANDLE, "shaders/shared.wgsl", Shader::from_wgsl);
        load_internal_asset!(app, VERTEX_HANDLE, "shaders/vertex.wgsl", Shader::from_wgsl);

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
        VERTEX_HANDLE.into()
    }

    fn fragment_shader() -> ShaderRef {
        FRAGMENT_HANDLE.into()
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
