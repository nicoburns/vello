// Copyright 2025 the Vello Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Types for paints.

use crate::pixmap::Pixmap;
use alloc::sync::Arc;
pub use peniko::Color;
use peniko::{
    Gradient,
    color::{AlphaColor, PremulRgba8, Srgb},
};

/// A paint that needs to be resolved via its index.
// In the future, we might add additional flags, that's why we have
// this thin wrapper around u32, so we can change the underlying
// representation without breaking the API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedPaint(u32);

impl IndexedPaint {
    /// Create a new indexed paint from an index.
    pub fn new(index: usize) -> Self {
        Self(u32::try_from(index).expect("exceeded the maximum number of paints"))
    }

    /// Return the index of the paint.
    pub fn index(&self) -> usize {
        usize::try_from(self.0).unwrap()
    }
}

/// A paint that is used internally by a rendering frontend to store how a wide tile command
/// should be painted. There are only two types of paint:
///
/// 1) Simple solid colors, which are stored in premultiplied representation so that
///    each wide tile doesn't have to recompute it.
/// 2) Indexed paints, which can represent any arbitrary, more complex paint that is
///    determined by the frontend. The intended way of using this is to store a vector
///    of paints and store its index inside `IndexedPaint`.
#[derive(Debug, Clone, PartialEq)]
pub enum Paint {
    /// A premultiplied RGBA8 color.
    Solid(PremulColor),
    /// A paint that needs to be resolved via an index.
    Indexed(IndexedPaint),
}

impl From<AlphaColor<Srgb>> for Paint {
    fn from(value: AlphaColor<Srgb>) -> Self {
        Self::Solid(PremulColor::from_alpha_color(value))
    }
}

/// Opaque image handle
#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug)]
pub struct ImageId(u32);

impl ImageId {
    // TODO: make this private in future
    /// Create a new image id from a u32.
    pub fn new(value: u32) -> Self {
        Self(value)
    }

    /// Return the image id as a u32.
    pub fn as_u32(&self) -> u32 {
        self.0
    }
}

/// Bitmap source used by `Image`.
#[derive(Debug, Clone)]
pub enum ImageSource {
    /// Pixmap pixels travel with the scene packet.
    Pixmap(Arc<Pixmap>),
    /// Pixmap pixels were registered earlier; this is just a handle.
    OpaqueId {
        /// The image handle.
        id: ImageId,
        /// Whether the image may contain non-opaque pixels.
        may_have_transparency: bool,
    },
}

impl ImageSource {
    /// Create an [`ImageSource`] from a pre-registered image handle.
    ///
    /// Conservatively assumes the image may have non-opaque pixels.
    /// Use [`Self::opaque_id_with_transparency_hint`] when you know the image is fully opaque.
    pub fn opaque_id(id: ImageId) -> Self {
        Self::OpaqueId {
            id,
            may_have_transparency: true,
        }
    }

    /// Create an [`ImageSource`] from a pre-registered image handle,
    /// with an explicit hint about whether the image may have non-opaque pixels.
    pub fn opaque_id_with_transparency_hint(id: ImageId, may_have_transparency: bool) -> Self {
        Self::OpaqueId {
            id,
            may_have_transparency,
        }
    }

    /// Returns whether this image source may contain non-opaque pixels.
    pub fn may_have_transparency(&self) -> bool {
        match self {
            Self::Pixmap(p) => p.may_have_transparency(),
            Self::OpaqueId {
                may_have_transparency,
                ..
            } => *may_have_transparency,
        }
    }

