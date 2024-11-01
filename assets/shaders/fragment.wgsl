#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
}

#import "shaders/shared.wgsl"::{ImposterVertexOut, unpack_pbrinput, unpack_rgba, unpack_normal, unpack_roughness, unpack_metallic};
#import "shaders/bindings.wgsl"::{imposter_data, imposter_texture, imposter_sampler, oct_mode_uv_from_normal, grid_weights, sample_tile, sample_tile_material};

@fragment
fn fragment(in: ImposterVertexOut) -> FragmentOutput {
    var out: FragmentOutput;
    let grid_count = f32(imposter_data.grid_size);

    var grid_pos = oct_mode_uv_from_normal(in.camera_direction) * (grid_count - 1);
    let grid_index = clamp(floor(grid_pos), vec2(0.0), vec2(grid_count - 2.0));
    let frac = clamp(grid_pos - grid_index, vec2(0.0), vec2(1.0));

    let weights = grid_weights(frac);

    let inv_rot = mat3x3(
        in.inverse_rotation_0c,
        in.inverse_rotation_1c,
        in.inverse_rotation_2c,
    );

#ifdef IMPOSTER_IMAGE
    let sample_tl = sample_tile(in.base_world_position, in.world_position, grid_index);
    let corner_offset = select(vec2(1.0, 0.0), vec2(0.0, 1.0), weights.y > 0.0);
    let sample_corner = sample_tile(in.base_world_position, in.world_position, grid_index + corner_offset);
    let sample_br = sample_tile(in.base_world_position, in.world_position, grid_index + vec2(1.0, 1.0));
    let sample = sample_tl * weights.x + sample_corner * (weights.y + weights.z) + sample_br * weights.w;

    if sample.a < 0.5 {
        discard;
    }

    out.color = clamp(sample, vec4(0.0), vec4(1.0));
#endif

#ifdef IMPOSTER_MATERIAL
    let sample_tl = sample_tile_material(in.base_world_position, in.world_position, inv_rot, grid_index);
    let corner_offset = select(vec2(1.0, 0.0), vec2(0.0, 1.0), weights.z > 0.0);
    let sample_corner = sample_tile_material(in.base_world_position, in.world_position, inv_rot, grid_index + corner_offset);
    let sample_br = sample_tile_material(in.base_world_position, in.world_position, inv_rot, grid_index + vec2(1.0, 1.0));

    var pbr_input = unpack_pbrinput(sample_tl, in.position);

    let color_tl = pbr_input.material.base_color;
    let color_corner = unpack_rgba(sample_corner.x);
    let color_br = unpack_rgba(sample_br.x);

    var total_weight = 0.0;
    var total_color = vec4<f32>(0.0);
    var total_normal = vec3<f32>(0.0);
    var total_rough_metallic = vec2<f32>(0.0);

    if color_tl.a > 0.0 {
        total_weight += weights.x;
        total_color += color_tl * weights.x;
        total_normal += pbr_input.N * weights.x;
        total_rough_metallic += vec2(pbr_input.material.perceptual_roughness, pbr_input.material.metallic) * weights.x;
    }
    if color_corner.a > 0.0 {
        total_weight += weights.y + weights.z;
        total_color += color_corner * (weights.y + weights.z);
        total_normal += unpack_normal(sample_corner.g) * (weights.y + weights.z);
        total_rough_metallic += vec2(unpack_roughness(sample_corner.r), unpack_metallic(sample_corner.r)) * (weights.y + weights.z);
    }
    if color_br.a > 0.0 {
        total_weight += weights.w;
        total_color += color_br * weights.w;
        total_normal += unpack_normal(sample_br.g) * weights.w;
        total_rough_metallic += vec2(unpack_roughness(sample_br.r), unpack_metallic(sample_br.r)) * weights.w;
    }
    if total_weight == 0.0 {
        discard;
    }

    pbr_input.material.base_color = vec4(total_color.xyz / total_weight, total_color.a);
    pbr_input.material.perceptual_roughness = total_rough_metallic.x / total_weight;
    pbr_input.material.metallic = total_rough_metallic.y / total_weight;
    
    pbr_input.N = inv_rot * normalize(total_normal / total_weight);
    pbr_input.world_normal = pbr_input.N;

    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr_input);
    } else {
        out.color = pbr_input.material.base_color;
    }

    // if out.color.a < 0.5 {
    //     out.color = vec4<f32>(0.1, 0.0, 0.0, 1.0);
    // }

    // if grid_index.x < 0.0 {
    //     out.color = vec4<f32>(0.0, 1.0, 0.0, 1.0);
    // }

    // out.color = clamp(out.color, vec4(0.0, 0.0, 0.0, 1.0), vec4(1.0));
    // out.color = vec4<f32>(f32(sample_tl.x) / f32(0xFFFFFFFF), f32(sample_tl.y) / f32(0xFFFFFFFF), 0.0, 1.0);
#endif

    return out;
}
