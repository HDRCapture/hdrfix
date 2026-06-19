use std::num;

use glam::f32::Vec3;
use rayon::prelude::*;
use thiserror::Error;

mod buffer;
mod color;
mod histogram;
mod colormap;
mod tonemap;

pub use buffer::{PixelBuffer, PixelFormat};
pub use colormap::ColorMap;
pub use tonemap::ToneMap;

use color::{apply_levels, clip, exposure_scale};
use colormap::color_map_fn;
use histogram::Histogram;
use tonemap::tone_map_fn;

#[derive(Error, Debug)]
pub enum HdrFixError {
    #[error("numeric format error: {0}")]
    ParseFloatError(#[from] num::ParseFloatError),
}

#[derive(Copy, Clone, Debug)]
pub enum Level {
    Scalar(f32),
    Percentile(f32),
}

impl Level {
    pub fn with_str(source: &str) -> Result<Self, HdrFixError> {
        match source.strip_suffix('%') {
            Some(val) => Ok(Self::Percentile(val.parse()?)),
            None => Ok(Self::Scalar(source.parse::<f32>()?)),
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub enum AutoExposure {
    Value(f32),
    Percentile(f32),
}

impl AutoExposure {
    pub fn with_str(source: &str) -> Result<Self, HdrFixError> {
        match source.strip_suffix('%') {
            Some(val) => Ok(Self::Percentile(val.parse()?)),
            None => Ok(Self::Value(source.parse::<f32>()?)),
        }
    }
}

#[derive(Clone, Debug)]
pub struct HdrFixOptions {
    pub exposure: f32,
    pub auto_exposure: AutoExposure,
    pub tone_map: ToneMap,
    pub hdr_max: Level,
    pub saturation: f32,
    pub color_map: ColorMap,
    pub pre_gamma: f32,
    pub pre_levels_min: Level,
    pub pre_levels_max: Level,
    pub post_gamma: f32,
    pub post_levels_min: Level,
    pub post_levels_max: Level,
}

impl Default for HdrFixOptions {
    fn default() -> Self {
        Self {
            exposure: 0.0,
            auto_exposure: AutoExposure::Value(0.5),
            tone_map: ToneMap::Hable,
            hdr_max: Level::Percentile(100.0),
            saturation: 1.0,
            color_map: ColorMap::Clip,
            pre_gamma: 1.0,
            pre_levels_min: Level::Scalar(0.0),
            pre_levels_max: Level::Scalar(1.0),
            post_gamma: 1.0,
            post_levels_min: Level::Scalar(0.0),
            post_levels_max: Level::Scalar(1.0),
        }
    }
}

pub(crate) struct Options {
    pub scale: f32,
    pub hdr_max: f32,
    pub saturation: f32,
    pub tone_map: fn(Vec3, &Options) -> Vec3,
    pub color_map: fn(Vec3) -> Vec3,
}

fn hdr_to_sdr_pixel(rgb_scrgb: Vec3, options: &Options) -> Vec3 {
    let val = rgb_scrgb * options.scale;
    let val = (options.tone_map)(val, options);
    (options.color_map)(val)
}

struct Lazy<T, F>
where
    F: (FnOnce() -> T),
{
    value: Option<T>,
    func: Option<F>,
}

impl<T, F> Lazy<T, F>
where
    F: (FnOnce() -> T),
{
    fn new(func: F) -> Self {
        Lazy {
            value: None,
            func: Some(func),
        }
    }

    fn force(&mut self) -> &T {
        if self.value.is_none() {
            let func = self.func.take().unwrap();
            self.value = Some(func());
        }
        self.value.as_ref().unwrap()
    }
}

impl<F> Lazy<Histogram, F>
where
    F: (FnOnce() -> Histogram),
{
    fn level(&mut self, level: Level) -> f32 {
        match level {
            Level::Scalar(val) => val,
            Level::Percentile(val) => self.force().percentile(val),
        }
    }
}

pub fn convert_scrgb_to_srgb(
    input: &[Vec3],
    width: usize,
    height: usize,
    options: &HdrFixOptions,
) -> Result<Vec<u8>, HdrFixError> {
    let pixel_count = width * height;
    assert_eq!(
        input.len(),
        pixel_count,
        "input pixel count must match width * height"
    );

    let mut pixels: Vec<Vec3> = input.to_vec();

    // Pre-levels (lazy histogram for percentile-based levels)
    let mut pre_histogram = Lazy::new(|| Histogram::from_pixels(&pixels));
    let pre_levels_min = pre_histogram.level(options.pre_levels_min);
    let pre_levels_max = pre_histogram.level(options.pre_levels_max);
    pixels
        .par_iter_mut()
        .for_each(|rgb| *rgb = apply_levels(*rgb, pre_levels_min, pre_levels_max, options.pre_gamma));

    // Auto-exposure histogram
    let mut input_histogram = Lazy::new(|| Histogram::from_pixels(&pixels));
    let scale = exposure_scale(options.exposure) * 0.5
        / match options.auto_exposure {
            AutoExposure::Value(level) => level,
            AutoExposure::Percentile(percent) => input_histogram.force().average_below_percentile(percent),
        };

    let hdr_max = match options.hdr_max {
        Level::Scalar(nits) => nits / color::SDR_WHITE,
        Level::Percentile(val) => input_histogram.force().percentile(val),
    } * scale;

    let internal_options = Options {
        scale,
        hdr_max,
        saturation: options.saturation,
        tone_map: tone_map_fn(&options.tone_map),
        color_map: color_map_fn(&options.color_map),
    };

    // Tone mapping
    pixels
        .par_iter_mut()
        .for_each(|rgb| *rgb = hdr_to_sdr_pixel(*rgb, &internal_options));

    // Post-levels (lazy histogram)
    let mut post_histogram = Lazy::new(|| Histogram::from_pixels(&pixels));
    let post_levels_min = post_histogram.level(options.post_levels_min);
    let post_levels_max = post_histogram.level(options.post_levels_max);

    let color_map_fn = internal_options.color_map;
    pixels.par_iter_mut().for_each(|rgb| {
        *rgb = clip(color_map_fn(apply_levels(
            *rgb,
            post_levels_min,
            post_levels_max,
            options.post_gamma,
        )));
    });

    // Convert to SDR8bit buffer and extract bytes
    let mut dest = PixelBuffer::new(width, height, PixelFormat::SDR8bit);
    dest.fill(pixels.par_iter().copied());
    Ok(dest.data)
}