    /// Convert a [`peniko::ImageData`] to an [`ImageSource`].
    ///
    /// This is a somewhat lossy conversion, as the image data data is transformed to
    /// [premultiplied RGBA8](`PremulRgba8`).
    ///
    /// # Panics
    ///
    /// This panics if `image` has a `width` or `height` greater than `u16::MAX`.
    pub fn from_peniko_image_data(image: &peniko::ImageData) -> Self {
        // TODO: how do we deal with `peniko::ImageFormat` growing? See also
        // <https://github.com/linebender/vello/pull/996#discussion_r2080510863>.
        let do_alpha_multiply = image.alpha_type != peniko::ImageAlphaType::AlphaPremultiplied;

        assert!(
            image.width <= u16::MAX as u32 && image.height <= u16::MAX as u32,
            "The image is too big. Its width and height can be no larger than {} pixels.",
            u16::MAX,
        );
        let width = image.width.try_into().unwrap();
        let height = image.height.try_into().unwrap();

        // TODO: SIMD
        #[expect(clippy::cast_possible_truncation, reason = "This cannot overflow.")]
        let pixels = image
            .data
            .data()
            .chunks_exact(4)
            .map(|pixel| {
                let rgba: [u8; 4] = match image.format {
                    peniko::ImageFormat::Rgba8 => pixel.try_into().unwrap(),
                    peniko::ImageFormat::Bgra8 => [pixel[2], pixel[1], pixel[0], pixel[3]],
                    format => unimplemented!("Unsupported image format: {format:?}"),
                };
                let alpha = u16::from(rgba[3]);
                let multiply = |component| ((alpha * u16::from(component)) / 255) as u8;
                if do_alpha_multiply {
                    PremulRgba8 {
                        r: multiply(rgba[0]),
                        g: multiply(rgba[1]),
                        b: multiply(rgba[2]),
                        a: rgba[3],
                    }
                } else {
                    PremulRgba8 {
                        r: rgba[0],
                        g: rgba[1],
                        b: rgba[2],
                        a: rgba[3],
                    }
                }
            })
            .collect();
        let pixmap = Pixmap::from_parts(pixels, width, height);

        Self::Pixmap(Arc::new(pixmap))
    }
}

/// An image.
pub type Image = peniko::ImageBrush<ImageSource>;

/// Trait for resolving opaque image IDs to pixmaps at rasterization time.
///
/// This allows delaying the resolution of `ImageSource::OpaqueId` until the
/// image is actually needed during rasterization, enabling patterns like
/// dynamic sprite atlases where the image data may be updated between
/// encoding and rendering.
pub trait ImageResolver: Send + Sync {
    /// Resolve an `ImageId` to its pixmap data.
    ///
    /// This method may be called repeatedly (dozens or even hundreds of times
    /// per frame) and should therefore be very fast.
    ///
    /// Returns `None` if the image ID is not found in the registry.
    fn resolve(&self, id: ImageId) -> Option<Arc<Pixmap>>;
}

/// A no-op image resolver that always returns `None`.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoOpImageResolver;

impl ImageResolver for NoOpImageResolver {
    fn resolve(&self, _id: ImageId) -> Option<Arc<Pixmap>> {
        None
    }
}

/// A premultiplied color.
#[derive(Debug, Clone, PartialEq, Copy)]
pub struct PremulColor {
    premul_u8: PremulRgba8,
    premul_f32: peniko::color::PremulColor<Srgb>,
}

impl PremulColor {
    /// Create a new premultiplied color.
    pub fn from_alpha_color(color: AlphaColor<Srgb>) -> Self {
        Self::from_premul_color(color.premultiply())
    }

    /// Create a new premultiplied color from `peniko::PremulColor`.
    pub fn from_premul_color(color: peniko::color::PremulColor<Srgb>) -> Self {
        Self {
            premul_u8: color.to_rgba8(),
            premul_f32: color,
        }
    }

    /// Return the color as a premultiplied RGBA8 color.
    pub fn as_premul_rgba8(&self) -> PremulRgba8 {
        self.premul_u8
    }

    /// Return the color as a premultiplied RGBAF32 color.
    pub fn as_premul_f32(&self) -> peniko::color::PremulColor<Srgb> {
        self.premul_f32
    }

    /// Return whether the color is opaque (i.e. doesn't have transparency).
    pub fn is_opaque(&self) -> bool {
        self.premul_f32.components[3] == 1.0
    }
}

/// How tint color is applied to an image.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TintMode {
    /// Alpha-mask tinting: `tint_premul * source.alpha`.
    ///
    /// The source image's alpha channel is used as a coverage mask,
    /// and the result is filled with the premultiplied tint color.
    /// This is the standard approach for glyph / monochrome image tinting.
    AlphaMask = 0,
    /// Component-wise multiply: `source * tint`.
    ///
    /// Each channel of the source pixel is multiplied by the corresponding
    /// channel of the tint color. This works well for full-color images.
    Multiply = 1,
}

