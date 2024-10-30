#import bevy_pbr::{
    pbr_types::{PbrInput, STANDARD_MATERIAL_FLAGS_UNLIT_BIT, pbr_input_new},
    view_transformations::{position_ndc_to_world, frag_coord_to_ndc},
    pbr_functions::calculate_view,
    mesh_view_bindings::view,
};



const GRID_HEMISPHERICAL: u32 = 1;

struct ImposterData {
    center_and_scale: vec4<f32>,
    grid_size: u32,
    flags: u32,
}

struct ImposterVertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) camera_direction: vec3<f32>,
    @location(2) base_world_position: vec3<f32>,
}

fn spherical_uv_from_normal(dir: vec3<f32>) -> vec2<f32> {
    let octant: vec3<f32> = sign(dir);
    let sum: f32 = dot(dir, octant);
    let octahedron: vec3<f32> = dir / sum;
    let absolute: vec3<f32> = abs(octahedron);
    return (select(octahedron.xz, octant.xz * vec2(1.0 - absolute.z, 1.0 - absolute.x), octahedron.y < 0.0) + 1.0) * 0.5;
}

fn spherical_normal_from_uv(uv: vec2<f32>) -> vec3<f32> {
    let x = uv.x * 2.0 - 1.0;
    let z = uv.y * 2.0 - 1.0;
    let y = 1.0 - abs(x) - abs(z);

    let n = select(
        vec3(x, y, z),
        vec3(sign(x) * (1.0 - abs(z)), y, sign(z) * (1.0 - abs(x))),
        y < 0.0
    );
    return normalize(n);
}


// rg32uint
// r: [0-5] r, [6-10] g, [11-15] b, [16] a, [17-24] roughness, [25-32] metallic
// g: [0-24] normal, [25-32] flags (unlit etc)

// pack
fn pack_bits(input: f32, offset: u32, count: u32) -> u32 {
    let mask = (1u << count) - 1u;
    return u32(saturate(input) * f32(mask) + 0.5) << offset;
}

fn pack_normal_and_flags(normal: vec3<f32>, flags: u32) -> u32 {
    let octahedral_normal = spherical_uv_from_normal(normal);
    return 
        pack_bits(octahedral_normal.x, 0u, 12u) + 
        pack_bits(octahedral_normal.y, 12u, 12u) +
        (flags << 24u);
}

fn pack_rgba_roughness_metallic(albedo: vec4<f32>, roughness: f32, metallic: f32) -> u32 {
    return 
        pack_bits(albedo.r, 0u, 5u) +
        pack_bits(albedo.g, 5u, 5u) +
        pack_bits(albedo.b, 10u, 5u) +
        pack_bits(albedo.a, 15u, 1u) +
        pack_bits(roughness, 16u, 8u) +
        pack_bits(metallic, 24u, 8u);
}

fn pack_pbrinput(input: PbrInput) -> vec2<u32> {
    return vec2<u32>(
        pack_rgba_roughness_metallic(input.material.base_color, input.material.perceptual_roughness, input.material.metallic),
        pack_normal_and_flags(input.world_normal, u32((input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) != 0u))
    );
}

// unpack
fn unpack_bits(input: u32, offset: u32, count: u32) -> f32 {
    let mask = (1u << count) - 1u;
    return f32(((input >> offset) & mask)) / f32(mask);
}

fn unpack_normal(input: u32) -> vec3<f32> {
    return spherical_normal_from_uv(vec2<f32>(
        unpack_bits(input, 0u, 12u),
        unpack_bits(input, 12u, 12u),
    ));
}

fn unpack_flags(input: u32) -> u32 {
    return (input >> 24u) & 0xFF;
}

fn unpack_rgba(input: u32) -> vec4<f32> {
    return vec4<f32>(
        unpack_bits(input, 0u, 5u),
        unpack_bits(input, 5u, 5u),
        unpack_bits(input, 10u, 5u),
        unpack_bits(input, 15u, 1u),
    );
}

fn unpack_roughness(input: u32) -> f32 {
    return unpack_bits(input, 16u, 8u);
}

fn unpack_metallic(input: u32) -> f32 {
    return unpack_bits(input, 24u, 8u);
}

fn unpack_pbrinput(packed: vec2<u32>, frag_coord: vec4<f32>) -> PbrInput {
    // let packed = bitcast<u32>(packed_f32);
    var input = pbr_input_new();

    input.material.base_color = unpack_rgba(packed.r);
    input.material.perceptual_roughness = unpack_roughness(packed.r);
    input.material.metallic = unpack_metallic(packed.r);


    let flags = unpack_flags(packed.g);
    if flags != 0u {
        input.material.flags |= STANDARD_MATERIAL_FLAGS_UNLIT_BIT;
    }

    input.N = unpack_normal(packed.g);
    input.world_normal = input.N;
    input.frag_coord = frag_coord;
    input.world_position = vec4(position_ndc_to_world(frag_coord_to_ndc(frag_coord)), 1.0);
    input.is_orthographic = view.clip_from_view[3].w == 1.0;
    input.V = calculate_view(input.world_position, input.is_orthographic);

    return input;
}
