// Copyright 2026 the Vello Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! An integration benchmark rendering a synthetic "web page" scene.
//!
//! Real-world web pages (as rendered by HTML engines such as Blitz) produce workloads that
//! are not covered by the existing micro-benchmarks: thousands of small glyphs, hundreds of
//! small solid rectangles (backgrounds, borders), a few scaled images and tens of nested
//! clip layers, all spread over a full-screen viewport. On such scenes the dominant costs
//! are per-command dispatch overhead, tile sorting and strip generation for many small
//! paths, and glyph rendering — this benchmark exercises those end-to-end.
//!
//! The multi-threaded variant requires the `multithreading` feature:
//! `cargo bench --features multithreading -- webpage`.

use std::sync::Arc;

use criterion::Criterion;
use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, GlyphRun, Layout, LayoutContext,
    PositionedLayoutItem,
};
use rand::Rng;
use rand::SeedableRng;
use rand::rngs::SmallRng;
use vello_common::color::{AlphaColor, Srgb};
use vello_common::kurbo::{Affine, Rect, RoundedRect, Shape};
use vello_common::paint::{Image, ImageSource};
use vello_common::peniko::{Extend, ImageAlphaType, ImageQuality, ImageSampler};
use vello_common::pixmap::{PixelMetadata, Pixmap};
use vello_cpu::{Glyph, RenderContext, RenderSettings, Resources};

use crate::SEED;

/// Full-screen viewport at 2x DPI (1366x768 logical).
const VIEWPORT_WIDTH: u16 = 2732;
const VIEWPORT_HEIGHT: u16 = 1536;

/// Web-page-scale integration benchmark.
pub fn webpage(c: &mut Criterion) {
    let mut g = c.benchmark_group("webpage");
    // The scene is much larger than the other benchmarks, so use a reduced sample count.
    g.sample_size(20);

    let scene = SceneData::new();

    #[allow(
        unused_mut,
        reason = "mutated only when the `multithreading` feature is enabled"
    )]
    let mut variants = vec![(
        "single_threaded",
        RenderSettings {
            num_threads: 0,
            ..RenderSettings::default()
        },
    )];

    #[cfg(feature = "multithreading")]
    variants.push(("multi_threaded", RenderSettings::default()));

    for (name, settings) in variants {
        g.bench_function(name, |b| {
            let mut ctx = RenderContext::new_with(VIEWPORT_WIDTH, VIEWPORT_HEIGHT, settings);
            let mut resources = Resources::new();
            let mut pixmap = Pixmap::new(VIEWPORT_WIDTH, VIEWPORT_HEIGHT);

            // Warm up the glyph caches so that steady-state rendering is measured.
            encode_scene(&mut ctx, &mut resources, &scene);
            ctx.flush();
            ctx.render(&mut pixmap, &mut resources);

            b.iter(|| {
                ctx.reset();
                encode_scene(&mut ctx, &mut resources, &scene);
                ctx.flush();
                ctx.render(&mut pixmap, &mut resources);
                std::hint::black_box(&pixmap);
            });
        });
    }

    g.finish();
}

#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct Brush;

/// Pre-computed scene content, built once outside of the benchmark loop so that only
/// encoding and rendering are measured.
struct SceneData {
    heading_layout: Layout<Brush>,
    body_layout: Layout<Brush>,
    sidebar_layout: Layout<Brush>,
    boxes: Vec<(Rect, AlphaColor<Srgb>)>,
    image: ImageSource,
}

const HEADING_TEXT: &str = "The Quick Brown Fox: A Comprehensive Study of Typographic Layout";

const BODY_TEXT: &str = "The quick brown fox jumps over the lazy dog. Pack my box with five \
dozen liquor jugs. How vexingly quick daft zebras jump! Sphinx of black quartz, judge my vow. \
The five boxing wizards jump quickly, while jackdaws love my big sphinx of quartz. ";

/// Palette of typical web page colors.
const PALETTE: [AlphaColor<Srgb>; 6] = [
    AlphaColor::new([0.95, 0.95, 0.95, 1.0]),
    AlphaColor::new([0.88, 0.91, 0.96, 1.0]),
    AlphaColor::new([0.20, 0.40, 0.80, 1.0]),
    AlphaColor::new([0.93, 0.86, 0.80, 1.0]),
    AlphaColor::new([0.30, 0.30, 0.32, 1.0]),
    AlphaColor::new([0.98, 0.92, 0.60, 1.0]),
];

const TEXT_COLOR: AlphaColor<Srgb> = AlphaColor::new([0.1, 0.1, 0.1, 1.0]);
const BACKGROUND_COLOR: AlphaColor<Srgb> = AlphaColor::new([1.0, 1.0, 1.0, 1.0]);

