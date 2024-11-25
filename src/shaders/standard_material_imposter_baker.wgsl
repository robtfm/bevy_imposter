#import bevy_pbr::{
    prepass_io::VertexOutput,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
}

#import boimp::shared::pack_pbrinput;

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> @location(0) vec2<u32> {
    // generate a PbrInput struct from the StandardMaterial bindings
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    // alpha discard
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

    // we can only store a single result, so we're going to unilaterally discard alpha < 0.5
    // todo: optionally we could
    // - run opaque
    // - copy texture out
    // - provide texture as input to alpha mat rendering
    // for materials to merge more intelligently
    if pbr_input.material.base_color.a < 0.5 {
        discard;
    }

    // write the imposter gbuffer
    return pack_pbrinput(pbr_input);
}

