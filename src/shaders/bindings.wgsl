#define_import_path boimp::bindings

#import bevy_pbr::{
    view_transformations::position_view_to_world,
}

#import boimp::shared::{
    ImposterData, 
    UnpackedMaterialProps,
    spherical_normal_from_uv,
    spherical_uv_from_normal, 
    unpack_props,
    weighted_props,
};

@group(2) @binding(0)
var<uniform> imposter_data: ImposterData;

@group(2) @binding(1) 
var imposter_pixels: texture_2d<u32>;

#ifdef INDEXED_PIXELS
@group(2) @binding(2)
var imposter_indices: texture_2d<u32>;
#endif

struct SamplePositions {
    tile_indices: array<vec2<u32>, 3>,
    tile_weights: vec3<f32>,
}

fn oct_sample_weights(tile_uv: vec2<f32>) -> vec3<f32> {
    let res = vec3<f32>(
        1.0 - max(tile_uv.x, tile_uv.y),
        abs(tile_uv.x - tile_uv.y),
        min(tile_uv.x, tile_uv.y),
    );
    return res / (res.x + res.y + res.z);
}

fn oct_sample_positions(uv: vec2<f32>) -> SamplePositions {
    var sample_positions: SamplePositions;

    let grid_pos = uv * (f32(imposter_data.grid_size) - 1.0);
    sample_positions.tile_indices[0] = clamp(vec2<u32>(grid_pos), vec2(0u), vec2(imposter_data.grid_size - 2));

    let frac = clamp(grid_pos - vec2<f32>(sample_positions.tile_indices[0]), vec2(0.0), vec2(1.0));

    sample_positions.tile_weights = oct_sample_weights(frac);
    sample_positions.tile_indices[1] = sample_positions.tile_indices[0] + select(vec2(0u,1u), vec2(1u,0u), frac.x >= frac.y);
    sample_positions.tile_indices[2] = sample_positions.tile_indices[0] + vec2(1u,1u);

    return sample_positions;
}

fn sample_positions_from_camera_dir(dir: vec3<f32>) -> SamplePositions {
    let grid_size = f32(imposter_data.grid_size);

#ifdef GRID_HEMISPHERICAL
        // map direction to uv
        let dir2 = normalize(max(dir, vec3(-1.0, 0.0, -1.0)));
        let octant: vec3<f32> = sign(dir2);
        let sum: f32 = dot(dir2, octant);
        let octahedron: vec3<f32> = dir2 / sum;
        let uv = (vec2<f32>(octahedron.x + octahedron.z, octahedron.z - octahedron.x) + 1.0) * 0.5;
        
        return oct_sample_positions(uv);
#endif

#ifdef GRID_HORIZONTAL
        let dir2 = normalize(vec2(dir.x, dir.z));
        let angle = 0.5 - atan2(dir2.x, -dir2.y) / 6.283185307;
        let index = angle * f32(imposter_data.grid_size * imposter_data.grid_size);
        let l_index = u32(index);
        let r_index = l_index + 1u;
        var sample_positions: SamplePositions;
        sample_positions.tile_indices[0] = vec2(l_index % imposter_data.grid_size, (l_index / imposter_data.grid_size) % imposter_data.grid_size);
        sample_positions.tile_indices[1] = vec2(r_index % imposter_data.grid_size, (r_index / imposter_data.grid_size) % imposter_data.grid_size);
        sample_positions.tile_weights[1] = fract(index);
        sample_positions.tile_weights[0] = 1.0 - sample_positions.tile_weights[1];
        return sample_positions;
#endif

#ifdef GRID_SPHERICAL
        let uv = spherical_uv_from_normal(dir);
        return oct_sample_positions(uv);
#endif
}

struct Basis {
    normal: vec3<f32>,
    up: vec3<f32>,
}

