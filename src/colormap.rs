use std::cmp::Ordering;

use glam::f32::Vec3;
use oklab::Oklab;

use crate::color::{clip, oklab_to_scrgb, scrgb_to_oklab};

#[derive(Copy, Clone, Debug)]
pub enum ColorMap {
    Clip,
    Darken,
    Desaturate,
}

pub fn color_clip(input: Vec3) -> Vec3 {
    clip(input)
}

fn darken_oklab(c_in: Oklab, brightness: f32) -> Vec3 {
    let c_out = Oklab {
        l: c_in.l * brightness,
        a: c_in.a * brightness,
        b: c_in.b * brightness,
    };
    oklab_to_scrgb(c_out)
}

fn desat_oklab(c_in: Oklab, saturation: f32) -> Vec3 {
    let c_out = Oklab {
        l: c_in.l,
        a: c_in.a * saturation,
        b: c_in.b * saturation,
    };
    oklab_to_scrgb(c_out)
}

const EPSILON: f32 = 0.001; // good enough for us for now

fn close_enough(a: f32, b: f32) -> Ordering {
    let delta = a - b;
    if delta.abs() < EPSILON {
        Ordering::Equal
    } else if delta < 0.0 {
        Ordering::Less
    } else {
        Ordering::Greater
    }
}

fn binary_search<I, O, F, G>(input: I, min: f32, max: f32, func: F, comparator: G) -> O
where
    I: Copy + Clone,
    O: Copy + Clone,
    F: Fn(I, f32) -> O,
    G: Fn(O) -> Ordering,
{
    let mid = (min + max) / 2.0;
    let result = func(input, mid);
    match close_enough(min, max) {
        Ordering::Equal => result,
        _ => match comparator(result) {
            Ordering::Less => binary_search(input, mid, max, func, comparator),
            Ordering::Greater => binary_search(input, min, mid, func, comparator),
            Ordering::Equal => result,
        },
    }
}

pub fn color_darken_oklab(c_in: Vec3) -> Vec3 {
    let max = c_in.max_element();
    if max > 1.0 {
        let c_in_oklab = scrgb_to_oklab(c_in);
        let c_out = binary_search(c_in_oklab, 0.0, 1.0, darken_oklab, |rgb| {
            close_enough(rgb.max_element(), 1.0)
        });
        clip(c_out)
    } else {
        c_in
    }
}

pub fn color_desat_oklab(c_in: Vec3) -> Vec3 {
    let max = c_in.max_element();
    if max > 1.0 {
        let c_in_oklab = scrgb_to_oklab(c_in);
        let c_out = binary_search(c_in_oklab, 0.0, 1.0, desat_oklab, |rgb| {
            close_enough(rgb.max_element(), 1.0)
        });
        clip(c_out)
    } else {
        c_in
    }
}

pub(crate) fn color_map_fn(color_map: &ColorMap) -> fn(Vec3) -> Vec3 {
    match color_map {
        ColorMap::Clip => color_clip,
        ColorMap::Darken => color_darken_oklab,
        ColorMap::Desaturate => color_desat_oklab,
    }
}
