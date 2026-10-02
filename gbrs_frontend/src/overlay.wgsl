// Draws a line of text on a translucent box, whose bounds are the scissor rectangle. The text is in
// an 8x8 bitmap font, scaled up by a whole number so it stays sharp.

// The box behind the text. Blending is in linear light, so it looks lighter than its alpha suggests.
const BACKGROUND: vec4<f32> = vec4<f32>(0.0, 0.0, 0.0, 0.85);
const TEXT: vec4<f32> = vec4<f32>(1.0, 1.0, 1.0, 1.0);
// How many characters `Locals::text` holds.
const MAX_LEN: i32 = 64;

struct Locals {
    // The top-left corner of the text, in framebuffer pixels.
    origin: vec2<f32>,
    // Framebuffer pixels per font pixel.
    scale: f32,
    // How opaque the whole overlay is, as it fades out.
    opacity: f32,
    // The text's ASCII codes, 4 per word, least significant byte first, padded with 0 (which draws
    // nothing).
    text: array<vec4<u32>, 4>,
}

// The font: 8 bytes for each of the 128 ASCII characters, one per row from the top, whose bit 0 is
// the leftmost pixel.
struct Font {
    rows: array<vec4<u32>, 64>,
}

@group(0) @binding(0) var<uniform> locals: Locals;
@group(0) @binding(1) var<uniform> font: Font;

// One triangle covering the whole render target, which the scissor rectangle clips to the box.
@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    // The font pixel this fragment is in, from the top-left corner of the text.
    let p = vec2<i32>(floor((position.xy - locals.origin) / locals.scale));
    var color = BACKGROUND;
    if is_lit(p) {
        color = TEXT;
    }
    return vec4<f32>(color.rgb, color.a * locals.opacity);
}

// Whether the font pixel at `p` is part of a character.
fn is_lit(p: vec2<i32>) -> bool {
    if p.x < 0 || p.y < 0 || p.y >= 8 || p.x >= 8 * MAX_LEN {
        return false;
    }
    let index = u32(p.x) / 8u;
    let c = byte(locals.text[index / 16u], index);
    let row_index = c * 8u + u32(p.y);
    let row = byte(font.rows[row_index / 16u], row_index);
    return ((row >> (u32(p.x) % 8u)) & 1u) != 0u;
}

// Byte `index` of an array of bytes packed in `vec4<u32>`s, from the `vec4` that holds it.
fn byte(words: vec4<u32>, index: u32) -> u32 {
    return (words[(index / 4u) % 4u] >> ((index % 4u) * 8u)) & 0xffu;
}
