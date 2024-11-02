#define_import_path boimp::bindings

#import bevy_pbr::{
    view_transformations::position_view_to_world,
}

#import boimp::shared::{
    ImposterData, 
    UnpackedMaterialProps,
    GRID_MODE_BITS, 
    GRID_SPHERICAL, 
    GRID_HEMISPHERICAL, 
    GRID_HORIZONTAL, 
    MATERIAL_MULTISAMPLE, 
    spherical_normal_from_uv,
    spherical_uv_from_normal, 
    unpack_props,
    weighted_props,
};

@group(2) @binding(200)
var<uniform> imposter_data: ImposterData;

@group(2) @binding(201) 
#ifdef IMPOSTER_MATERIAL
    var imposter_texture: texture_2d<u32>;
#else
    var imposter_texture: texture_2d<f32>;
#endif

@group(2) @binding(202) 
var imposter_sampler: sampler;

fn oct_mode_uv_from_normal(dir: vec3<f32>) -> vec2<f32> {
    let mode = imposter_data.flags & GRID_MODE_BITS;
	if mode == GRID_HEMISPHERICAL || mode == GRID_HORIZONTAL {
        var dir2: vec3<f32> = dir;
        dir2.y = select(0.0, max(dir2.y, 0.0), mode == GRID_HEMISPHERICAL);
        dir2 = normalize(dir2);
        let octant: vec3<f32> = sign(dir2);
        let sum: f32 = dot(dir2, octant);
        let octahedron: vec3<f32> = dir2 / sum;
        return (vec2<f32>(octahedron.x + octahedron.z, octahedron.z - octahedron.x) + 1.0) * 0.5;
    } else {
        return spherical_uv_from_normal(dir);
    }
}

struct Basis {
    normal: vec3<f32>,
    up: vec3<f32>,
}

fn oct_mode_normal_from_uv(uv: vec2<f32>, inv_rot: mat3x3<f32>) -> Basis {
    let mode = imposter_data.flags & GRID_MODE_BITS;
    var n: vec3<f32>;
	if (mode == GRID_HEMISPHERICAL || mode == GRID_HORIZONTAL) {
        var x = uv.x - uv.y;
        var z = -1.0 + uv.x + uv.y;
        var y = select(0.0, 1.0 - abs(x) - abs(z), mode == GRID_HEMISPHERICAL);
        n = normalize(vec3(x, y, z));
    } else {
        n = spherical_normal_from_uv(uv);
    }

    let up = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), abs(n.y) > 0.99);

    var basis: Basis;
    basis.normal = inv_rot * n;
    basis.up = inv_rot * up;
    return basis;
}

fn grid_weights(coords: vec2<f32>) -> vec4<f32> {
    let corner_tr = select(0.0, 1.0, coords.x > coords.y);
    let corner_bl = 1.0 - corner_tr;
    let corner = abs(coords.x - coords.y);
    let res = vec4<f32>(
        1.0 - max(coords.x, coords.y),
        corner_tr * corner,
        corner_bl * corner,
        min(coords.x, coords.y),
    );
    return res / (res.x + res.y + res.w + res.z);
}

fn sample_uvs_unbounded(base_world_position: vec3<f32>, world_position: vec3<f32>, inv_rot: mat3x3<f32>, grid_index: vec2<f32>) -> vec2<f32> {
    let grid_count = f32(imposter_data.grid_size);
    let tile_origin = grid_index / grid_count;
    let tile_size = 1.0 / grid_count;
    let basis = oct_mode_normal_from_uv(tile_origin * grid_count / (grid_count - 1.0), inv_rot);
    let sample_normal = basis.normal;
    let camera_world_position = position_view_to_world(vec3<f32>(0.0));
    let cam_to_fragment = normalize(world_position - camera_world_position);
    let distance = dot(base_world_position - camera_world_position, sample_normal) / dot(cam_to_fragment, sample_normal);
    let intersect = distance * cam_to_fragment + camera_world_position;
    // calculate uv using basis of the sample plane
    let sample_r = normalize(cross(sample_normal, -basis.up));
    let sample_u = normalize(cross(sample_r, sample_normal));
    let v = intersect - base_world_position;
    let x = dot(v, sample_r / (imposter_data.center_and_scale.w * 2.0));
    let y = dot(v, sample_u / (imposter_data.center_and_scale.w * 2.0));
    let uv = vec2<f32>(x, y) + 0.5;
    return uv;
}

#ifdef IMPOSTER_IMAGE
fn sample_uvs(base_world_position: vec3<f32>, world_position: vec3<f32>, inv_rot: mat3x3<f32>, grid_index: vec2<f32>) -> vec2<f32> {
    let uv = sample_uvs_unbounded(base_world_position, world_position, inv_rot, grid_index);
    let grid_count = f32(imposter_data.grid_size);
    let tile_origin = grid_index / grid_count;
    let tile_size = 1.0 / grid_count;
    return select(
        vec2(tile_origin + tile_size * uv),
        vec2(-1.0),
        any(clamp(uv, vec2(0.0), vec2(1.0)) != uv)
    );
}

fn sample_tile(base_world_position: vec3<f32>, world_position: vec3<f32>, inv_rot: mat3x3<f32>, grid_index: vec2<f32>) -> vec4<f32> {
    let uv = sample_uvs(base_world_position, world_position, inv_rot, grid_index);
    let sample_tl = textureSample(imposter_texture, imposter_sampler, uv);
    return select(sample_tl, vec4(0.0), uv.x <= 0.0);
}
#endif

#ifdef IMPOSTER_MATERIAL
fn sample_tile_material(uv: vec2<f32>, grid_index: vec2<f32>) -> UnpackedMaterialProps {
    let grid_count = f32(imposter_data.grid_size);
    let tile_origin = grid_index / grid_count;
    let tile_size = 1.0 / grid_count;
    let local_uv = tile_origin + tile_size * uv;

    if (imposter_data.flags & MATERIAL_MULTISAMPLE) != 0u {
        let oob = any(uv <= vec2(0.0)) || any(uv >= vec2(1.0));
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
    } else {
        let coords = vec2<u32>(local_uv * vec2<f32>(textureDimensions(imposter_texture)) + 0.5);
        let pixel = textureLoad(imposter_texture, coords, 0);
        let oob = any(uv <= vec2(0.0)) || any(uv >= vec2(1.0));
        return unpack_props(select(pixel.xy, vec2(0u), oob));
    }
}
#endif
