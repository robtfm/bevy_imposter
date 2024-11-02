#import bevy_pbr::{
    pbr_functions::alpha_discard,
    pbr_fragment::pbr_input_from_standard_material,
    prepass_io::VertexOutput,
    prepass_io::FragmentOutput,
}

#import boimp::shared::{pack_pbrinput, unpack_pbrinput};

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
    return pack_pbrinput(pbr_input);
}

