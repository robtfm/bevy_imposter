#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
#import boimp::shared::{unpack_props, weighted_props, pack_props, UnpackedMaterialProps};

struct BlitData {
    samples: u32,
}

@group(0) @binding(0) var source: texture_2d<u32>;
@group(0) @binding(1) var<uniform> data: BlitData;
@fragment
fn blend_materials(in: FullscreenVertexOutput) -> @location(0) vec2<u32> {
    let source_dims = textureDimensions(source);
    let target_dims = source_dims / data.samples;
    
    let target_pixel = vec2<u32>(in.uv * vec2<f32>(target_dims));
    let viewport_pixel = target_pixel * data.samples;

    var y_samples: array<UnpackedMaterialProps,8>;
    var y_end = data.samples;

    for (var y = 0u; y < data.samples; y ++) {
        var x_end = data.samples;
        var x_samples: array<UnpackedMaterialProps,8>;
        for (var x = 0u; x < data.samples; x++) {
            let pixel = textureLoad(source, target_pixel * data.samples + vec2(x, y), 0).rg;
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

    return pack_props(y_samples[0]);
}

