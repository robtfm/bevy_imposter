const IMPOSTER_FLAG_HEMISPHERICAL: u32 = 1;

struct ImposterData {
    center_and_scale: vec4<f32>,
    grid_size: u32,
    flags: u32,
}

@group(2) @binding(200)
var<uniform> imposter_data: ImposterData;
@group(2) @binding(201) 
var imposter_texture: texture_2d<f32>;
@group(2) @binding(202) 
var imposter_sampler: sampler;

struct ImposterVertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) camera_direction: vec3<f32>,
    @location(2) base_world_position: vec3<f32>,
}

