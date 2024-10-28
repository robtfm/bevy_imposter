use bevy::{prelude::*, render::render_resource::{AsBindGroup, ShaderRef, ShaderType}};

#[derive(ShaderType, Clone, Debug)]
pub struct ImposterData {
    pub center_and_scale: Vec4,
    pub grid_size: u32,
    pub flags: u32,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct Imposter {
    #[uniform(200)]
    pub data: ImposterData,
    #[texture(201, dimension = "2d")]
    #[sampler(202)]
    pub image: Handle<Image>,
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
}
