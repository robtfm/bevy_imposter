use bevy::{
    pbr::MaterialExtension,
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderRef, ShaderType},
};

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
    #[texture(201, dimension = "2d", sample_type = "u_int")]
    // #[texture(201, dimension = "2d")]
    // #[sampler(202)]
    pub image: Handle<Image>,
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
        // AlphaMode::Mask(1.0)
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

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct StandardMaterialImposterMaker {}

impl MaterialExtension for StandardMaterialImposterMaker {
    fn fragment_shader() -> ShaderRef {
        "shaders/standard_material_imposter_baker.wgsl".into()
    }
}
