// Draws aether-player parts. Colours are premultiplied throughout.

struct Draw {
    // Document pixels to clip space: clip = position * view.xy + view.zw.
    view: vec4<f32>,
    // Multiply tint in rgb, opacity in w.
    multiply: vec4<f32>,
    // Screen tint in rgb; w > 0.5 when a clipping mask applies.
    screen: vec4<f32>,
    // Mask layer in x.
    mask: vec4<f32>,
};

@group(0) @binding(0) var<uniform> draw: Draw;
@group(1) @binding(0) var part_texture: texture_2d<f32>;
@group(1) @binding(1) var part_sampler: sampler;
@group(2) @binding(0) var masks: texture_2d_array<f32>;

struct Varyings {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@location(0) position: vec2<f32>, @location(1) uv: vec2<f32>) -> Varyings {
    var out: Varyings;
    out.position = vec4<f32>(position * draw.view.xy + draw.view.zw, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_part(in: Varyings) -> @location(0) vec4<f32> {
    let texel = textureSample(part_texture, part_sampler, in.uv);
    // Tint straight colour: rgb·multiply, then screen. On premultiplied
    // colour the screen term is scaled by alpha.
    var rgb = texel.rgb * draw.multiply.rgb;
    rgb = rgb + draw.screen.rgb * texel.a - rgb * draw.screen.rgb;
    var k = draw.multiply.w;
    if (draw.screen.w > 0.5) {
        // The mask covers exactly the target's pixels: read, don't filter.
        k = k * textureLoad(masks, vec2<i32>(in.position.xy), i32(draw.mask.x), 0).a;
    }
    return vec4<f32>(rgb, texel.a) * k;
}

@fragment
fn fs_mask(in: Varyings) -> @location(0) vec4<f32> {
    let alpha = textureSample(part_texture, part_sampler, in.uv).a * draw.multiply.w;
    return vec4<f32>(0.0, 0.0, 0.0, alpha);
}