fn oct_mode_normal_from_uv(grid_index: vec2<u32>, inv_rot: mat3x3<f32>) -> Basis {
    var n: vec3<f32>;

#ifdef GRID_HEMISPHERICAL
        let grid_count = f32(imposter_data.grid_size);
        let tile_origin = vec2<f32>(grid_index) / grid_count;
        let tile_size = 1.0 / grid_count;
        let uv = tile_origin * grid_count / (grid_count - 1.0);
        var x = uv.x - uv.y;
        var z = -1.0 + uv.x + uv.y;
        var y = 1.0 - abs(x) - abs(z);
        n = normalize(vec3(x, y, z));
#endif

#ifdef GRID_HORIZONTAL
        let index = grid_index.y * imposter_data.grid_size + grid_index.x;
        let angle: f32 = 6.283185307 * f32(index) / f32(imposter_data.grid_size * imposter_data.grid_size);
        let x: f32 = sin(angle);
        let z: f32 = cos(angle);
        n = vec3<f32>(x, 0.0, z);
#endif

#ifdef GRID_SPHERICAL
        let grid_count = f32(imposter_data.grid_size);
        let tile_origin = vec2<f32>(grid_index) / grid_count;
        let tile_size = 1.0 / grid_count;
        let uv = tile_origin * grid_count / (grid_count - 1.0);
        let uv2 = uv * (f32(imposter_data.grid_size) - 1.0) * f32(imposter_data.grid_size);
        n = spherical_normal_from_uv(uv);
#endif

    let up = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), abs(n.y) > 0.99);

    var basis: Basis;
    basis.normal = inv_rot * n;
    basis.up = inv_rot * up;
    return basis;
}

// uv at mid, impact of 1 depth on uv
fn sample_uvs_unbounded(base_world_position: vec3<f32>, world_position: vec3<f32>, inv_rot: mat3x3<f32>, grid_index: vec2<u32>) -> vec4<f32> {
    let basis = oct_mode_normal_from_uv(grid_index, inv_rot);
    let sample_r_vec = cross(basis.normal, -basis.up);
    let sample_u_vec = cross(sample_r_vec, basis.normal);
    let sample_r = normalize(sample_r_vec);
    let sample_u = normalize(sample_u_vec);
    let backplane_base_world_position = base_world_position + basis.normal * imposter_data.center_and_scale.w;

#ifdef VIEW_PROJECTION_ORTHOGRAPHIC
    let v = world_position - base_world_position;
    let x = dot(v, sample_r / (imposter_data.center_and_scale.w * 2.0));
    let y = dot(v, sample_u / (imposter_data.center_and_scale.w * 2.0));

    let backplane_v = world_position - backplane_base_world_position;
    let backplane_x = dot(backplane_v, sample_r / (imposter_data.center_and_scale.w * 2.0));
    let backplane_y = dot(backplane_v, sample_u / (imposter_data.center_and_scale.w * 2.0));
#else
    let camera_world_position = position_view_to_world(vec3<f32>(0.0));
    let cam_to_fragment = normalize(world_position - camera_world_position);
    let distance = dot(base_world_position - camera_world_position, basis.normal) / dot(cam_to_fragment, basis.normal);
    let intersect = distance * cam_to_fragment + camera_world_position;
    // calculate uv using basis of the sample plane
    let v = intersect - base_world_position;
    let x = dot(v, sample_r / (imposter_data.center_and_scale.w * 2.0));
    let y = dot(v, sample_u / (imposter_data.center_and_scale.w * 2.0));

    let backplane_distance = dot(backplane_base_world_position - camera_world_position, basis.normal) / dot(cam_to_fragment, basis.normal);
    let backplane_intersect = backplane_distance * cam_to_fragment + camera_world_position;
    let backplane_v = backplane_intersect - backplane_base_world_position;
    let backplane_x = dot(backplane_v, sample_r / (imposter_data.center_and_scale.w * 2.0));
    let backplane_y = dot(backplane_v, sample_u / (imposter_data.center_and_scale.w * 2.0));
#endif

    let uv = vec2<f32>(x, y) + 0.5;
    let backplane_uv = vec2<f32>(backplane_x, backplane_y) + 0.5;
    return vec4<f32>(uv, (backplane_uv - uv));
}

