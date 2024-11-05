#import bevy_pbr::{
    forward_io::FragmentOutput,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
}

#import boimp::shared::{ImposterVertexOut, unpack_pbrinput, weighted_props};
#import boimp::bindings::{imposter_data, sample_positions_from_camera_dir, sample_tile, sample_tile_material};

@fragment
fn fragment(in: ImposterVertexOut) -> FragmentOutput {
    var out: FragmentOutput;

    let samples = sample_positions_from_camera_dir(in.camera_direction);

    let inv_rot = mat3x3(
        in.inverse_rotation_0c,
        in.inverse_rotation_1c,
        in.inverse_rotation_2c,
    );

    let props_a = sample_tile_material(in.uv_ab.xy, samples.tile_indices[0]);
    let props_b = sample_tile_material(in.uv_ab.zw, samples.tile_indices[1]);
    let props_c = sample_tile_material(in.uv_c, samples.tile_indices[2]);

    let weights = samples.tile_weights;
    let props_ab = weighted_props(props_a, props_b, weights.x / (weights.x + weights.y));
    let props_final = weighted_props(props_ab, props_c, (weights.x + weights.y) / (weights.x + weights.y + weights.z));

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

    return out;
}
