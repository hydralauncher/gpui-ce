use crate::{GlyphRenderMode, RasterStyleRequest, Rgba8};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU32, Ordering};

// Hydra patch: Chromium's grayscale text (see `Window::set_chromium_grayscale_text`).
// The preblend Skia applies to an A8 glyph mask, so a platform rasterizer can
// make the coverage Chromium draws.

// Windows: fitted on Big Picture's toasts against Electron (DirectWrite's scaler
// preblends every A8 mask). Elsewhere: no preblend (contrast 0 and gamma 1 make
// `chromium_preblend_table` the identity), as Chromium's Linux Skia draws a
// FreeType A8 mask (`SK_GAMMA_APPLY_TO_A8` is only defined for Android); not
// yet checked against a Linux reference.
#[cfg(target_os = "windows")]
const DEFAULT_CHROMIUM_TEXT_PREBLEND: (f32, f32) = (0.5, 2.5);
#[cfg(not(target_os = "windows"))]
const DEFAULT_CHROMIUM_TEXT_PREBLEND: (f32, f32) = (0.0, 1.0);

static CHROMIUM_TEXT_CONTRAST: AtomicU32 =
    AtomicU32::new(DEFAULT_CHROMIUM_TEXT_PREBLEND.0.to_bits());
static CHROMIUM_TEXT_GAMMA: AtomicU32 = AtomicU32::new(DEFAULT_CHROMIUM_TEXT_PREBLEND.1.to_bits());

/// The monochrome sprite shader's grayscale correction (gamma ratios, grayscale
/// enhanced contrast) as a renderer without its own Chromium rasterizer
/// published it (`set_chromium_grayscale_shader_correction`): the wgpu renderer.
static GRAYSCALE_SHADER_CORRECTION: std::sync::RwLock<Option<([f32; 4], f32)>> =
    std::sync::RwLock::new(None);

/// Hydra patch: a renderer records the grayscale correction its monochrome
/// sprite shader applies (`apply_contrast_and_gamma_correction`'s gamma ratios
/// and enhanced contrast), so a portable rasterizer can undo it for Chromium's
/// grayscale text ([`chromium_undo_grayscale_correction`]). Process-wide; the
/// last renderer to set it wins.
pub fn set_chromium_grayscale_shader_correction(
    gamma_ratios: [f32; 4],
    grayscale_enhanced_contrast: f32,
) {
    if let Ok(mut slot) = GRAYSCALE_SHADER_CORRECTION.write() {
        *slot = Some((gamma_ratios, grayscale_enhanced_contrast));
    }
}

/// What [`set_chromium_grayscale_shader_correction`] recorded, if a renderer did.
pub fn chromium_grayscale_shader_correction() -> Option<([f32; 4], f32)> {
    GRAYSCALE_SHADER_CORRECTION.read().ok().and_then(|slot| *slot)
}

