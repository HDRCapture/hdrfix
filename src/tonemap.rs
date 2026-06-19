use glam::f32::Vec3;

use crate::color::{
    luma_oklab, luma_rgb, oklab_to_scrgb, scale_oklab_desat, scale_rgb, scrgb_to_oklab,
};
use crate::Options;

#[derive(Copy, Clone, Debug)]
pub enum ToneMap {
    Linear,
    Reinhard,
    ReinhardRgb,
    Uncharted2,
    Hable,
    Aces,
}

pub fn tonemap_linear(c_in: Vec3, _options: &Options) -> Vec3 {
    c_in
}

pub fn tonemap_reinhard_rgb(c_in: Vec3, options: &Options) -> Vec3 {
    let white = options.hdr_max;
    let white2 = white * white;
    c_in * (Vec3::ONE + c_in / white2) / (Vec3::ONE + c_in)
}

pub fn tonemap_reinhard_oklab(c_in: Vec3, options: &Options) -> Vec3 {
    let white = options.hdr_max;
    let white2 = white * white;

    let oklab_in = scrgb_to_oklab(c_in);
    let luma_in = luma_oklab(oklab_in);

    let luma_out = luma_in * (1.0 + luma_in / white2) / (1.0 + luma_in);
    let oklab_out = scale_oklab_desat(oklab_in, luma_out, options.saturation);
    oklab_to_scrgb(oklab_out)
}

// https://64.github.io/tonemapping/#uncharted-2
// Uncharted 2 / Hable Filmic
fn uncharted2_tonemap_partial(x: f32) -> f32 {
    const A: f32 = 0.15;
    const B: f32 = 0.50;
    const C: f32 = 0.10;
    const D: f32 = 0.20;
    const E: f32 = 0.02;
    const F: f32 = 0.30;
    ((x * (A * x + (C * B)) + (D * E)) / (x * (A * x + (B)) + (D * F))) - (E / F)
}

pub fn tonemap_uncharted2(v: Vec3, _options: &Options) -> Vec3 {
    let exposure_bias: f32 = 2.0;
    let luma = luma_rgb(v);
    let curr = uncharted2_tonemap_partial(luma * exposure_bias);

    let w = 11.2f32;
    let white_scale = 1.0f32 / uncharted2_tonemap_partial(w);
    let luma_out = curr * white_scale;

    scale_rgb(v, luma_out)
}

pub fn tonemap_hable(val: Vec3, _options: &Options) -> Vec3 {
    // stolen from ffmpeg's vf_tonemap

    // desat
    let luma = luma_rgb(val);
    let desaturation: f32 = 2.0;
    let epsilon: f32 = 1e-6;
    let overbright = f32::max(luma - desaturation, epsilon) / f32::max(luma, epsilon);
    let rgb_out = val * (1.0 - overbright) + luma * overbright;
    let sig_orig = f32::max(rgb_out.max_element(), epsilon);

    // hable/uncharted2
    let exposure_bias: f32 = 2.0;
    let luma = sig_orig;
    let curr = uncharted2_tonemap_partial(luma * exposure_bias);
    let w = 11.2f32;
    let white_scale = 1.0f32 / uncharted2_tonemap_partial(w);
    let sig = curr * white_scale;

    rgb_out * (sig / sig_orig)
}

// can't use glam's Mat3 as a constant literal?
type Matrix3x3 = [[f32; 3]; 3];

// https://64.github.io/tonemapping/#aces
// ACES (Academy Color Encoding System)
const ACES_INPUT_MATRIX: Matrix3x3 =
    [[0.59719, 0.35458, 0.04823], [0.07600, 0.90834, 0.01566], [0.02840, 0.13383, 0.83777]];

const ACES_OUTPUT_MATRIX: Matrix3x3 =
    [[1.60475, -0.53108, -0.07367], [-0.10208, 1.10813, -0.00605], [-0.00327, -0.07276, 1.07602]];

#[allow(clippy::many_single_char_names)]
fn aces_mul(m: &Matrix3x3, v: Vec3) -> Vec3 {
    let x = m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2];
    let y = m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2];
    let z = m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2];
    Vec3::new(x, y, z)
}

fn aces_rtt_and_odt_fit(v: Vec3) -> Vec3 {
    let a = v * (v + Vec3::splat(0.0245786)) - Vec3::splat(0.000090537);
    let b = v * (Vec3::splat(0.983729) * v + Vec3::splat(0.432951)) + Vec3::splat(0.238081);
    a / b
}

pub fn tonemap_aces(c_in: Vec3, _options: &Options) -> Vec3 {
    let v = c_in;
    let v = aces_mul(&ACES_INPUT_MATRIX, v);
    let v = aces_rtt_and_odt_fit(v);
    aces_mul(&ACES_OUTPUT_MATRIX, v)
}

pub(crate) fn tone_map_fn(tone_map: &ToneMap) -> fn(Vec3, &Options) -> Vec3 {
    match tone_map {
        ToneMap::Linear => tonemap_linear,
        ToneMap::Reinhard => tonemap_reinhard_oklab,
        ToneMap::ReinhardRgb => tonemap_reinhard_rgb,
        ToneMap::Uncharted2 => tonemap_uncharted2,
        ToneMap::Hable => tonemap_hable,
        ToneMap::Aces => tonemap_aces,
    }
}
