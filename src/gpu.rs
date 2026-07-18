//! The ONE `DisplayList` → GPU translator — shared by every nami-core GPU
//! consumer (mado's floating browser panel + namimado's window), replacing the
//! per-consumer `DrawCmd → QuadInstance/glyphon` loops they each re-authored.
//!
//! Two layers:
//! - [`display_list_to_gpu`] is the **pure** shared core: `DrawCmd::Rect`/`Image`
//!   → garasu [`QuadInstance`](garasu::QuadInstance) (sRGB→linear), `DrawCmd::Text`
//!   → a typed [`TextRun`] the consumer feeds to *its own* `TextRenderer`. No GPU,
//!   no glyphon — unit-testable on a fixture `DisplayList`. Coordinates are
//!   offset by `origin` (mado uses `[0,0]`; namimado uses `[0,56]` for its
//!   address-bar chrome), so a consumer never re-offsets.
//! - [`render_display_list_to_rgba`] is the bare offscreen rasterizer (its own
//!   `HeadlessTarget`, no chrome) mado uses for its panel textures + snapshots.
//!   namimado keeps its own richer `render_content` wrapper (live-or-headless
//!   target + chrome text) but drives its inner loop through
//!   [`display_list_to_gpu`], so the `DrawCmd → GPU` mapping lives here once.
//!
//! Gated on the `gpu` feature (garasu + glyphon + wgpu + ishou-tokens) so
//! nami-core stays GPU-free for pure consumers.

use crate::paint::DisplayList;

/// One positioned text run extracted from a [`DrawCmd::Text`](crate::paint::DrawCmd::Text)
/// — the typed hand-off between the shared translation and the consumer's own
/// `TextRenderer` (which owns the font system). Coordinates already include the
/// caller's `origin` offset. `color` is resolved (the cmd's color, or the
/// caller's `fg_fallback` when the cmd inherits / has alpha 0), RGBA in `0..=1`.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    /// Left edge (px), origin-offset.
    pub x: f32,
    /// Top edge (px), origin-offset.
    pub y: f32,
    /// Box width — the renderer wraps/clips within this.
    pub max_width: f32,
    /// Box height — the renderer clips within this.
    pub max_height: f32,
    /// The text content.
    pub text: String,
    /// Font size (px).
    pub font_size: f32,
    /// Line height (px).
    pub line_height: f32,
    /// Resolved glyph color (RGBA `0..=1`).
    pub color: [f32; 4],
}

/// The pure shared translation: a [`DisplayList`] + a content `origin` (px) →
/// garasu quads (backgrounds/images, sRGB→linear) + typed [`TextRun`]s. No GPU
/// state touched — the caller feeds the quads to a `QuadPipeline` and the runs to
/// a `TextRenderer`. This is the loop mado + namimado used to each own.
#[must_use]
pub fn display_list_to_gpu(
    dl: &DisplayList,
    origin: [f32; 2],
    fg_fallback: [f32; 4],
) -> (Vec<garasu::QuadInstance>, Vec<TextRun>) {
    use crate::paint::DrawCmd;
    use garasu::QuadInstance;
    use ishou_tokens::space::srgb_channel_to_linear as lin;

    let [ox, oy] = origin;
    let mut quads: Vec<QuadInstance> = Vec::new();
    let mut runs: Vec<TextRun> = Vec::new();
    for cmd in &dl.cmds {
        match cmd {
            DrawCmd::Rect {
                x,
                y,
                width,
                height,
                color,
            } => quads.push(QuadInstance {
                pos: [ox + *x, oy + *y],
                size: [*width, *height],
                color: [lin(color.r), lin(color.g), lin(color.b), color.a],
            }),
            DrawCmd::Image {
                x,
                y,
                width,
                height,
                placeholder,
                ..
            } => quads.push(QuadInstance {
                pos: [ox + *x, oy + *y],
                size: [*width, *height],
                color: [
                    lin(placeholder.r),
                    lin(placeholder.g),
                    lin(placeholder.b),
                    placeholder.a,
                ],
            }),
            DrawCmd::Text {
                x,
                y,
                max_width,
                max_height,
                text,
                color,
                font_size,
                line_height,
            } => {
                // Inherited / currentColor (alpha 0) falls back to the caller's
                // scheme foreground — never an invisible glyph.
                let resolved = if color.a > 0.0 {
                    [color.r, color.g, color.b, color.a]
                } else {
                    fg_fallback
                };
                runs.push(TextRun {
                    x: ox + *x,
                    y: oy + *y,
                    max_width: *max_width,
                    max_height: *max_height,
                    text: text.clone(),
                    font_size: *font_size,
                    line_height: *line_height,
                    color: resolved,
                });
            }
        }
    }
    (quads, runs)
}