/// Hydra patch: for each wanted coverage, the mask value the monochrome sprite
/// shader's `apply_contrast_and_gamma_correction` turns into it for text of
/// `color` (its enhanced contrast for dark text, `light_on_dark_contrast`, and
/// its gamma ratios), so the screen gets the wanted coverage.
pub fn chromium_undo_grayscale_correction(
    color: Rgba8,
    gamma_ratios: [f32; 4],
    grayscale_enhanced_contrast: f32,
) -> [u8; 256] {
    let channel = |c: u8| c as f32 / 255.0;
    // REC_601_LUMA_WEIGHTS
    let brightness =
        channel(color.red) * 0.30 + channel(color.green) * 0.59 + channel(color.blue) * 0.11;
    // light_on_dark_contrast: DARK_TEXT_CONTRAST_SCALE, DARK_TEXT_BRIGHTNESS_CUTOFF
    let contrast = grayscale_enhanced_contrast * ((0.75 - brightness) * 4.0).clamp(0.0, 1.0);
    let ratios = gamma_ratios;
    let forward = |alpha: f32| {
        let alpha = alpha * (contrast + 1.0) / (alpha * contrast + 1.0);
        let adjustment = brightness * ratios[0] + ratios[1];
        let correction = adjustment * alpha + (brightness * ratios[2] + ratios[3]);
        alpha + alpha * (1.0 - alpha) * correction
    };
    let mut table = [0u8; 256];
    for (wanted, entry) in table.iter_mut().enumerate() {
        let target = wanted as f32 / 255.0;
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..32 {
            let mid = (lo + hi) / 2.0;
            if forward(mid) < target {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        *entry = (255.0 * (lo + hi) / 2.0).round().clamp(0.0, 255.0) as u8;
    }
    table
}

/// Sets the contrast and gamma of Chromium's grayscale text preblend (Skia's
/// `SkScalerContextRec` contrast and device / paint gamma; a gamma of 0 is
/// sRGB). Process-wide; glyphs already rasterized keep their old masks.
pub fn set_chromium_text_preblend(contrast: f32, gamma: f32) {
    CHROMIUM_TEXT_CONTRAST.store(contrast.to_bits(), Ordering::Relaxed);
    CHROMIUM_TEXT_GAMMA.store(gamma.to_bits(), Ordering::Relaxed);
}

/// The `(contrast, gamma)` set by [`set_chromium_text_preblend`].
pub fn chromium_text_preblend() -> (f32, f32) {
    (
        f32::from_bits(CHROMIUM_TEXT_CONTRAST.load(Ordering::Relaxed)),
        f32::from_bits(CHROMIUM_TEXT_GAMMA.load(Ordering::Relaxed)),
    )
}

/// `SkComputeLuminance`: an A8 mask's preblend follows the text colour's
/// luminance.
pub fn chromium_text_luminance(red: u8, green: u8, blue: u8) -> u8 {
    ((red as u32 * 54 + green as u32 * 183 + blue as u32 * 19) >> 8) as u8
}

fn chromium_to_luma(gamma: f32, value: f32) -> f32 {
    if gamma == 0.0 {
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    } else {
        value.powf(gamma)
    }
}

fn chromium_from_luma(gamma: f32, luma: f32) -> f32 {
    let luma = luma.max(0.0);
    if gamma == 0.0 {
        if luma <= 0.003_130_8 {
            luma * 12.92
        } else {
            1.055 * luma.powf(1.0 / 2.4) - 0.055
        }
    } else {
        luma.powf(1.0 / gamma)
    }
}

/// The coverage table Skia's `SkTMaskGamma<3, 3, 3>` gives an A8 mask of text
/// with `luminance` ([`chromium_text_luminance`]): the luminance keeps 3 bits
/// (`sk_t_scale255`), then `SkTMaskGamma_build_correcting_lut` with the same
/// paint and device gamma.
pub fn chromium_preblend_table(luminance: u8, contrast: f32, gamma: f32) -> [u8; 256] {
    let level = luminance >> 5;
    let src_i = (level << 5) | (level << 2) | (level >> 1);
    let src = src_i as f32 / 255.0;
    let lin_src = chromium_to_luma(gamma, src);
    // the destination is guessed as the inverse (crbug.com/141425)
    let dst = 1.0 - src;
    let lin_dst = chromium_to_luma(gamma, dst);
    // the contrast tapers off to 0 as the source luminance becomes white
    let adjusted_contrast = contrast * lin_dst;
    let apply_contrast = |srca: f32| srca + ((1.0 - srca) * adjusted_contrast * srca);
    let mut table = [0u8; 256];
    for (i, entry) in table.iter_mut().enumerate() {
        let raw_srca = i as f32 / 255.0;
        let srca = apply_contrast(raw_srca);
        let value = if (src - dst).abs() < 1.0 / 256.0 {
            srca
        } else {
            let dsta = 1.0 - srca;
            let lin_out = lin_src * srca + dsta * lin_dst;
            let out = chromium_from_luma(gamma, lin_out);
            (out - dst) / (src - dst)
        };
        *entry = (255.0 * value).round().clamp(0.0, 255.0) as u8;
    }
    table
}

/// The raster style a window in Chromium's grayscale mode asks for: a
/// grayscale mask preblended for `color` (its alpha left out of the key).
pub fn chromium_grayscale_raster_style(color: crate::Rgba) -> PreparedRasterStyle {
    use crate::RasterColorExt as _;
    let color = color.quantize_raster_color();
    PreparedRasterStyle {
        mode: GlyphRenderMode::Grayscale,
        color_effect: RasterColorEffect::Preblend(Rgba8::new(
            color.red,
            color.green,
            color.blue,
            255,
        )),
        foreground_dependency: ForegroundDependency::Full,
    }
}

/// The color a [`chromium_grayscale_raster_style`] mask was preblended for,
/// or None for any other style.
pub fn chromium_grayscale_color(style: &PreparedRasterStyle) -> Option<Rgba8> {
    match (style.mode, style.color_effect) {
        (GlyphRenderMode::Grayscale, RasterColorEffect::Preblend(color)) => Some(color),
        _ => None,
    }
}

/// Hydra patch: the raster style a window with Chromium's LCD preblend asks
/// for (`Window::set_chromium_subpixel_preblend`): a ClearType mask whose
/// three coverages go through Skia's preblend for `color`'s channels
/// (`SkTMaskGamma::preBlend`, `RGBToLcd16<APPLY_PREBLEND>`).
pub fn chromium_subpixel_raster_style(color: crate::Rgba) -> PreparedRasterStyle {
    use crate::RasterColorExt as _;
    let color = color.quantize_raster_color();
    PreparedRasterStyle {
        mode: GlyphRenderMode::Subpixel,
        color_effect: RasterColorEffect::Preblend(Rgba8::new(
            color.red,
            color.green,
            color.blue,
            255,
        )),
        foreground_dependency: ForegroundDependency::Full,
    }
}

/// The color a [`chromium_subpixel_raster_style`] mask was preblended for,
/// or None for any other style.
pub fn chromium_subpixel_color(style: &PreparedRasterStyle) -> Option<Rgba8> {
    match (style.mode, style.color_effect) {
        (GlyphRenderMode::Subpixel, RasterColorEffect::Preblend(color)) => Some(color),
        _ => None,
    }
}

/// Hydra patch: for each wanted coverage of one LCD channel, the mask value
/// the subpixel sprite shader's `apply_contrast_and_gamma_correction3` turns
/// into it for text of `color` (its enhanced contrast for dark text from the
/// colour's luma, the gamma correction from that channel's value).
pub fn chromium_undo_subpixel_correction(
    color: Rgba8,
    channel: usize,
    gamma_ratios: [f32; 4],
    subpixel_enhanced_contrast: f32,
) -> [u8; 256] {
    let value = |c: u8| c as f32 / 255.0;
    let luma = value(color.red) * 0.30 + value(color.green) * 0.59 + value(color.blue) * 0.11;
    let contrast = subpixel_enhanced_contrast * ((0.75 - luma) * 4.0).clamp(0.0, 1.0);
    let brightness = value([color.red, color.green, color.blue][channel.min(2)]);
    let ratios = gamma_ratios;
    let forward = |alpha: f32| {
        let alpha = alpha * (contrast + 1.0) / (alpha * contrast + 1.0);
        let adjustment = brightness * ratios[0] + ratios[1];
        let correction = adjustment * alpha + (brightness * ratios[2] + ratios[3]);
        alpha + alpha * (1.0 - alpha) * correction
    };
    let mut table = [0u8; 256];
    for (wanted, entry) in table.iter_mut().enumerate() {
        let target = wanted as f32 / 255.0;
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..32 {
            let mid = (lo + hi) / 2.0;
            if forward(mid) < target {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        *entry = (255.0 * (lo + hi) / 2.0).round().clamp(0.0, 255.0) as u8;
    }
    table
}

/// Describes which foreground channels can change rasterized color artwork.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ForegroundDependency {
    /// The artwork or its fallback may use the complete foreground color.
    Full,
    /// Fixed RGB artwork uses only the foreground alpha.
    AlphaOnly,
}

/// The cache-relevant raster settings chosen by a platform rasterizer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PreparedRasterStyle {
    /// The normalized render mode.
    pub mode: GlyphRenderMode,
    /// Any normalized color effect baked into coverage or color pixels.
    pub color_effect: RasterColorEffect,
    /// Which foreground channels can alter this raster or its fallback.
    pub foreground_dependency: ForegroundDependency,
}

impl PreparedRasterStyle {
    /// Creates a color-independent raster style.
    pub const fn independent(mode: GlyphRenderMode) -> Self {
        Self {
            mode,
            color_effect: RasterColorEffect::Independent,
            foreground_dependency: ForegroundDependency::Full,
        }
    }

    /// Creates a native preblended style from the request's relevant color channels.
    pub fn preblend(request: RasterStyleRequest) -> Self {
        Self {
            mode: request.requested_mode,
            color_effect: RasterColorEffect::Preblend(request.normalized_color()),
            foreground_dependency: request.foreground_dependency,
        }
    }
}

/// The part of the requested color, if any, which changes rasterized pixels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RasterColorEffect {
    /// Coverage and color pixels do not depend on the scene color.
    Independent,
    /// CoreGraphics' five-level font-smoothing dilation.
    Dilation(u8),
    /// A quantized color consumed by a native preblending or `currentColor` path.
    Preblend(Rgba8),
}

impl Hash for RasterColorEffect {
    fn hash<H: Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);

        match self {
            Self::Independent => {}
            Self::Dilation(dilation) => dilation.hash(state),
            Self::Preblend(color) => <[u8; 4]>::from(*color).hash(state),
        }
    }
}
