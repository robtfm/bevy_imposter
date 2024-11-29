#import boimp::shared::{ImposterVertexOut, unpack_pbrinput, weighted_props, pack_pbrinput};
#import boimp::bindings::{sample_positions_from_camera_dir, sample_tile_material, sample_uvs_unbounded};

#import bevy_pbr::{
    pbr_types::{pbr_input_new, STANDARD_MATERIAL_FLAGS_UNLIT_BIT},
    view_transformations::{direction_view_to_world, position_view_to_world}
}

@fragment
fn fragment(in: ImposterVertexOut) -> @location(0) vec2<u32> {
    let inv_rot = mat3x3(
        in.inverse_rotation_0c,
        in.inverse_rotation_1c,
        in.inverse_rotation_2c,
    );

    let camera_world_position = position_view_to_world(vec3<f32>(0.0));
#ifdef VIEW_PROJECTION_ORTHOGRAPHIC
    let back_vec = direction_view_to_world(vec3<f32>(0.0, 0.0, 1.0));
#else
    let back_vec = camera_world_position - in.base_world_position;
#endif

    let back = normalize(back_vec);

    let samples = sample_positions_from_camera_dir(back * inv_rot);

    let uv_a = sample_uvs_unbounded(in.base_world_position, in.world_position, inv_rot, samples.tile_indices[0]);
    let uv_b = sample_uvs_unbounded(in.base_world_position, in.world_position, inv_rot, samples.tile_indices[1]);

    let props_a = sample_tile_material(uv_a, samples.tile_indices[0], vec2(0.0));
    let props_b = sample_tile_material(uv_b, samples.tile_indices[1], vec2(0.0));

#ifndef GRID_HORIZONTAL
    let uv_c = sample_uvs_unbounded(in.base_world_position, in.world_position, inv_rot, samples.tile_indices[2]);
    let props_c = sample_tile_material(uv_c, samples.tile_indices[2], vec2(0.0));
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
