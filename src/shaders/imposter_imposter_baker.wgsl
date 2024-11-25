#import boimp::shared::{ImposterVertexOut, unpack_pbrinput, weighted_props, pack_pbrinput};
#import boimp::bindings::{imposter_data, sample_positions_from_camera_dir, sample_tile_material};

#import bevy_pbr::pbr_types::{pbr_input_new, STANDARD_MATERIAL_FLAGS_UNLIT_BIT};

@fragment
fn fragment(in: ImposterVertexOut) -> @location(0) vec2<u32> {
    let samples = sample_positions_from_camera_dir(in.camera_direction);

    let inv_rot = mat3x3(
        in.inverse_rotation_0c,
        in.inverse_rotation_1c,
        in.inverse_rotation_2c,
    );

    let props_a = sample_tile_material(in.uv_ab.xy, samples.tile_indices[0], vec2(0.0));
    let props_b = sample_tile_material(in.uv_ab.zw, samples.tile_indices[1], vec2(0.0));
#ifndef GRID_HORIZONTAL
    let props_c = sample_tile_material(in.uv_c, samples.tile_indices[2], vec2(0.0));
#endif

    let weights = samples.tile_weights;
    let props_ab = weighted_props(props_a, props_b, weights.x / max(weights.x + weights.y, 0.0001));
#ifndef GRID_HORIZONTAL
    let props_final = weighted_props(props_ab, props_c, (weights.x + weights.y) / (weights.x + weights.y + weights.z));
#else 
    let props_final = props_ab;
#endif

    if props_final.rgba.a < 0.5 {
        discard;
    }

    var pbr_input = unpack_pbrinput(props_final, in.position);
    pbr_input.material.base_color.a = 1.0;
    pbr_input.N = inv_rot * normalize(pbr_input.N);
    pbr_input.world_normal = pbr_input.N;

    // write the imposter gbuffer
    return pack_pbrinput(pbr_input);
}