const MAIN_COLUMN_X: f64 = 96.0;
const MAIN_COLUMN_WIDTH: f32 = 1700.0;
const SIDEBAR_X: f64 = 1900.0;
const SIDEBAR_WIDTH: f32 = 736.0;
const SECTION_HEIGHT: f64 = 176.0;
const SECTION_COUNT: usize = 7;
const SIDEBAR_CARD_COUNT: usize = 8;
const SIDEBAR_CARD_HEIGHT: f64 = 160.0;

impl SceneData {
    fn new() -> Self {
        let mut layout_cx = LayoutContext::new();
        let mut font_cx = FontContext::new();

        let mut layout_for = |text: &str, font_size: f32, max_width: f32| {
            let mut builder = layout_cx.ranged_builder(&mut font_cx, text, 1.0, true);
            builder.push_default(FontFamily::named("Roboto"));
            builder.push_default(parley::StyleProperty::FontSize(font_size));
            let mut layout: Layout<Brush> = builder.build(text);
            layout.break_all_lines(Some(max_width));
            layout.align(Alignment::Start, AlignmentOptions::default());
            layout
        };

        // Body text at 16px * 2 (2x DPI), headings larger. The body paragraph is repeated
        // once per section, resulting in a few thousand glyphs in total.
        let heading_layout = layout_for(HEADING_TEXT, 48.0, MAIN_COLUMN_WIDTH);
        let body_text = BODY_TEXT.repeat(3);
        let body_layout = layout_for(&body_text, 32.0, MAIN_COLUMN_WIDTH);
        let sidebar_layout = layout_for(BODY_TEXT, 28.0, SIDEBAR_WIDTH - 48.0);

        // Several hundred small solid rects: buttons, tags, and thin border lines.
        let mut rng = SmallRng::from_seed(SEED);
        let mut boxes = Vec::new();
        for _ in 0..400 {
            let x = rng.random_range(0.0..f64::from(VIEWPORT_WIDTH) - 220.0);
            let y = rng.random_range(0.0..f64::from(VIEWPORT_HEIGHT) - 60.0);
            let (w, h) = if rng.random_range(0_u32..4) == 0 {
                // Thin rect, as used for borders / separator lines.
                (rng.random_range(60.0..220.0), rng.random_range(1.0..4.0))
            } else {
                // Small box, as used for buttons / tags / avatars.
                (rng.random_range(24.0..180.0), rng.random_range(16.0..56.0))
            };
            let color = PALETTE[rng.random_range(0..PALETTE.len())];
            boxes.push((Rect::new(x, y, x + w, y + h), color));
        }

        Self {
            heading_layout,
            body_layout,
            sidebar_layout,
            boxes,
            image: load_flower_image(),
        }
    }
}

