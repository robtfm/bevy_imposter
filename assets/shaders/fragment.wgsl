#import bevy_pbr::{
    forward_io::FragmentOutput,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
}

#import "shaders/shared.wgsl"::{ImposterVertexOut, unpack_pbrinput, unpack_props};
#import "shaders/bindings.wgsl"::{imposter_data, oct_mode_uv_from_normal, grid_weights, sample_tile, sample_tile_material};

@fragment
fn fragment(in: ImposterVertexOut) -> FragmentOutput {
    var out: FragmentOutput;
    let grid_count = f32(imposter_data.grid_size);

    let grid_pos = oct_mode_uv_from_normal(in.camera_direction) * (grid_count - 1);
    let grid_index = clamp(floor(grid_pos), vec2(0.0), vec2(grid_count - 2.0));
    let frac = clamp(grid_pos - grid_index, vec2(0.0), vec2(1.0));

    let weights = grid_weights(frac);

    let inv_rot = mat3x3(
        in.inverse_rotation_0c,
        in.inverse_rotation_1c,
        in.inverse_rotation_2c,
    );

#ifdef IMPOSTER_IMAGE
    let sample_tl = sample_tile(in.base_world_position, in.world_position, grid_index);
    let corner_offset = select(vec2(1.0, 0.0), vec2(0.0, 1.0), weights.z > 0.0);
    let sample_corner = sample_tile(in.base_world_position, in.world_position, grid_index + corner_offset);
    let sample_br = sample_tile(in.base_world_position, in.world_position, grid_index + vec2(1.0, 1.0));
    let sample = sample_tl * weights.x + sample_corner * (weights.y + weights.z) + sample_br * weights.w;

    if sample.a < 0.5 {
        discard;
    }

    out.color = clamp(sample, vec4(0.0), vec4(1.0));
#endif

#ifdef IMPOSTER_MATERIAL
    let corner_offset = select(vec2(1.0, 0.0), vec2(0.0, 1.0), weights.z > 0.0);

    let sample_tl = sample_tile_material(in.uv_tl_c.xy, grid_index);
    let sample_corner = sample_tile_material(in.uv_tl_c.zw, grid_index + corner_offset);
    let sample_br = sample_tile_material(in.uv_br, grid_index + vec2(1.0));

    let props_tl = unpack_props(sample_tl.rg);
    let props_corner = unpack_props(sample_corner.rg);
    let props_br = unpack_props(sample_br.rg);

    var total_weight = 0.0;
    var total_color = vec4<f32>(0.0);
    var total_normal = vec3<f32>(0.0);
    var total_rough_metallic = vec2<f32>(0.0);

    if props_tl.rgba.a * weights.x > 0.0001 {
        total_weight += weights.x;
        total_color += props_tl.rgba * weights.x;
        total_normal += props_tl.normal * weights.x;
        total_rough_metallic += vec2(props_tl.roughness, props_tl.metallic) * weights.x;
    }
    let weight_corner = weights.y + weights.z;
    if props_corner.rgba.a * weight_corner > 0.0001 {
        total_weight += weight_corner;
        total_color += props_corner.rgba * weight_corner;
        total_normal += props_corner.normal * weight_corner;
        total_rough_metallic += vec2(props_corner.roughness, props_corner.metallic) * weight_corner;
    }
    if props_br.rgba.a * weights.w > 0.0001 {
        total_weight += weights.w;
        total_color += props_br.rgba * weights.w;
        total_normal += props_br.normal * weights.w;
        total_rough_metallic += vec2(props_br.roughness, props_br.metallic) * weights.w;
    }
    if total_weight < 0.0001 {
        discard;
    }
    
    var pbr_input = unpack_pbrinput(props_tl, in.position);
    pbr_input.material.base_color = vec4(total_color.xyz / total_weight, total_color.a);
    pbr_input.material.perceptual_roughness = total_rough_metallic.x / total_weight;
    pbr_input.material.metallic = total_rough_metallic.y / total_weight;
    
    pbr_input.N = inv_rot * normalize(total_normal / total_weight);
    pbr_input.world_normal = pbr_input.N;

    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr_input);
    } else {
        out.color = pbr_input.material.base_color;
    }

    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
#endif

    return out;
}
