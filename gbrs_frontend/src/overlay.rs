//! Shows short messages, such as the palette switched to or "State saved", in the bottom-left
//! corner of the screen for a moment. They're drawn at the window's resolution, over the screen,
//! by `overlay.wgsl`.

use std::time::{Duration, Instant};

use gbrs::SCREEN_WIDTH;
use pixels::{
    Pixels, PixelsContext,
    wgpu::{self, util::DeviceExt},
};

/// How long a message stays on screen, including fading out.
const MESSAGE_DURATION: Duration = Duration::from_secs(2);
const FADE_DURATION: Duration = Duration::from_millis(500);

/// How many characters fit in the shader's `Locals::text`. Longer messages are cut short.
const MAX_LEN: usize = 64;
/// The shader's `Locals`: the text's origin, scale and opacity, then the text.
const UNIFORMS_SIZE: usize = 4 * size_of::<f32>() + MAX_LEN;

/// Font pixels between the text and the edges of its box.
const PADDING: u32 = 3;
/// Font pixels between the box and the edges of the screen.
const MARGIN: u32 = 4;
/// The font's characters are 8x8, but the last row and column are blank, to space them out.
const CHAR_SIZE: u32 = 8;
const TEXT_HEIGHT: u32 = 7;

/// A message, shown from when it's created.
pub struct Message {
    text: String,
    shown_at: Instant,
}

impl Message {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            shown_at: Instant::now(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// How opaque the message is at `now`, from 1 until it starts fading out to 0, when it's gone.
    pub fn opacity(&self, now: Instant) -> f32 {
        let shown_for = now.saturating_duration_since(self.shown_at);
        let remaining = MESSAGE_DURATION.saturating_sub(shown_for);
        (remaining.as_secs_f32() / FADE_DURATION.as_secs_f32()).min(1.0)
    }
}

pub struct OverlayRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
}

impl OverlayRenderer {
    pub fn new(pixels: &Pixels) -> Self {
        let device = pixels.device();
        let module = device.create_shader_module(wgpu::include_wgsl!("overlay.wgsl"));
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("overlay_uniforms"),
            size: UNIFORMS_SIZE as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let font = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("overlay_font"),
            contents: font8x8::legacy::BASIC_LEGACY.as_flattened(),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        let uniform_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("overlay_bind_group_layout"),
            entries: &[uniform_entry(0), uniform_entry(1)],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("overlay_bind_group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: font.as_entire_binding(),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("overlay_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("overlay_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: pixels.render_texture_format(),
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            bind_group,
            uniforms,
        }
    }

    /// Draw `text` over the screen, which has already been drawn to `render_target`. It's cut
    /// short if it doesn't fit, and not drawn at all if the screen is too small.
    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        render_target: &wgpu::TextureView,
        context: &PixelsContext,
        text: &str,
        opacity: f32,
    ) {
        let Some(layout) = Layout::new(context.scaling_renderer.clip_rect(), text.chars().count())
        else {
            return;
        };

        let mut uniforms = Vec::with_capacity(UNIFORMS_SIZE);
        for f in [
            layout.origin.0 as f32,
            layout.origin.1 as f32,
            layout.scale as f32,
            opacity,
        ] {
            uniforms.extend_from_slice(&f.to_ne_bytes());
        }
        uniforms.extend(
            text.chars()
                .take(layout.len)
                .map(|c| if c.is_ascii() { c as u8 } else { b'?' }),
        );
        uniforms.resize(UNIFORMS_SIZE, 0);
        context.queue.write_buffer(&self.uniforms, 0, &uniforms);

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("overlay_render_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: render_target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        let (x, y, width, height) = layout.rect;
        pass.set_scissor_rect(x, y, width, height);
        pass.draw(0..3, 0..1);
    }
}

/// Where a message goes, in framebuffer pixels.
#[derive(Debug, PartialEq)]
struct Layout {
    /// The box behind the text: x, y, width, height.
    rect: (u32, u32, u32, u32),
    /// The top-left corner of the text.
    origin: (u32, u32),
    /// Framebuffer pixels per font pixel.
    scale: u32,
    /// How many characters fit.
    len: usize,
}

impl Layout {
    /// Lay out a message of `len` characters on the screen drawn at `clip_rect` (x, y, width,
    /// height), or `None` if no character fits.
    fn new(clip_rect: (u32, u32, u32, u32), len: usize) -> Option<Self> {
        let (x, y, width, height) = clip_rect;
        // A font pixel is half a Game Boy pixel, rounded up, so about 40 characters fit across.
        let scale = (width / SCREEN_WIDTH as u32).div_ceil(2).max(1);
        let (width, height) = (width / scale, height / scale);

        let box_height = TEXT_HEIGHT + 2 * PADDING;
        let room = (width + 1).checked_sub(2 * (MARGIN + PADDING))? / CHAR_SIZE;
        let len = len.min(MAX_LEN).min(room as usize);
        if len == 0 || height < box_height + 2 * MARGIN {
            return None;
        }
        let box_width = len as u32 * CHAR_SIZE - 1 + 2 * PADDING;
        let box_x = x + MARGIN * scale;
        let box_y = y + (height - MARGIN - box_height) * scale;
        Some(Self {
            rect: (box_x, box_y, box_width * scale, box_height * scale),
            origin: (box_x + PADDING * scale, box_y + PADDING * scale),
            scale,
            len,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shader_is_valid() {
        crate::lcd::validate_shader(include_str!("overlay.wgsl"));
    }

    #[test]
    fn test_message_fades_out() {
        let message = Message::new("State saved");
        let at = |ms| message.shown_at + Duration::from_millis(ms);
        assert_eq!(message.opacity(at(0)), 1.0);
        assert_eq!(message.opacity(at(1500)), 1.0);
        assert_eq!(message.opacity(at(1750)), 0.5);
        assert_eq!(message.opacity(at(2000)), 0.0);
        assert_eq!(message.opacity(at(5000)), 0.0);
    }

    #[test]
    fn test_layout() {
        // A window 3 times the size of the screen, with a border on each side.
        let layout = Layout::new((10, 0, 480, 432), 11).unwrap();
        assert_eq!(
            layout,
            Layout {
                rect: (18, 398, 186, 26),
                origin: (24, 404),
                scale: 2,
                len: 11,
            }
        );
        // Messages too long for the screen are cut short.
        assert_eq!(Layout::new((0, 0, 160, 144), 100).unwrap().len, 18);
        assert_eq!(Layout::new((0, 0, 10, 10), 5), None);
    }
}
