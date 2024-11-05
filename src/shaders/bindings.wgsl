#define_import_path boimp::bindings

#import bevy_pbr::{
    view_transformations::position_view_to_world,
}

#import boimp::shared::{
    ImposterData, 
    UnpackedMaterialProps,
    GRID_MODE_MASK, 
    GRID_SPHERICAL, 
    GRID_HEMISPHERICAL, 
    GRID_HORIZONTAL, 
    spherical_normal_from_uv,
    spherical_uv_from_normal, 
    unpack_props,
    weighted_props,
};

@group(2) @binding(200)
var<uniform> imposter_data: ImposterData;

@group(2) @binding(201) 
var imposter_texture: texture_2d<u32>;

@group(2) @binding(202)
var imposter_sampler: sampler;

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
    let mode = imposter_data.flags & GRID_MODE_MASK;
    let grid_size = f32(imposter_data.grid_size);
	if mode == GRID_HEMISPHERICAL {
        // map direction to uv
        let dir2 = normalize(max(dir, vec3(-1.0, 0.0, -1.0)));
        let octant: vec3<f32> = sign(dir2);
        let sum: f32 = dot(dir2, octant);
        let octahedron: vec3<f32> = dir2 / sum;
        let uv = (vec2<f32>(octahedron.x + octahedron.z, octahedron.z - octahedron.x) + 1.0) * 0.5;
        
        return oct_sample_positions(uv);
    } else if mode == GRID_HORIZONTAL {
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
    } else {
        let uv = spherical_uv_from_normal(dir);
        return oct_sample_positions(uv);
    }
}

struct Basis {
    normal: vec3<f32>,
    up: vec3<f32>,
}

fn oct_mode_normal_from_uv(grid_index: vec2<u32>, inv_rot: mat3x3<f32>) -> Basis {
    let mode = imposter_data.flags & GRID_MODE_MASK;
    var n: vec3<f32>;
	if mode == GRID_HEMISPHERICAL {
        let grid_count = f32(imposter_data.grid_size);
        let tile_origin = vec2<f32>(grid_index) / grid_count;
        let tile_size = 1.0 / grid_count;
        let uv = tile_origin * grid_count / (grid_count - 1.0);
        var x = uv.x - uv.y;
        var z = -1.0 + uv.x + uv.y;
        var y = 1.0 - abs(x) - abs(z);
        n = normalize(vec3(x, y, z));
    } else if mode == GRID_HORIZONTAL {
        let index = grid_index.y * imposter_data.grid_size + grid_index.x;
        let angle: f32 = 6.283185307 * f32(index) / f32(imposter_data.grid_size * imposter_data.grid_size);
        let x: f32 = sin(angle);
        let z: f32 = cos(angle);
        n = vec3<f32>(x, 0.0, z);
    } else {
        let grid_count = f32(imposter_data.grid_size);
        let tile_origin = vec2<f32>(grid_index) / grid_count;
        let tile_size = 1.0 / grid_count;
        let uv = tile_origin * grid_count / (grid_count - 1.0);
        let uv2 = uv * (f32(imposter_data.grid_size) - 1.0) * f32(imposter_data.grid_size);
        n = spherical_normal_from_uv(uv);
    }

    let up = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), abs(n.y) > 0.99);

    var basis: Basis;
    basis.normal = inv_rot * n;
    basis.up = inv_rot * up;
    return basis;
}

fn sample_uvs_unbounded(base_world_position: vec3<f32>, world_position: vec3<f32>, inv_rot: mat3x3<f32>, grid_index: vec2<u32>) -> vec2<f32> {
    let basis = oct_mode_normal_from_uv(grid_index, inv_rot);

    let sample_normal = basis.normal;
    let camera_world_position = position_view_to_world(vec3<f32>(0.0));
    let cam_to_fragment = normalize(world_position - camera_world_position);
    let distance = dot(base_world_position - camera_world_position, sample_normal) / dot(cam_to_fragment, sample_normal);
    let intersect = distance * cam_to_fragment + camera_world_position;
    // calculate uv using basis of the sample plane
    let sample_r = cross(sample_normal, -basis.up);
    let sample_u = cross(sample_r, sample_normal);
    let v = intersect - base_world_position;
    let x = dot(v, normalize(sample_r) / (imposter_data.center_and_scale.w * 2.0));
    let y = dot(v, normalize(sample_u) / (imposter_data.center_and_scale.w * 2.0));
    let uv = vec2<f32>(x, y) + 0.5;
    return uv;
}

fn sample_tile_material(uv: vec2<f32>, grid_index: vec2<u32>) -> UnpackedMaterialProps {
    let grid_count = f32(imposter_data.grid_size);
    let tile_origin = vec2<f32>(grid_index) / grid_count;
    let tile_size = 1.0 / grid_count;
    let local_uv = tile_origin + tile_size * uv;

    let oob = any(uv <= vec2(0.0)) || any(uv >= vec2(1.0));

#ifdef MATERIAL_MULTISAMPLE
        if oob {
            return unpack_props(vec2(0u));
        } else {
            let coords = local_uv * vec2<f32>(textureDimensions(imposter_texture));
            let pixel_tl = unpack_props(textureLoad(imposter_texture, vec2<u32>(coords), 0).rg);
            let pixel_tr = unpack_props(textureLoad(imposter_texture, vec2<u32>(coords + vec2(1.0, 0.0)), 0).rg);
            let pixel_bl = unpack_props(textureLoad(imposter_texture, vec2<u32>(coords + vec2(0.0, 1.0)), 0).rg);
            let pixel_br = unpack_props(textureLoad(imposter_texture, vec2<u32>(coords + vec2(1.0, 1.0)), 0).rg);

            let frac = fract(coords);
            let pixel_top = weighted_props(pixel_tl, pixel_tr, 1.0 - frac.x);
            let pixel_bottom = weighted_props(pixel_bl, pixel_br, 1.0 - frac.x);
            let pixel = weighted_props(pixel_top, pixel_bottom, 1.0 - frac.y);
            return pixel;
        }
#else
        let coords = vec2<u32>(local_uv * vec2<f32>(textureDimensions(imposter_texture)) + 0.5);
        let pixel = textureLoad(imposter_texture, coords, 0);
        return unpack_props(select(pixel.xy, vec2(0u), oob));
#endif

}