/// Rasterize a [`DisplayList`] to a tightly-packed RGBA8 buffer, offscreen (no
/// window) — the bare path (own `HeadlessTarget`, no chrome, origin 0). `bg`
/// clears the frame; `fg_fallback` colors text with no explicit color. Both are
/// LINEAR-space RGBA (the caller converts from sRGB), matching an
/// `Rgba8UnormSrgb` target. Drives the shared [`display_list_to_gpu`] core.
#[must_use]
pub fn render_display_list_to_rgba(
    gpu: &garasu::GpuContext,
    dl: &DisplayList,
    width: u32,
    height: u32,
    bg: [f32; 4],
    fg_fallback: [f32; 4],
) -> Vec<u8> {
    use garasu::{QuadPipeline, TextRenderer};
    use glyphon::{Color as GlyphColor, TextArea, TextBounds};

    let fmt = wgpu::TextureFormat::Rgba8UnormSrgb;
    let w = width.max(1);
    let h = height.max(1);
    let target = garasu::headless::HeadlessTarget::new(gpu, w, h, fmt);
    let mut quad = QuadPipeline::new(&gpu.device, fmt);
    quad.update_resolution(&gpu.queue, w, h);
    let mut text = TextRenderer::new(&gpu.device, &gpu.queue, fmt);

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let to_byte = |f: f32| (f.clamp(0.0, 1.0) * 255.0).round() as u8;

    // The shared translation: DrawCmd → quads + typed runs (origin 0 — the panel
    // IS the content rect, no chrome offset).
    let (quads, text_runs) = display_list_to_gpu(dl, [0.0, 0.0], fg_fallback);

    // Build glyphon buffers from the typed runs on THIS renderer's font system.
    // Buffers must outlive the `areas` that borrow them.
    let mut buffers: Vec<(glyphon::Buffer, f32, f32, f32, f32, GlyphColor)> =
        Vec::with_capacity(text_runs.len());
    for run in &text_runs {
        let mut buf = text.create_buffer(&run.text, run.font_size, run.line_height);
        buf.set_size(
            &mut text.font_system,
            Some(run.max_width.max(1.0)),
            Some(run.max_height.max(run.line_height)),
        );
        buf.shape_until_scroll(&mut text.font_system, false);
        let glyph = GlyphColor::rgb(
            to_byte(run.color[0]),
            to_byte(run.color[1]),
            to_byte(run.color[2]),
        );
        buffers.push((buf, run.x, run.y, run.max_width, run.max_height, glyph));
    }

    #[allow(clippy::cast_possible_truncation)]
    let areas: Vec<TextArea> = buffers
        .iter()
        .map(|(buf, left, top, rw, rh, color)| TextArea {
            buffer: buf,
            left: *left,
            top: *top,
            scale: 1.0,
            bounds: TextBounds {
                left: *left as i32,
                top: *top as i32,
                right: (*left + *rw) as i32,
                bottom: (*top + *rh) as i32,
            },
            default_color: *color,
            custom_glyphs: &[],
        })
        .collect();
    // On a prepare failure, still render the rects (never a silent full-blank).
    let _ = text.prepare(&gpu.device, &gpu.queue, w, h, areas);

    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nami_page"),
        });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("nami_page_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target.view(),
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: f64::from(bg[0]),
                        g: f64::from(bg[1]),
                        b: f64::from(bg[2]),
                        a: f64::from(bg[3]),
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        // Backgrounds/images before text, so glyphs sit on their box fills.
        quad.draw(&gpu.device, &gpu.queue, &mut pass, &quads);
        let _ = text.render(&mut pass);
    }
    gpu.queue.submit(std::iter::once(encoder.finish()));
    target.read_pixels_rgba8(gpu)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paint::{DrawCmd, Rgba};

    #[test]
    fn display_list_to_gpu_maps_rects_and_text_and_applies_origin_and_fallback() {
        let green = Rgba {
            r: 0.0,
            g: 1.0,
            b: 0.0,
            a: 1.0,
        };
        let inherited = Rgba {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        };
        let dl = DisplayList {
            cmds: vec![
                DrawCmd::Rect {
                    x: 10.0,
                    y: 20.0,
                    width: 100.0,
                    height: 40.0,
                    color: green,
                },
                DrawCmd::Text {
                    x: 5.0,
                    y: 8.0,
                    max_width: 90.0,
                    max_height: 20.0,
                    text: "hi".to_owned(),
                    color: inherited, // alpha 0 → fg_fallback
                    font_size: 14.0,
                    line_height: 18.0,
                },
            ],
        };
        let fg = [0.9, 0.9, 0.9, 1.0];
        let (quads, runs) = display_list_to_gpu(&dl, [3.0, 56.0], fg);

        // One quad, origin-offset, alpha preserved (color goes sRGB→linear so we
        // only assert position + alpha, not the channel values).
        assert_eq!(quads.len(), 1);
        assert_eq!(quads[0].pos, [13.0, 76.0], "rect offset by origin");
        assert_eq!(quads[0].size, [100.0, 40.0]);
        assert!((quads[0].color[3] - 1.0).abs() < 1e-6, "alpha preserved");

        // One text run, origin-offset, inherited color → fg_fallback.
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].x, 8.0, "text x offset by origin");
        assert_eq!(runs[0].y, 64.0, "text y offset by origin");
        assert_eq!(runs[0].text, "hi");
        assert_eq!(runs[0].color, fg, "alpha-0 text resolves to fg_fallback");
    }

    #[test]
    fn text_with_explicit_color_keeps_it_over_the_fallback() {
        let red = Rgba {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        };
        let dl = DisplayList {
            cmds: vec![DrawCmd::Text {
                x: 0.0,
                y: 0.0,
                max_width: 10.0,
                max_height: 10.0,
                text: "x".to_owned(),
                color: red,
                font_size: 12.0,
                line_height: 14.0,
            }],
        };
        let (_q, runs) = display_list_to_gpu(&dl, [0.0, 0.0], [0.5, 0.5, 0.5, 1.0]);
        assert_eq!(runs[0].color, [1.0, 0.0, 0.0, 1.0], "explicit color wins");
    }
}
