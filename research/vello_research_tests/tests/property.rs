// Copyright 2024 the Vello Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Simple property tests of rendered Vello scenes.

// The following lints are part of the Linebender standard set,
// but resolving them has been deferred for now.
// Feel free to send a PR that solves one or more of these.
#![allow(
    clippy::missing_assert_message,
    clippy::allow_attributes_without_reason
)]

use vello::kurbo::{Affine, Rect};
use vello::peniko::color::palette::css::TRANSPARENT;
use vello::peniko::{Brush, Color, ImageFormat, color::palette};
use vello::peniko::{ImageAlphaType, ImageData, ImageSampler};
use vello::{AaConfig, Scene};
use vello_research_tests::TestParams;

fn simple_square(use_cpu: bool) {
    let mut scene = Scene::new();
    scene.fill(
        vello::peniko::Fill::NonZero,
        Affine::IDENTITY,
        &Brush::Solid(palette::css::RED),
        None,
        &Rect::from_center_size((100., 100.), (50., 50.)),
    );
    let params = TestParams {
        use_cpu,
        ..TestParams::new("simple_square", 150, 150)
    };
    let image = vello_research_tests::render_then_debug_sync(&scene, &params).unwrap();
    assert_eq!(image.format, ImageFormat::Rgba8);
    let mut red_count = 0;
    let mut black_count = 0;
    for pixel in image.data.data().chunks_exact(4) {
        let &[r, g, b, a] = pixel else { unreachable!() };
        let is_red = r == 255 && g == 0 && b == 0 && a == 255;
        let is_black = r == 0 && g == 0 && b == 0 && a == 255;
        if !is_red && !is_black {
            panic!("{pixel:?}");
        }
        match (is_red, is_black) {
            (true, true) => unreachable!(),
            (true, false) => red_count += 1,
            (false, true) => black_count += 1,
            (false, false) => panic!("Got unexpected pixel {pixel:?}"),
        }
    }
    assert_eq!(red_count, 50 * 50);
    assert_eq!(black_count, 150 * 150 - 50 * 50);
}

fn empty_scene(use_cpu: bool) {
    let scene = Scene::new();

    // Adding an alpha factor here changes the resulting color *slightly*,
    // presumably due to pre-multiplied alpha.
    // We just assume that alpha scenarios work fine
    let color = palette::css::PLUM;
    let params = TestParams {
        use_cpu,
        base_color: Some(color),
        ..TestParams::new("simple_square", 150, 150)
    };
    let image = vello_research_tests::render_then_debug_sync(&scene, &params).unwrap();
    assert_eq!(image.format, ImageFormat::Rgba8);
    for pixel in image.data.data().chunks_exact(4) {
        let &[r, g, b, a] = pixel else { unreachable!() };
        let image_color = Color::from_rgba8(r, g, b, a);
        if image_color.premultiply().difference(color.premultiply()) > 1e-4 {
            panic!("Got {image_color:?}, expected clear color {color:?}");
        }
    }
}

/// A blurred rounded rect with a standard deviation of zero is just a sharp rectangle, so if its
/// edges are on pixel boundaries then every pixel must be either fully covered or fully uncovered.
///
/// The rect is drawn through a clip shape which covers the whole image, so that the result only
/// depends on where the blur is evaluated, and not on the coverage of the clip shape.
fn zero_blur_pixel_aligned_rect(use_cpu: bool) {
    const WIDTH: u32 = 40;
    const HEIGHT: u32 = 30;
    let rect = Rect::new(8., 6., 29., 19.);

    let mut scene = Scene::new();
    scene.draw_blurred_rounded_rect_in(
        &Rect::new(0., 0., WIDTH.into(), HEIGHT.into()),
        Affine::IDENTITY,
        rect,
        palette::css::RED,
        0.,
        0.,
    );
    let params = TestParams {
        use_cpu,
        ..TestParams::new("zero_blur_pixel_aligned_rect", WIDTH, HEIGHT)
    };
    let image = vello_research_tests::render_then_debug_sync(&scene, &params).unwrap();
    assert_eq!(image.format, ImageFormat::Rgba8);
    assert_eq!((image.width, image.height), (WIDTH, HEIGHT));
    let coords = (0..HEIGHT).flat_map(|y| (0..WIDTH).map(move |x| (x, y)));
    for ((x, y), pixel) in coords.zip(image.data.data().chunks_exact(4)) {
        let inside = rect.contains((f64::from(x) + 0.5, f64::from(y) + 0.5));
        let expected = if inside {
            [255, 0, 0, 255]
        } else {
            [0, 0, 0, 255]
        };
        assert_eq!(
            pixel,
            expected,
            "Pixel ({x}, {y}) should be fully {}",
            if inside { "covered" } else { "uncovered" }
        );
    }
}

