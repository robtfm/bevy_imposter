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
    out.base_world_position = imposter_world_position;

    let up = vec3<f32>(0.0, 1.0, 0.0);
    let back = direction_view_to_world(vec3<f32>(0.0, 0.0, 1.0));
    let right = cross(up, back);
    let up2 = cross(back, right);

    let view_matrix = transpose(mat3x3(
        normalize(right), 
        normalize(up2), 
        back
    ));
    out.world_position = imposter_world_position + (vertex.position * scale * 2.0) * view_matrix;

    // project the actual frag position to the furthest of the front plane of the imposter, and the camera near plane * 0.9
    let ray_direction = normalize(camera_world_position - out.world_position);
    let plane_normal = direction_view_to_world(vec3<f32>(0.0, 0.0, 1.0));

    let imposter_front_plane_origin = imposter_world_position + plane_normal * imposter_data.center_and_scale.w;
    let imposter_front_plane_distance = dot(imposter_front_plane_origin - out.world_position, plane_normal) / dot(ray_direction, plane_normal);

    let camera_near_plane_origin = camera_world_position - perspective_camera_near() * back;
    let camera_near_plane_distance = dot(camera_near_plane_origin - out.world_position, plane_normal) / dot(ray_direction, plane_normal);

    let plane_distance = min(camera_near_plane_distance * 0.9, imposter_front_plane_distance);

    let point_on_plane = out.world_position + plane_distance * 1.0 * ray_direction;

    out.position = position_world_to_clip(point_on_plane);
    return out;
}
