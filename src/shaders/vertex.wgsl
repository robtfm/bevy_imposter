#import bevy_pbr::{
    mesh_functions,
    forward_io::Vertex,
    view_transformations::{position_world_to_clip, position_view_to_world},
}

#import boimp::shared::{ImposterVertexOut, VERTEX_BILLBOARD, GRID_HORIZONTAL};
#import boimp::bindings::{imposter_data, sample_uvs_unbounded, grid_weights, oct_mode_uv_from_normal};

@vertex
fn vertex(vertex: Vertex) -> ImposterVertexOut {
    var out: ImposterVertexOut;

    var model = mesh_functions::get_world_from_local(vertex.instance_index);

    let center = imposter_data.center_and_scale.xyz;
    let scale = imposter_data.center_and_scale.w;

    let imposter_world_position = mesh_functions::mesh_position_local_to_world(model, vec4<f32>(center, 1.0)).xyz;
    let camera_world_position = position_view_to_world(vec3<f32>(0.0));

    let is_horiz = (imposter_data.flags & GRID_HORIZONTAL) == GRID_HORIZONTAL;
    let back = normalize((camera_world_position - imposter_world_position) * select(vec3(1.0), vec3(1.0, 0.0, 1.0), is_horiz));
    let inv_rot = transpose(mat3x3<f32>(
        model[0].xyz,
        model[1].xyz,
        model[2].xyz
    ));
    out.inverse_rotation_0c = inv_rot[0];
    out.inverse_rotation_1c = inv_rot[1];
    out.inverse_rotation_2c = inv_rot[2];
    out.camera_direction = normalize(back * inv_rot);
    out.base_world_position = imposter_world_position;

    let billboard = (imposter_data.flags & VERTEX_BILLBOARD) != 0u;

    if billboard {
        let up = vec3<f32>(0.0, 1.0, 0.0);
        let right = normalize(cross(up, back));
        let up2 = normalize(cross(back, right));
    
        let view_matrix = transpose(mat3x3(right, up2, back));
        out.world_position = imposter_world_position + (vertex.position * scale * 2.0) * view_matrix;
    } else {
        out.world_position = mesh_functions::mesh_position_local_to_world(model, vec4<f32>(vertex.position, 1.0)).xyz;
    }

    out.position = position_world_to_clip(out.world_position);
    
    let relative_world_position = out.world_position - imposter_world_position;
    let distance = dot(relative_world_position, back);
    let projected_world_position = out.world_position - distance * back;

    let grid_count = f32(imposter_data.grid_size);
    var grid_pos = oct_mode_uv_from_normal(out.camera_direction) * (grid_count - 1);
    let grid_index = clamp(floor(grid_pos), vec2(0.0), vec2(grid_count - 2.0));
    let frac = clamp(grid_pos - grid_index, vec2(0.0), vec2(1.0));
    let weights = grid_weights(frac);
    let uv_tl = sample_uvs_unbounded(imposter_world_position, projected_world_position, inv_rot, grid_index);
    let corner_offset = select(vec2(1.0, 0.0), vec2(0.0, 1.0), weights.z > 0.0);
    let uv_corner = sample_uvs_unbounded(imposter_world_position, projected_world_position, inv_rot, grid_index + corner_offset);
    let uv_br = sample_uvs_unbounded(imposter_world_position, projected_world_position, inv_rot, grid_index + vec2(1.0, 1.0));

    out.uv_tl_c = vec4(uv_tl, uv_corner);
    out.uv_br = uv_br;

    return out;
}