fn encode_scene(ctx: &mut RenderContext, resources: &mut Resources, scene: &SceneData) {
    let ImageSource::Pixmap(ref image_pixmap) = scene.image else {
        panic!("Expected Pixmap");
    };
    let image_width = f64::from(image_pixmap.width());

    // Page background.
    ctx.set_paint(BACKGROUND_COLOR);
    ctx.fill_rect(&Rect::new(
        0.0,
        0.0,
        f64::from(VIEWPORT_WIDTH),
        f64::from(VIEWPORT_HEIGHT),
    ));

    // Header bar.
    ctx.set_paint(PALETTE[2]);
    ctx.fill_rect(&Rect::new(0.0, 0.0, f64::from(VIEWPORT_WIDTH), 128.0));

    // Main column, clipped to its overflow box like a scroll container.
    let main_clip = Rect::new(
        MAIN_COLUMN_X - 16.0,
        128.0,
        MAIN_COLUMN_X + f64::from(MAIN_COLUMN_WIDTH) + 16.0,
        f64::from(VIEWPORT_HEIGHT),
    )
    .to_path(0.1);
    ctx.push_clip_layer(&main_clip);

    ctx.set_transform(Affine::translate((MAIN_COLUMN_X, 160.0)));
    ctx.set_paint(TEXT_COLOR);
    render_layout(ctx, resources, &scene.heading_layout);

    // Article sections: each is clipped (overflow: hidden), with a background card,
    // a couple of border lines and a paragraph of body text.
    for i in 0..SECTION_COUNT {
        let y = 260.0 + i as f64 * (SECTION_HEIGHT + 16.0);
        let section_rect = Rect::new(
            MAIN_COLUMN_X,
            y,
            MAIN_COLUMN_X + f64::from(MAIN_COLUMN_WIDTH),
            y + SECTION_HEIGHT,
        );

        ctx.set_transform(Affine::IDENTITY);
        ctx.push_clip_layer(&section_rect.to_path(0.1));

        ctx.set_paint(PALETTE[i % 2]);
        ctx.fill_rect(&section_rect);
        // Top border line.
        ctx.set_paint(PALETTE[4]);
        ctx.fill_rect(&Rect::new(
            section_rect.x0,
            section_rect.y0,
            section_rect.x1,
            section_rect.y0 + 2.0,
        ));

        ctx.set_transform(Affine::translate((MAIN_COLUMN_X + 24.0, y + 16.0)));
        ctx.set_paint(TEXT_COLOR);
        render_layout(ctx, resources, &scene.body_layout);

        ctx.pop_layer();
    }

    ctx.pop_layer();

    // Sidebar cards: rounded-rect (non-rectangular) clips with text and border lines.
    for i in 0..SIDEBAR_CARD_COUNT {
        let y = 160.0 + i as f64 * (SIDEBAR_CARD_HEIGHT + 12.0);
        let card = RoundedRect::new(
            SIDEBAR_X,
            y,
            SIDEBAR_X + f64::from(SIDEBAR_WIDTH),
            y + SIDEBAR_CARD_HEIGHT,
            12.0,
        );

        ctx.set_transform(Affine::IDENTITY);
        ctx.push_clip_layer(&card.to_path(0.1));

        ctx.set_paint(PALETTE[(i + 1) % 3]);
        ctx.fill_rect(&card.rect());

        ctx.set_transform(Affine::translate((SIDEBAR_X + 24.0, y + 12.0)));
        ctx.set_paint(TEXT_COLOR);
        render_layout(ctx, resources, &scene.sidebar_layout);

        ctx.pop_layer();
    }

    // A few scaled image fills (hero images / thumbnails).
    ctx.set_transform(Affine::IDENTITY);
    for (i, width) in [512.0, 384.0, 256.0, 192.0].into_iter().enumerate() {
        let x = 160.0 + i as f64 * 640.0;
        let y = 1180.0;
        let scale = width / image_width;
        ctx.set_paint_transform(Affine::translate((x, y)) * Affine::scale(scale));
        ctx.set_paint(Image {
            image: scene.image.clone(),
            sampler: ImageSampler {
                x_extend: Extend::Pad,
                y_extend: Extend::Pad,
                quality: ImageQuality::Medium,
                alpha: 1.0,
            },
        });
        ctx.fill_rect(&Rect::new(x, y, x + width, y + width * 0.75));
    }
    ctx.reset_paint_transform();

    // Scattered small boxes (buttons, tags, separator lines).
    ctx.set_transform(Affine::IDENTITY);
    for (rect, color) in &scene.boxes {
        ctx.set_paint(*color);
        ctx.fill_rect(rect);
    }
}

fn render_layout(ctx: &mut RenderContext, resources: &mut Resources, layout: &Layout<Brush>) {
    for line in layout.lines() {
        for item in line.items() {
            if let PositionedLayoutItem::GlyphRun(glyph_run) = item {
                render_glyph_run(ctx, resources, &glyph_run);
            }
        }
    }
}

fn render_glyph_run(
    ctx: &mut RenderContext,
    resources: &mut Resources,
    glyph_run: &GlyphRun<'_, Brush>,
) {
    let mut run_x = glyph_run.offset();
    let run_y = glyph_run.baseline();
    let glyphs = glyph_run.glyphs().map(move |glyph| {
        let glyph_x = run_x + glyph.x;
        let glyph_y = run_y - glyph.y;
        run_x += glyph.advance;

        Glyph {
            id: glyph.id,
            x: glyph_x,
            y: glyph_y,
        }
    });

    let run = glyph_run.run();
    ctx.glyph_run(resources, run.font())
        .font_size(run.font_size())
        .hint(true)
        .fill_glyphs(glyphs);
}

fn load_flower_image() -> ImageSource {
    let image_data = include_bytes!("../../../examples/assets/splash-flower.jpg");
    let image = image::load_from_memory(image_data).expect("Failed to decode image");
    let width = image.width();
    let height = image.height();
    let rgba_data = image.into_rgba8().into_vec();

    #[expect(
        clippy::cast_possible_truncation,
        reason = "Image dimensions fit in u16"
    )]
    let pixmap = Pixmap::from_parts(
        rgba_data,
        width as u16,
        height as u16,
        PixelMetadata::new(ImageAlphaType::Alpha, true),
    );

    ImageSource::Pixmap(Arc::new(pixmap))
}
