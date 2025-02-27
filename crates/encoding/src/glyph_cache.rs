// Copyright 2022 the Vello Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::collections::HashMap;

use super::{Encoding, StreamOffsets};

use peniko::{
    kurbo::{BezPath, Shape},
    Fill, Style,
};
use skrifa::{instance::NormalizedCoord, outline::OutlinePen, GlyphId, OutlineGlyphCollection};

#[derive(Copy, Clone, PartialEq, Eq, Hash, Default, Debug)]
pub struct GlyphKey {
    pub font_id: u64,
    pub font_index: u32,
    pub glyph_id: u32,
    pub font_size_bits: u32,
    pub hint: bool,
}

#[derive(Default)]
pub struct GlyphCache {
    pub encoding: Encoding,
    glyphs: HashMap<GlyphKey, CachedRange>,
}

impl GlyphCache {
    pub fn clear(&mut self) {
        self.encoding.reset();
        self.glyphs.clear();
    }

    pub fn get_or_insert(
        &mut self,
        outlines: &OutlineGlyphCollection,
        key: GlyphKey,
        style: &Style,
        font_size: f32,
        coords: &[NormalizedCoord],
    ) -> Option<CachedRange> {
        let size = skrifa::instance::Size::new(font_size);
        let is_var = !coords.is_empty();
        let encoding_cache = &mut self.encoding;
        let mut encode_glyph = || {
            let start = encoding_cache.stream_offsets();
            let fill = match style {
                Style::Fill(fill) => *fill,
                Style::Stroke(_) => Fill::NonZero,
            };
            encoding_cache.encode_fill_style(fill);
            let mut path = encoding_cache.encode_path(true);
            let outline = outlines.get(GlyphId::new(key.glyph_id as u16))?;
            // FIXME: Re-add hinting when skrifa supports it
            // Tracking issue <https://github.com/googlefonts/fontations/issues/620>
            let draw_settings = skrifa::outline::DrawSettings::unhinted(size, coords);
            match style {
                Style::Fill(_) => {
                    outline.draw(draw_settings, &mut path).ok()?;
                }
                Style::Stroke(stroke) => {
                    const STROKE_TOLERANCE: f64 = 0.01;
                    let mut pen = BezPathPen::default();
                    outline.draw(draw_settings, &mut pen).ok()?;
                    let stroked = peniko::kurbo::stroke(
                        pen.0.path_elements(STROKE_TOLERANCE),
                        stroke,
                        &Default::default(),
                        STROKE_TOLERANCE,
                    );
                    path.shape(&stroked);
                }
            }
            if path.finish(false) == 0 {
                return None;
            }
            let end = encoding_cache.stream_offsets();
            Some(CachedRange { start, end })
        };
        // For now, only cache non-zero filled, non-variable glyphs so we don't need to keep style
        // as part of the key.
        let range = if matches!(style, Style::Fill(Fill::NonZero)) && !is_var {
            use std::collections::hash_map::Entry;
            match self.glyphs.entry(key) {
                Entry::Occupied(entry) => *entry.get(),
                Entry::Vacant(entry) => *entry.insert(encode_glyph()?),
            }
        } else {
            encode_glyph()?
        };
        Some(range)
    }
}

#[derive(Copy, Clone, Default, Debug)]
pub struct CachedRange {
    pub start: StreamOffsets,
    pub end: StreamOffsets,
}

impl CachedRange {
    pub fn len(&self) -> StreamOffsets {
        StreamOffsets {
            path_tags: self.end.path_tags - self.start.path_tags,
            path_data: self.end.path_data - self.start.path_data,
            draw_tags: self.end.draw_tags - self.start.draw_tags,
            draw_data: self.end.draw_data - self.start.draw_data,
            transforms: self.end.transforms - self.start.transforms,
            styles: self.end.styles - self.start.styles,
        }
    }
}

// A wrapper newtype so we can implement the `OutlinePen` trait.
#[derive(Default)]
struct BezPathPen(BezPath);

impl OutlinePen for BezPathPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((x as f64, y as f64));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to((x as f64, y as f64));
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0
            .quad_to((cx0 as f64, cy0 as f64), (x as f64, y as f64));
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.curve_to(
            (cx0 as f64, cy0 as f64),
            (cx1 as f64, cy1 as f64),
            (x as f64, y as f64),
        );
    }

    fn close(&mut self) {
        self.0.close_path();
    }
}