#[test]
#[cfg_attr(skip_gpu_tests, ignore)]
fn simple_square_gpu() {
    simple_square(false);
}

#[test]
// The fine shader still requires a GPU, and so we still get a wgpu device
// skip this for now
#[cfg_attr(skip_gpu_tests, ignore)]
fn simple_square_cpu() {
    simple_square(true);
}

#[test]
#[cfg_attr(skip_gpu_tests, ignore)]
fn empty_scene_gpu() {
    empty_scene(false);
}

#[test]
// The fine shader still requires a GPU, and so we still get a wgpu device
// skip this for now
#[cfg_attr(skip_gpu_tests, ignore)]
fn empty_scene_cpu() {
    empty_scene(true);
}

#[test]
#[cfg_attr(skip_gpu_tests, ignore)]
fn zero_blur_pixel_aligned_rect_gpu() {
    zero_blur_pixel_aligned_rect(false);
}

#[test]
// The fine shader still requires a GPU, and so we still get a wgpu device
// skip this for now
#[cfg_attr(skip_gpu_tests, ignore)]
fn zero_blur_pixel_aligned_rect_cpu() {
    zero_blur_pixel_aligned_rect(true);
}

#[test]
#[cfg_attr(skip_gpu_tests, ignore)]
fn bgra_image() {
    let mut scene = Scene::new();
    let colors = [
        palette::css::RED,
        palette::css::BLUE,
        palette::css::LIME,
        palette::css::WHITE,
    ];
    let blob: Vec<u8> = colors
        .iter()
        .flat_map(|c| {
            let [r, g, b, a] = c.to_rgba8().to_u8_array();
            [b, g, r, a]
        })
        .collect();
    let image = vello::peniko::ImageBrush {
        image: ImageData {
            data: blob.into(),
            format: ImageFormat::Bgra8,
            width: 2,
            height: 2,
            alpha_type: ImageAlphaType::Alpha,
        },
        sampler: ImageSampler {
            quality: vello::peniko::ImageQuality::Low,
            ..Default::default()
        },
    };
    scene.draw_image(&image, Affine::IDENTITY);
    let scene_image =
        vello_research_tests::render_then_debug_sync(&scene, &TestParams::new("bgra", 2, 2))
            .unwrap();
    assert_eq!(scene_image.format, ImageFormat::Rgba8);
    for (i, pixel) in scene_image.data.data().chunks_exact(4).enumerate() {
        let &[r, g, b, a] = pixel else { unreachable!() };
        let image_color = Color::from_rgba8(r, g, b, a);
        let color = colors[i];
        if image_color.premultiply().difference(color.premultiply()) > 1e-4 {
            panic!("Got {image_color:?}, expected color {color:?}");
        }
    }
}

#[test]
#[cfg_attr(skip_gpu_tests, ignore)]
fn premultiplied_image() {
    let mut scene = Scene::new();
    let colors = [
        palette::css::RED.with_alpha(0.5).premultiply(),
        palette::css::BLUE.with_alpha(0.5).premultiply(),
        palette::css::LIME.with_alpha(0.5).premultiply(),
        palette::css::WHITE.with_alpha(0.5).premultiply(),
    ];
    let blob: Vec<u8> = colors
        .iter()
        .flat_map(|c| c.to_rgba8().to_u8_array())
        .collect();
    let image = vello::peniko::ImageBrush {
        image: ImageData {
            data: blob.into(),
            format: ImageFormat::Rgba8,
            width: 2,
            height: 2,
            alpha_type: ImageAlphaType::AlphaPremultiplied,
        },
        sampler: ImageSampler {
            quality: vello::peniko::ImageQuality::Low,
            ..Default::default()
        },
    };
    scene.draw_image(&image, Affine::IDENTITY);
    let params = TestParams {
        width: 2,
        height: 2,
        base_color: Some(TRANSPARENT),
        use_cpu: false,
        name: "bgra".into(),
        anti_aliasing: AaConfig::Area,
    };
    let scene_image = vello_research_tests::render_then_debug_sync(&scene, &params).unwrap();
    assert_eq!(scene_image.format, ImageFormat::Rgba8);
    for (i, pixel) in scene_image.data.data().chunks_exact(4).enumerate() {
        let &[r, g, b, a] = pixel else { unreachable!() };
        let image_color = Color::from_rgba8(r, g, b, a).premultiply();
        let color = colors[i];
        if image_color.difference(color) > 1e-2 {
            panic!("Got {image_color:?}, expected color {color:?}");
        }
    }
}
