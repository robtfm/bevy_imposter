#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    view_transformations::position_view_to_world,
}

const IMPOSTER_FLAG_HEMISPHERICAL: u32 = 1;

struct ImposterData {
    center_and_scale: vec4<f32>,
    grid_size: u32,
    flags: u32,
}

@group(2) @binding(200)
var<uniform> imposter_data: ImposterData;
@group(2) @binding(201) 
var imposter_texture: texture_2d<f32>;
@group(2) @binding(202) 
var imposter_sampler: sampler;

struct ImposterVertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) camera_direction: vec3<f32>,
    @location(2) base_world_position: vec3<f32>,
}

fn dir_to_grid(dir: vec3<f32>) -> vec2<f32> {
	if ((imposter_data.flags & IMPOSTER_FLAG_HEMISPHERICAL) != 0) {
        var dir2: vec3<f32> = dir;
        dir2.y = max(dir2.y, 0.001);
        dir2 = normalize(dir2);
        let octant: vec3<f32> = sign(dir2);
        let sum: f32 = dot(dir2, octant);
        let octahedron: vec3<f32> = dir2 / sum;

        return vec2<f32>(octahedron.x + octahedron.z, octahedron.z - octahedron.x);
    } else {
        let octant: vec3<f32> = sign(dir);
        let sum: f32 = dot(dir, octant);
        let octahedron: vec3<f32> = dir / sum;
        if (octahedron.y < 0.0) {
            let absolute: vec3<f32> = abs(octahedron);
            return octant.xz * vec2(1.0 - absolute.z, 1.0 - absolute.x);
        } else {
            return octahedron.xz;
        }
	}
}

fn normal_from_uv(uv: vec2<f32>) -> vec3<f32> {
	if ((imposter_data.flags & IMPOSTER_FLAG_HEMISPHERICAL) != 0) {
        let x = uv.x - uv.y;
        let z = -1.0 + uv.x + uv.y;
        let y = 1.0 - abs(x) - abs(z);
        return vec3(x, y, z);
    } else {
        let x = uv.x * 2.0 - 1.0;
        let z = uv.y * 2.0 - 1.0;
        let y = 1.0 - abs(x) - abs(z);

        if (y < 0.0) {
            return vec3(
                sign(x) * (1.0 - abs(z)),
                y,
                sign(z) * (1.0 - abs(x)),
            );
        } else {
            return vec3(x, y, z);
        }
    }
}

fn grid_weights(coords: vec2<f32>) -> vec4<f32> {
    var corner_tr = select(0.0, 1.0, coords.x > coords.y);
    let corner_bl = 1.0 - corner_tr;
    let corner = abs(coords.x - coords.y);
    let res = vec4<f32>(
        1.0 - max(coords.x, coords.y),
        corner_tr * corner,
        corner_bl * corner,
        min(coords.x, coords.y),
    );
    return res / (res.x + res.y + res.z + res.w);
}

fn sample_tile(base_world_position: vec3<f32>, world_position: vec3<f32>, grid_index: vec2<f32>) -> vec4<f32> {
    let grid_count = f32(imposter_data.grid_size);
    let tile_origin = grid_index / grid_count;
    let tile_size = 1.0 / grid_count;
    let sample_normal = normalize(normal_from_uv(tile_origin * grid_count / (grid_count - 1.0)));
    let camera_world_position = position_view_to_world(vec3<f32>(0.0));
    let cam_to_fragment = normalize(world_position - camera_world_position);
    let distance = dot(world_position - camera_world_position, sample_normal) / dot(cam_to_fragment, sample_normal);
    let intersect = distance * cam_to_fragment + camera_world_position;
    // calculate uv using basis of the sample plane
    var up = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), abs(sample_normal.y) > 0.5);
    let sample_r = normalize(cross(sample_normal, up)) / (imposter_data.center_and_scale.w * 2.0);
    let sample_u = normalize(cross(sample_r, sample_normal)) / (imposter_data.center_and_scale.w * 2.0);
    let v = intersect - base_world_position;
    let x = dot(v, sample_r);
    let y = dot(v, sample_u);
    let uv = vec2<f32>(x, y) + 0.5;
    let sample_tl = textureSample(imposter_texture, imposter_sampler, tile_origin + uv * tile_size);
    let valid = select(1.0, 0.0, any(clamp(uv, vec2(0.0), vec2(1.0)) != uv));
    return sample_tl * valid;
}

@fragment
fn fragment(in: ImposterVertexOut) -> FragmentOutput {
    var out: FragmentOutput;
    let grid_count = f32(imposter_data.grid_size);

    var grid_pos = (dir_to_grid(in.camera_direction) + 1.0) * 0.5 * (grid_count - 1);
    let grid_index = min(floor(grid_pos), vec2(grid_count - 2.0));
    let frac = clamp(grid_pos - grid_index, vec2(0.0), vec2(1.0));

    let sample_tl = sample_tile(in.world_position, in.base_world_position, grid_index);
    let sample_tr = sample_tile(in.world_position, in.base_world_position, grid_index + vec2(1.0, 0.0));
    let sample_bl = sample_tile(in.world_position, in.base_world_position, grid_index + vec2(0.0, 1.0));
    let sample_br = sample_tile(in.world_position, in.base_world_position, grid_index + vec2(1.0, 1.0));

    let weights = grid_weights(frac);
    let sample = sample_tl * weights.x + sample_tr * weights.y + sample_bl * weights.z + sample_br * weights.w;

    if sample.a < 0.5 {
        discard;
    }

    out.color = clamp(sample, vec4(0.0), vec4(1.0));
    return out;
}
