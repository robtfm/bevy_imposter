#import bevy_pbr::{
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::{STANDARD_MATERIAL_FLAGS_UNLIT_BIT, STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT},
    view_transformations::{direction_view_to_world, position_view_to_world, position_world_to_clip}
}

#ifdef PREPASS_PIPELINE
 #import bevy_pbr::prepass_io::FragmentOutput;
#else
 #import bevy_pbr::forward_io::FragmentOutput;
#endif

#import boimp::shared::{ImposterVertexOut, unpack_pbrinput, weighted_props};
#import boimp::bindings::{imposter_data, sample_positions_from_camera_dir, sample_uvs_unbounded, sample_tile, sample_tile_material};

@fragment
fn fragment(in: ImposterVertexOut) -> FragmentOutput {
    var out: FragmentOutput;

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

    let cam_to_frag = normalize((camera_world_position - in.world_position) * inv_rot);
    let samples = sample_positions_from_camera_dir(cam_to_frag);

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

    if props_final.rgba.a < 0.01 {
        discard;
        // out.color = vec4(0.0, 0.2, 0.0, 0.2);
        // return out;
    }

// we can discard based on actual depth if we have the depth prepass data
#ifdef DEPTH_PREPASS
#ifndef PREPASS_PIPELINE
    let existing_depth_ndc = bevy_pbr::prepass_utils::prepass_depth(in.position, 0u);
    let imposted_ndc = position_world_to_clip(in.world_position + back * props_final.depth * imposter_data.center_and_scale.w);
    let imposter_depth_ndc = imposted_ndc.z / imposted_ndc.w;
    
    if imposter_depth_ndc < existing_depth_ndc {
        // out.color = vec4<f32>(0.0, 0.5, 0.0, 0.5);
        // return out;
        discard;
    }
#endif
#endif

    var pbr_input = unpack_pbrinput(props_final, in.position);    
    pbr_input.N = inv_rot * normalize(pbr_input.N);
    pbr_input.world_normal = pbr_input.N;

    pbr_input.material.base_color.a *= imposter_data.alpha;

#ifdef PREPASS_PIPELINE
    #ifdef NORMAL_PREPASS
        out.normal = vec4<f32>(pbr_input.N, 0.0);
    #endif
    // we don't support MOTION_VECTOR or DEFERRED
    #ifdef DEPTH_CLAMP_ORTHO
        out.frag_depth = in.position.z;
    #endif
#else 
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr_input);
    } else {
        out.color = pbr_input.material.base_color;
    }

    pbr_input.material.flags |= STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT;

    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
#endif

    // out.color = clamp(out.color, vec4<f32>(0.2, 0.0, 0.0, 0.2), vec4<f32>(1.0, 1.0, 1.0, 0.7));

    return out;
}
