#define_import_path boimp::shared

#import bevy_pbr::{
    pbr_types::{PbrInput, STANDARD_MATERIAL_FLAGS_UNLIT_BIT, pbr_input_new},
    view_transformations::{position_ndc_to_world, frag_coord_to_ndc},
    pbr_functions::calculate_view,
    mesh_view_bindings::view,
};

struct ImposterData {
    center_and_scale: vec4<f32>,
    packed_offset: vec2<u32>,
    packed_size: vec2<u32>,
    grid_size: u32,
    base_tile_size: u32,
    flags: u32,
    alpha: f32,
}

struct ImposterVertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) camera_direction: vec3<f32>,
    @location(2) base_world_position: vec3<f32>,
    @location(3) inverse_rotation_0c: vec3<f32>,
    @location(4) inverse_rotation_1c: vec3<f32>,
    @location(5) inverse_rotation_2c: vec3<f32>,
    @location(6) uv_ab: vec4<f32>,
    @location(7) uv_c: vec2<f32>,
}

struct UnpackedMaterialProps {
    rgba: vec4<f32>,
    normal: vec3<f32>,
    roughness: f32,
    metallic: f32,
    flags: u32,
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

fn normalize_or_zero(in: vec3<f32>) -> vec3<f32> {
    let len = length(in);
    return select(in / len, vec3(0.0), len < 0.00001);
}
// rg32uint
// r: [0-4] r, [5-9] g, [10-14] b, [15] a, [16-23] roughness, [24-31] metallic
// g: [0-23] normal, [24-31] flags (unlit etc)

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
        pack_bits(albedo.a, 15u, 5u) +
        pack_bits(roughness, 20u, 6u) +
        pack_bits(metallic, 26u, 6u);
}

fn pack_props(input: UnpackedMaterialProps) -> vec2<u32> {
    return vec2<u32>(
        pack_rgba_roughness_metallic(input.rgba, input.roughness, input.metallic),
        pack_normal_and_flags(input.normal, input.flags)
    );
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
        unpack_bits(input, 15u, 5u),
    );
}

fn unpack_roughness(input: u32) -> f32 {
    return clamp(unpack_bits(input, 20u, 6u), 0.1, 0.9);
}

fn unpack_metallic(input: u32) -> f32 {
    return clamp(unpack_bits(input, 26u, 6u), 0.1, 0.9);
}

fn unpack_props(packed: vec2<u32>) -> UnpackedMaterialProps {
    var props: UnpackedMaterialProps;
    props.rgba = unpack_rgba(packed.r);
    props.roughness = unpack_roughness(packed.r);
    props.metallic = unpack_metallic(packed.r);
    props.normal = unpack_normal(packed.g);
    props.flags = unpack_flags(packed.g);
    return props;
}

fn weighted_props(a: UnpackedMaterialProps, b: UnpackedMaterialProps, weight_a: f32) -> UnpackedMaterialProps {
    var out: UnpackedMaterialProps;

    let raw_wa = a.rgba.a * weight_a;
    let raw_wb = b.rgba.a * (1.0 - weight_a);
    let total_weight = raw_wa + raw_wb;
    if total_weight == 0.0 {
        return out;
    }

    let wa = raw_wa / total_weight;
    let wb = raw_wb / total_weight;

    out.rgba = vec4(a.rgba.rgb * wa + b.rgba.rgb * wb, a.rgba.a * weight_a + b.rgba.a * (1.0 - weight_a));
    out.roughness = a.roughness * wa + b.roughness * wb;
    out.metallic = a.metallic * wa + b.metallic * wb;
    out.normal = normalize_or_zero(a.normal * wa + b.normal * wb);
    out.flags = a.flags;
    return out;
}

fn unpack_pbrinput(props: UnpackedMaterialProps, frag_coord: vec4<f32>) -> PbrInput {
    var input = pbr_input_new();

    input.material.base_color = props.rgba;
    input.material.perceptual_roughness = props.roughness;
    input.material.metallic = props.metallic;

    if props.flags != 0u {
        input.material.flags |= STANDARD_MATERIAL_FLAGS_UNLIT_BIT;
    }

    input.N = props.normal;
    input.world_normal = input.N;
    input.frag_coord = frag_coord;
    input.world_position = vec4(position_ndc_to_world(frag_coord_to_ndc(frag_coord)), 1.0);
    input.is_orthographic = view.clip_from_view[3].w == 1.0;
    input.V = calculate_view(input.world_position, input.is_orthographic);

    return input;
}