fn single_sample(coords: vec2<f32>, bounds_min: vec2<f32>, bounds_max: vec2<f32>) -> UnpackedMaterialProps {
#ifdef INDEXED_PIXELS
    let pixel_dims = textureDimensions(imposter_pixels);
    var index: u32;

    if pixel_dims.x * pixel_dims.y < 65536 {
        // using u16 pairs
        let index_pair = textureLoad(imposter_indices, vec2<u32>(coords * vec2(0.5, 1.0)), 0).r;
        index = select(index_pair & 0xFFFF, index_pair >> 16, (u32(coords.x) & 1u) == 1u);
    } else {
        index = textureLoad(imposter_indices, vec2<u32>(coords), 0).r;
    }

    let index_x = index % pixel_dims.x;
    let index_y = index / pixel_dims.x;

    let props = textureLoad(imposter_pixels, vec2(index_x, index_y), 0).rg * vec2(select(1u, 0u, any(coords < bounds_min) || any(coords >= bounds_max)));
#else
    let props = textureLoad(imposter_pixels, vec2<u32>(coords), 0).rg * vec2(select(1u, 0u, any(coords < bounds_min) || any(coords >= bounds_max)));
#endif
    return unpack_props(props);
}

fn sample_tile_material(uv_and_dd: vec4<f32>, grid_index: vec2<u32>, coord_offset: vec2<f32>) -> UnpackedMaterialProps {
    let bounds_min = vec2<f32>(grid_index * imposter_data.packed_size);
    let bounds_max = bounds_min + vec2<f32>(imposter_data.packed_size);
    let coords_unadjusted = bounds_min - vec2<f32>(imposter_data.packed_offset) + uv_and_dd.xy * vec2<f32>(imposter_data.base_tile_size) + coord_offset;

#ifdef MATERIAL_MULTISAMPLE
        // multisample for depth
        let pixel_tl_depth = single_sample(coords_unadjusted, bounds_min, bounds_max);
        let pixel_tr_depth = single_sample(coords_unadjusted + vec2(1.0, 0.0), bounds_min, bounds_max);
        let pixel_bl_depth = single_sample(coords_unadjusted + vec2(0.0, 1.0), bounds_min, bounds_max);
        let pixel_br_depth = single_sample(coords_unadjusted + vec2(1.0, 1.0), bounds_min, bounds_max);

        let frac = fract(coords_unadjusted);
        let pixel_top_depth = weighted_props(pixel_tl_depth, pixel_tr_depth, 1.0 - frac.x);
        let pixel_bottom_depth = weighted_props(pixel_bl_depth, pixel_br_depth, 1.0 - frac.x);
        let pixel_depth = weighted_props(pixel_top_depth, pixel_bottom_depth, 1.0 - frac.y);
        let depth = pixel_depth.depth;

        let coords = coords_unadjusted + depth * uv_and_dd.zw * vec2<f32>(imposter_data.base_tile_size);

        // multisample final material
        let pixel_tl = single_sample(coords, bounds_min, bounds_max);
        let pixel_tr = single_sample(coords + vec2(1.0, 0.0), bounds_min, bounds_max);
        let pixel_bl = single_sample(coords + vec2(0.0, 1.0), bounds_min, bounds_max);
        let pixel_br = single_sample(coords + vec2(1.0, 1.0), bounds_min, bounds_max);

        let frac2 = fract(coords);
        let pixel_top = weighted_props(pixel_tl, pixel_tr, 1.0 - frac2.x);
        let pixel_bottom = weighted_props(pixel_bl, pixel_br, 1.0 - frac2.x);
        let pixel = weighted_props(pixel_top, pixel_bottom, 1.0 - frac2.y);
        return pixel;
#else
        let pixel_depth = single_sample(coords_unadjusted, bounds_min, bounds_max);
        let depth = pixel_depth.depth;
        let coords = coords_unadjusted + depth * uv_and_dd.zw * vec2<f32>(imposter_data.base_tile_size);
        let pixel = single_sample(coords, bounds_min, bounds_max);

        return pixel;
#endif
}
