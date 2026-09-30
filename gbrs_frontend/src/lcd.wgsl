// Draws the frame as the Game Boy's LCD looked: each pixel is a square with a thin gap around it,
// through which the unlit screen shows, and it casts a faint shadow on the reflective backing
// behind it, down and to the right.
//
// It reads the frame texture directly and works out which Game Boy pixel each fragment is in, so
// the effect scales with the window, and fades out gracefully when a pixel is only a few
// fragments wide.

// The width of the gap around each pixel, as a fraction of a pixel.
const GAP: f32 = 0.12;
// How far the shadows fall, in Game Boy pixels.
const SHADOW_OFFSET: vec2<f32> = vec2<f32>(0.25, 0.25);
// How much the shadow of a black pixel darkens the unlit screen.
const SHADOW_STRENGTH: f32 = 0.4;

struct Locals {
    // Where the screen is drawn, in framebuffer pixels: x, y, width, height.
    rect: vec4<f32>,
    // The colour of the unlit screen (the palette's lightest shade), in linear RGB.
    background: vec4<f32>,
}

@group(0) @binding(0) var frame: texture_2d<f32>;
@group(0) @binding(1) var<uniform> locals: Locals;

// One triangle covering the whole render target, which the scissor rectangle clips to the screen.
@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    // Fragments per Game Boy pixel. `pixels` scales by a whole number, so it's the same both ways.
    let scale = locals.rect.z / f32(textureDimensions(frame).x);
    // This fragment's centre, in Game Boy pixels.
    let pos = (position.xy - locals.rect.xy) / scale;

    let color = pixel(pos);
    let lit = coverage(pos, scale);
    // The shadow only shows where what's in front of it is lighter.
    let own = darkness(color) * lit;
    let caster = pos - SHADOW_OFFSET;
    let shadowing = darkness(pixel(caster)) * coverage(caster, scale);
    let shadow = SHADOW_STRENGTH * max(shadowing - own, 0.0);

    let rgb = mix(locals.background.rgb, color, lit) * (1.0 - shadow);
    return vec4<f32>(rgb, 1.0);
}

// The colour of the Game Boy pixel at `pos`, or the unlit screen outside it.
fn pixel(pos: vec2<f32>) -> vec3<f32> {
    let p = vec2<i32>(floor(pos));
    let size = vec2<i32>(textureDimensions(frame));
    if any(p < vec2<i32>(0)) || any(p >= size) {
        return locals.background.rgb;
    }
    return textureLoad(frame, p, 0).rgb;
}

// How much of the fragment centred on `pos` its Game Boy pixel's square covers, leaving out the
// gap. `scale` is the number of fragments per Game Boy pixel: the square's edges are antialiased
// over one fragment.
fn coverage(pos: vec2<f32>, scale: f32) -> f32 {
    // The distance to the nearest edge of the pixel, in fragments.
    let edge = (0.5 - abs(fract(pos) - 0.5)) * scale;
    let covered = clamp(edge - GAP * 0.5 * scale + 0.5, vec2<f32>(0.0), vec2<f32>(1.0));
    return covered.x * covered.y;
}

// How much darker than the unlit screen `color` is, from 0 to 1.
fn darkness(color: vec3<f32>) -> f32 {
    let background = max(luminance(locals.background.rgb), 1e-4);
    return clamp(1.0 - luminance(color) / background, 0.0, 1.0);
}

fn luminance(color: vec3<f32>) -> f32 {
    return dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
}
