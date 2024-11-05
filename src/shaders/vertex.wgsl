#import bevy_pbr::{
    mesh_functions,
    forward_io::Vertex,
    view_transformations::{position_world_to_clip, position_view_to_world},
}

#import boimp::shared::{ImposterVertexOut, VERTEX_BILLBOARD, USE_SOURCE_UV_Y, GRID_HORIZONTAL};
#import boimp::bindings::{imposter_data, sample_uvs_unbounded, grid_weights, sample_positions_from_camera_dir};

@vertex
fn vertex(vertex: Vertex) -> ImposterVertexOut {
    var out: ImposterVertexOut;

    var model = mesh_functions::get_world_from_local(vertex.instance_index);

    let center = imposter_data.center_and_scale.xyz;
    let scale = imposter_data.center_and_scale.w;

    let imposter_world_position = mesh_functions::mesh_position_local_to_world(model, vec4<f32>(center, 1.0)).xyz;
    let camera_world_position = position_view_to_world(vec3<f32>(0.0));

    let is_horizontal = (imposter_data.flags & GRID_HORIZONTAL) == GRID_HORIZONTAL;
    let back = normalize((camera_world_position - imposter_world_position) * select(vec3(1.0), vec3(1.0, 0.0, 1.0), is_horizontal));

    // extract inverse rotation
    let inv_rot = transpose(mat3x3<f32>(
        normalize(model[0].xyz),
        normalize(model[1].xyz),
        normalize(model[2].xyz)
    ));
    // todo: we could pass the instance index instead, and extract in frag shader
    out.inverse_rotation_0c = inv_rot[0];
    out.inverse_rotation_1c = inv_rot[1];
    out.inverse_rotation_2c = inv_rot[2];
    out.camera_direction = normalize(back * inv_rot);
    out.base_world_position = imposter_world_position;

    if (imposter_data.flags & VERTEX_BILLBOARD) == VERTEX_BILLBOARD {
        let up = vec3<f32>(0.0, 1.0, 0.0);
        let right = cross(up, back);
        let up2 = cross(back, right);
    
        let view_matrix = transpose(mat3x3(
            normalize(right), 
            normalize(up2), 
            back
        ));
        out.world_position = imposter_world_position + (vertex.position * scale * 2.0) * view_matrix;
    } else {
        out.world_position = mesh_functions::mesh_position_local_to_world(model, vec4<f32>(vertex.position, 1.0)).xyz;
    }

    out.position = position_world_to_clip(out.world_position);
    
    let relative_world_position = out.world_position - imposter_world_position;
    let distance = dot(relative_world_position, back);
    let projected_world_position = out.world_position - distance * back;

    let sample_positions = sample_positions_from_camera_dir(out.camera_direction);

    // todo: doing uv samples in the vertex shader is a negligible perf improvement, and can cause interpolation issues up close.
    // potentially move this back into the frag shader.
    let uv_a = sample_uvs_unbounded(imposter_world_position, projected_world_position, inv_rot, sample_positions.tile_indices[0]);
    let uv_b = sample_uvs_unbounded(imposter_world_position, projected_world_position, inv_rot, sample_positions.tile_indices[1]);
    let uv_c = sample_uvs_unbounded(imposter_world_position, projected_world_position, inv_rot, sample_positions.tile_indices[2]);

    if (imposter_data.flags & USE_SOURCE_UV_Y) == USE_SOURCE_UV_Y {
        out.uv_ab = vec4(uv_a.x, 1.0-vertex.uv.y, uv_b.x, 1.0-vertex.uv.y);
        out.uv_c = vec2(uv_c.x, 1.0-vertex.uv.y);
    } else {
        out.uv_ab = vec4(uv_a, uv_b);
        out.uv_c = uv_c;
    }

    return out;
}
