#import bevy_pbr::{
    mesh_functions,
    view_transformations::{position_world_to_clip, position_view_to_world, direction_view_to_world, perspective_camera_near},
}

#ifdef PREPASS_PIPELINE
    #import bevy_pbr::prepass_io::Vertex;
#else
    #import bevy_pbr::forward_io::Vertex;
#endif

#import boimp::shared::ImposterVertexOut;
#import boimp::bindings::{imposter_data, sample_uvs_unbounded, grid_weights, sample_positions_from_camera_dir};

@vertex
fn vertex(vertex: Vertex) -> ImposterVertexOut {
    var out: ImposterVertexOut;

    var model = mesh_functions::get_world_from_local(vertex.instance_index);

    let center = imposter_data.center_and_scale.xyz;
    let scale = imposter_data.center_and_scale.w;

    let imposter_world_position = mesh_functions::mesh_position_local_to_world(model, vec4<f32>(center, 1.0)).xyz;
    let camera_world_position = position_view_to_world(vec3<f32>(0.0));

#ifdef VIEW_PROJECTION_ORTHOGRAPHIC
    let back_vec = direction_view_to_world(vec3<f32>(0.0, 0.0, 1.0));
#else
    let back_vec = camera_world_position - imposter_world_position;
#endif

    let back = normalize(back_vec);

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
    out.back = back;
    out.base_world_position = imposter_world_position;

#ifdef VERTEX_BILLBOARD
        let up = vec3<f32>(0.0, 1.0, 0.0);
        let back_vec2 = direction_view_to_world(vec3<f32>(0.0, 0.0, 1.0));
        let right = cross(up, back_vec2);
        let up2 = cross(back_vec2, right);
    
        let view_matrix = transpose(mat3x3(
            normalize(right), 
            normalize(up2), 
            back_vec2
        ));
        out.world_position = imposter_world_position + (vertex.position * scale * 2.0) * view_matrix;
#else
        out.world_position = mesh_functions::mesh_position_local_to_world(model, vec4<f32>(vertex.position, 1.0)).xyz;
#endif

    let ray_origin = out.world_position;
    let ray_direction = normalize(camera_world_position - ray_origin);
    let plane_normal = direction_view_to_world(vec3<f32>(0.0, 0.0, 1.0));
    let imposter_front_plane_origin = imposter_world_position + plane_normal * imposter_data.center_and_scale.w;
    let imposter_front_plane_distance = dot(imposter_front_plane_origin - ray_origin, plane_normal) / dot(ray_direction, plane_normal);

    let camera_near_plane_origin = camera_world_position - perspective_camera_near() * back;
    let camera_near_plane_distance = dot(camera_near_plane_origin - ray_origin, plane_normal) / dot(ray_direction, plane_normal);

    let plane_distance = min(camera_near_plane_distance * 0.9, imposter_front_plane_distance);

    let point_on_plane = ray_origin + plane_distance * 1.0 * ray_direction;

    out.position = position_world_to_clip(point_on_plane);

#ifndef VIEW_PROJECTION_ORTHOGRAPHIC
    let relative_world_position = out.world_position - imposter_world_position;
    let distance = dot(relative_world_position, back);
    let projected_world_position = out.world_position - distance * back;
#else
    let projected_world_position = out.world_position;
#endif

    let sample_positions = sample_positions_from_camera_dir(out.camera_direction);

    // todo: doing uv samples in the vertex shader is a negligible perf improvement, and can cause interpolation issues up close.
    // potentially move this back into the frag shader.
    out.uv_a = sample_uvs_unbounded(imposter_world_position, projected_world_position, inv_rot, sample_positions.tile_indices[0]);
    out.uv_b = sample_uvs_unbounded(imposter_world_position, projected_world_position, inv_rot, sample_positions.tile_indices[1]);
    out.uv_c = sample_uvs_unbounded(imposter_world_position, projected_world_position, inv_rot, sample_positions.tile_indices[2]);

#ifdef USE_SOURCE_UV_Y
        out.uv_a.y = 1.0-vertex.uv.y;
        out.uv_b.y = 1.0-vertex.uv.y;
        out.uv_c.y = 1.0-vertex.uv.y;
#endif

    return out;
}
