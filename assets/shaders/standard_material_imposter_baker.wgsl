#import bevy_pbr::{
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
    pbr_functions::alpha_discard,
    pbr_fragment::pbr_input_from_standard_material,
    prepass_io::VertexOutput,
    prepass_io::FragmentOutput,
}

#import "shaders/shared.wgsl"::{pack_pbrinput, unpack_pbrinput};

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> @location(0) vec2<u32> {
    // generate a PbrInput struct from the StandardMaterial bindings
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    // alpha discard
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

    // write the imposter gbuffer
    var gbuffer = pack_pbrinput(pbr_input);
    let reconstructed_pbr_input = unpack_pbrinput(gbuffer, in.position);
    var color = pbr_input.material.base_color;
    var reconstructed_color = reconstructed_pbr_input.material.base_color;

    var normal = pbr_input.world_normal;
    var reconstructed_normal = reconstructed_pbr_input.world_normal;

    return gbuffer;
}