impl TintMode {
    /// Return the discriminant as a `u32`.
    pub fn as_u32(self) -> u32 {
        self as u32
    }
}

/// A tint applied to image paints.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Tint {
    /// The tint color.
    pub color: Color,
    /// How the tint is applied.
    pub mode: TintMode,
    /// Luminance-aware text contrast in `[0, 1]`, applied to the coverage of
    /// [`TintMode::AlphaMask`] tints (i.e. glyphs).
    ///
    /// A value of `0` disables the correction. Larger values increase the strength of the
    /// adjustment. See [`Tint::coverage_gain`] for the model used.
    pub contrast: f32,
}

impl Tint {
    /// Compute the signed coverage-gain factor `k` for luminance-aware text contrast.
    ///
    /// This implements a cheap, monotonic, endpoint-preserving approximation of gamma-correct
    /// text blending. The corrected coverage is
    ///
    /// ```text
    /// a' = a + k * a * (1 - a)
    /// ```
    ///
    /// where `a` is the glyph's anti-aliasing coverage and `k = contrast * (2 * L - 1)`, with `L`
    /// the (premultiplied) Rec. 709 luminance of the tint color in `[0, 1]`.
    ///
    /// Dark text (low `L`) gets `k < 0`, which thins the anti-aliased edges; light text (high `L`)
    /// gets `k > 0`, which thickens them. This counteracts the perceptual weight shift that occurs
    /// when compositing in non-linear sRGB space, so text keeps a consistent weight across light
    /// and dark backgrounds. Returns `0` when no correction should be applied.
    #[inline]
    pub fn coverage_gain(&self) -> f32 {
        if self.contrast == 0.0 || self.mode != TintMode::AlphaMask {
            return 0.0;
        }
        let [r, g, b, _] = self.color.premultiply().components;
        let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        self.contrast * (2.0 * luminance - 1.0)
    }
}

/// A kind of paint that can be used for filling and stroking shapes.
pub type PaintType = peniko::Brush<Image, Gradient>;

#[cfg(test)]
mod tests {
    use super::{Color, Tint, TintMode};

    fn tint(color: Color, mode: TintMode, contrast: f32) -> Tint {
        Tint {
            color,
            mode,
            contrast,
        }
    }

    #[test]
    fn coverage_gain_disabled_when_contrast_zero() {
        assert_eq!(
            tint(Color::BLACK, TintMode::AlphaMask, 0.0).coverage_gain(),
            0.0
        );
        assert_eq!(
            tint(Color::WHITE, TintMode::AlphaMask, 0.0).coverage_gain(),
            0.0
        );
    }

    #[test]
    fn coverage_gain_only_applies_to_alpha_mask() {
        assert_eq!(
            tint(Color::WHITE, TintMode::Multiply, 1.0).coverage_gain(),
            0.0
        );
    }

    #[test]
    fn coverage_gain_is_luminance_aware() {
        // Dark text thins the coverage (`k < 0`); light text thickens it (`k > 0`).
        assert!(tint(Color::BLACK, TintMode::AlphaMask, 1.0).coverage_gain() < 0.0);
        assert!(tint(Color::WHITE, TintMode::AlphaMask, 1.0).coverage_gain() > 0.0);

        // Pure black/white map to exactly `-contrast`/`+contrast`.
        assert_eq!(
            tint(Color::BLACK, TintMode::AlphaMask, 1.0).coverage_gain(),
            -1.0
        );
        assert_eq!(
            tint(Color::WHITE, TintMode::AlphaMask, 1.0).coverage_gain(),
            1.0
        );
    }

    #[test]
    fn coverage_gain_scales_with_contrast() {
        let half = tint(Color::WHITE, TintMode::AlphaMask, 0.5).coverage_gain();
        let full = tint(Color::WHITE, TintMode::AlphaMask, 1.0).coverage_gain();
        assert!((half - 0.5).abs() < 1e-6);
        assert!((full - 1.0).abs() < 1e-6);
    }
}
