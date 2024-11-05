#import boimp::shared::{unpack_props, weighted_props, pack_props, UnpackedMaterialProps};

struct BlitData {
    samples: u32,
}

@group(0) @binding(0) var source: texture_storage_2d<rg32uint, read>;
@group(0) @binding(1) var<uniform> data: BlitData;
@group(0) @binding(2) var output: texture_storage_2d<rg32uint, write>;
var<push_constant> viewport: array<u32,2>;

@compute
@workgroup_size(16, 16, 1)
fn blend_materials(
    @builtin(global_invocation_id) gid: vec3<u32>,
) {
    let viewport_pixel = vec2<u32>(viewport[0], viewport[1]);
    let target_pixel = gid.xy;

    var y_samples: array<UnpackedMaterialProps,8>;
    var y_end = data.samples;

    for (var y = 0u; y < data.samples; y ++) {
        var x_end = data.samples;
        var x_samples: array<UnpackedMaterialProps,8>;
        for (var x = 0u; x < data.samples; x++) {
            let pixel = textureLoad(source, target_pixel * data.samples + vec2(x, y)).rg;
            x_samples[x] = unpack_props(pixel);
        }

        while x_end > 1u {
            x_end /= 2u;

            for (var x = 0u; x < x_end; x++) {
                x_samples[x] = weighted_props(x_samples[x], x_samples[x + x_end], 0.5);
            }
        }

        y_samples[y] = x_samples[0];
    }

    while y_end > 1u {
        y_end /= 2u;

        for (var y = 0u; y < y_end; y++) {
            y_samples[y] = weighted_props(y_samples[y], y_samples[y + y_end], 0.5);
        }
    }

    textureStore(output, viewport_pixel + target_pixel, vec4(pack_props(y_samples[0]), 0u, 0u));
    // textureStore(output, viewport_pixel + target_pixel, vec4(gid.x, gid.y, 0u, 0u));
}

