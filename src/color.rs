use glam::f32::{Mat3, Vec3};
use oklab::{linear_srgb_to_oklab, oklab_to_linear_srgb, Oklab};

pub const REC2100_MAX: f32 = 10000.0; // the 1.0 value for BT.2100 linear
pub const SDR_WHITE: f32 = 200.0;

pub fn pq_to_linear(val: Vec3) -> Vec3 {
    let inv_m1: f32 = 1.0 / 0.15930176;
    let inv_m2: f32 = 1.0 / 78.84375;
    let c1 = Vec3::splat(0.8359375);
    let c2 = Vec3::splat(18.851563);
    let c3 = Vec3::splat(18.6875);
    let val_powered = val.powf(inv_m2);
    (Vec3::max(val_powered - c1, Vec3::ZERO) / (c2 - c3 * val_powered)).powf(inv_m1)
}

pub fn rec2100_to_scrgb(val: Vec3) -> Vec3 {
    let matrix = Mat3::from_cols_array(&[
        1.6605, -0.1246, -0.0182,
        -0.5876, 1.1329, -0.1006,
        -0.0728, -0.0083, 1.1187,
    ]);
    let scale = REC2100_MAX / SDR_WHITE;
    matrix.mul_vec3(val * scale)
}

pub fn luma_scrgb(val: Vec3) -> f32 {
    luma_oklab(scrgb_to_oklab(val))
}

pub fn luma_oklab(val: Oklab) -> f32 {
    // oklab's l is not linear
    // so translate it back to linear srgb desaturated
    // and take one of its rgb values
    let oklab_gray = Oklab {
        l: val.l,
        a: 0.0,
        b: 0.0,
    };
    let rgb_gray = oklab_to_scrgb(oklab_gray);
    rgb_gray.x
}

pub fn luma_rgb(val: Vec3) -> f32 {
    val.x * 0.2126 + val.y * 0.7152 + val.z * 0.0722
}

pub fn oklab_l_for_luma(luma: f32) -> f32 {
    let gray_rgb = oklab::Rgb::new(luma, luma, luma);
    let gray_oklab = linear_srgb_to_oklab(gray_rgb);
    gray_oklab.l
}

pub fn scale_oklab_desat(oklab_in: Oklab, luma_out: f32, saturation: f32) -> Oklab {
    let l_in = oklab_in.l;
    if l_in == 0.0 {
        oklab_in
    } else {
        let l_out = oklab_l_for_luma(luma_out);
        // oklab coords scale cubically
        // 1.0 -> desaturate linearly according to luma compression ratio
        // 0.5 -> desaturate more aggressively
        // 2.0 -> saturate more aggressively
        let ratio = (l_out / l_in).powf(3.0 / saturation);
        Oklab {
            l: l_out,
            a: oklab_in.a * ratio,
            b: oklab_in.b * ratio,
        }
    }
}

pub fn scale_oklab(oklab_in: Oklab, luma_out: f32) -> Oklab {
    if oklab_in.l == 0.0 {
        oklab_in
    } else {
        let gray_l = oklab_l_for_luma(luma_out);
        let ratio = gray_l / oklab_in.l;
        Oklab {
            l: gray_l,
            a: oklab_in.a * ratio,
            b: oklab_in.b * ratio,
        }
    }
}

pub fn clip(input: Vec3) -> Vec3 {
    input.max(Vec3::ZERO).min(Vec3::ONE)
}

pub fn scale_rgb(val: Vec3, luma_out: f32) -> Vec3 {
    let luma_in = luma_rgb(val);
    let scale = luma_out / luma_in;
    val * scale
}

fn srgb_to_linear_srgb(c: Vec3) -> oklab::Rgb<f32> {
    oklab::Rgb::new(c.x, c.y, c.z)
}

fn linear_srgb_to_scrgb(c: oklab::Rgb<f32>) -> Vec3 {
    Vec3::new(c.r, c.g, c.b)
}

pub fn scrgb_to_oklab(c: Vec3) -> Oklab {
    linear_srgb_to_oklab(srgb_to_linear_srgb(c))
}

pub fn oklab_to_scrgb(c: Oklab) -> Vec3 {
    linear_srgb_to_scrgb(oklab_to_linear_srgb(c))
}

pub fn linear_to_srgb(val: Vec3) -> Vec3 {
    let min = Vec3::splat(0.0031308);
    let linear = val * Vec3::splat(12.92);
    let gamma = (val * Vec3::splat(1.055)).powf(1.0 / 2.4) - Vec3::splat(0.055);
    Vec3::select(val.cmple(min), linear, gamma)
}

pub fn apply_levels(c_in: Vec3, level_min: f32, level_max: f32, gamma: f32) -> Vec3 {
    let offset = level_min;
    let scale = level_max - level_min;
    let oklab_in = scrgb_to_oklab(c_in);
    let luma_in = luma_oklab(oklab_in);
    let luma_out = ((luma_in - offset) / scale).powf(gamma);
    let oklab_out = scale_oklab(oklab_in, luma_out);
    oklab_to_scrgb(oklab_out)
}

pub fn exposure_scale(stops: f32) -> f32 {
    2.0_f32.powf(stops)
}
