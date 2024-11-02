#import bevy_pbr::{
    forward_io::FragmentOutput,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
}

#import boimp::shared::{ImposterVertexOut, unpack_pbrinput, weighted_props};
#import boimp::bindings::{imposter_data, oct_mode_uv_from_normal, grid_weights, sample_tile, sample_tile_material};

@fragment
fn fragment(in: ImposterVertexOut) -> FragmentOutput {
    var out: FragmentOutput;
    let grid_count = f32(imposter_data.grid_size);

    let grid_pos = oct_mode_uv_from_normal(in.camera_direction) * (grid_count - 1);
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
    let corner_offset = select(vec2(1.0, 0.0), vec2(0.0, 1.0), weights.z > 0.0);
    let sample_corner = sample_tile(in.base_world_position, in.world_position, grid_index + corner_offset);
    let sample_br = sample_tile(in.base_world_position, in.world_position, grid_index + vec2(1.0, 1.0));
    let sample = sample_tl * weights.x + sample_corner * (weights.y + weights.z) + sample_br * weights.w;

    if sample.a < 0.5 {
        discard;
    }

    out.color = clamp(sample, vec4(0.0), vec4(1.0));
#endif

#ifdef IMPOSTER_MATERIAL
    let corner_offset = select(vec2(1.0, 0.0), vec2(0.0, 1.0), weights.z > 0.0);

    let props_tl = sample_tile_material(in.uv_tl_c.xy, grid_index);
    let props_corner = sample_tile_material(in.uv_tl_c.zw, grid_index + corner_offset);
    let props_br = sample_tile_material(in.uv_br, grid_index + vec2(1.0));

    let props_tlc = weighted_props(props_tl, props_corner, weights.x / (weights.x + weights.y + weights.z));
    let props_final = weighted_props(props_tlc, props_br, (weights.x + weights.y + weights.z) / (weights.x + weights.y + weights.z + weights.w));

    if props_final.rgba.a < 0.0001 {
        discard;
    }

    var pbr_input = unpack_pbrinput(props_final, in.position);    
    pbr_input.N = inv_rot * normalize(pbr_input.N);
    pbr_input.world_normal = pbr_input.N;

    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr_input);
    } else {
        out.color = pbr_input.material.base_color;
    }

    out.color = main_pass_post_lighting_processing(pbr_input, out.color);

    // out.color = clamp(out.color, vec4(0.2, 0.0, 0.0, 0.2), vec4(1.0));
#endif

    return out;
}
