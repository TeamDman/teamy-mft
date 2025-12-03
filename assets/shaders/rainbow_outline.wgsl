#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
}

#ifdef PREPASS_PIPELINE
#import bevy_pbr::{
    prepass_io::{VertexOutput, FragmentOutput},
    pbr_deferred_functions::deferred_output,
}
#else
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}
#endif

struct RainbowOutlineMaterial {
    glow_control: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100)
var<uniform> rainbow_outline: RainbowOutlineMaterial;

fn rainbow_color(phase: f32) -> vec3<f32> {
    let offsets = vec3<f32>(0.0, 2.09439510239, 4.18879020479);
    return 0.5 + 0.5 * sin(vec3<f32>(phase) + offsets);
}

fn rim_intensity(normal: vec3<f32>, view: vec3<f32>, exponent: f32) -> f32 {
    let ndotv = dot(normalize(normal), normalize(-view));
    return pow(clamp(1.0 - ndotv, 0.0, 1.0), exponent);
}

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

#ifdef PREPASS_PIPELINE
    let out = deferred_output(in, pbr_input);
#else
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);

    let glow_strength = rainbow_outline.glow_control.x;
    if glow_strength > 0.0 {
        let exponent = max(rainbow_outline.glow_control.z, 0.1);
        let rim = rim_intensity(pbr_input.N, pbr_input.V, exponent);
        let width = clamp(rainbow_outline.glow_control.w, 0.01, 0.9);
        let outline_mask = smoothstep(1.0 - width, 1.0, rim);
        if outline_mask > 0.0 {
            let angle = atan2(pbr_input.world_normal.z, pbr_input.world_normal.x);
            let rainbow = rainbow_color(rainbow_outline.glow_control.y + angle);
            let glow = rainbow * outline_mask * glow_strength;
            out.color = vec4<f32>(out.color.rgb + glow, out.color.a);
        }
    }

    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
#endif

    return out;
}
