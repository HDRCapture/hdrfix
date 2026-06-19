use glam::f32::Vec3;

// 16-bit floats
use half::prelude::*;

use crate::color::{clip, linear_to_srgb, pq_to_linear, rec2100_to_scrgb};

pub enum PixelFormat {
    SDR8bit,
    HDR8bit,
    HDR10bit,
    HDR16bit,
    HDRFloat16,
    HDRFloat32,
}
use PixelFormat::*;

// Note: currently assumes stride == width
pub struct PixelBuffer {
    pub width: usize,
    pub height: usize,
    pub bytes_per_pixel: usize,
    pub data: Vec<u8>,

    // If we wanted these could be traits
    // but we don't need that level of complexity
    pub read_rgb_func: fn(&[u8]) -> Vec3,
    pub write_rgb_func: fn(&mut [u8], Vec3),
}

impl PixelBuffer {
    pub fn new(width: usize, height: usize, format: PixelFormat) -> Self {
        let bytes_per_pixel = match format {
            SDR8bit | HDR8bit => 3,
            HDR10bit => 4,
            HDR16bit => 6,
            HDRFloat16 => 8,
            HDRFloat32 => 16,
        };
        let read_rgb_func = match format {
            SDR8bit => read_srgb_rgb24,
            HDR8bit => read_rec2100_rgb24,
            HDR10bit => read_rec2100_rgb32101010,
            HDR16bit => read_rec2100_rgb48,
            HDRFloat16 => read_scrgb_rgb64half,
            HDRFloat32 => read_scrgb_rgb128float,
        };
        let write_rgb_func = match format {
            SDR8bit => write_srgb_rgb24,
            HDR8bit => write_rec2100_rgb24,
            HDR10bit => write_rec2100_rgb32101010,
            HDR16bit => write_rec2100_rgb48,
            HDRFloat16 => write_scrgb_rgb64half,
            HDRFloat32 => write_scrgb_rgb128float,
        };
        let stride = width * bytes_per_pixel;
        let size = stride * height;
        let data = vec![0u8; size];

        PixelBuffer {
            width,
            height,
            bytes_per_pixel,
            data,
            read_rgb_func,
            write_rgb_func,
        }
    }

    pub fn from_scrgb_pixels(pixels: &[Vec3], width: usize, height: usize) -> Self {
        let mut buffer = Self::new(width, height, HDRFloat32);
        let write_fn = buffer.write_rgb_func;
        let bytes_per_pixel = buffer.bytes_per_pixel;
        buffer
            .data
            .par_chunks_mut(bytes_per_pixel)
            .zip(pixels.par_iter())
            .for_each(|(dest, rgb)| write_fn(dest, *rgb));
        buffer
    }

    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

    pub fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    pub fn par_iter(&self) -> impl IndexedParallelIterator<Item = &[u8]> {
        self.data.par_chunks(self.bytes_per_pixel)
    }

    pub fn par_iter_mut(&mut self) -> impl IndexedParallelIterator<Item = &mut [u8]> {
        self.data.par_chunks_mut(self.bytes_per_pixel)
    }

    pub fn pixels(&self) -> impl '_ + IndexedParallelIterator<Item = Vec3> {
        self.par_iter().map(self.read_rgb_func)
    }

    pub fn fill<T>(&mut self, source: T)
    where
        T: IndexedParallelIterator<Item = Vec3>,
    {
        let write_rgb_func = self.write_rgb_func;
        self.par_iter_mut()
            .zip(source)
            .for_each(|(dest, rgb)| write_rgb_func(dest, rgb))
    }
}

use rayon::prelude::*;

fn read_srgb_rgb24(_data: &[u8]) -> Vec3 {
    panic!("not yet implemented");
}

pub fn write_srgb_rgb24(data: &mut [u8], val: Vec3) {
    let gamma_out = linear_to_srgb(val);
    let clipped = clip(gamma_out);
    let scaled = clipped * 255.0;
    data[0] = scaled.x as u8;
    data[1] = scaled.y as u8;
    data[2] = scaled.z as u8;
}

fn read_rec2100_rgb24(data: &[u8]) -> Vec3 {
    let scale = Vec3::splat(1.0 / 255.0);
    let rgb_rec2100 = Vec3::new(data[0] as f32, data[1] as f32, data[2] as f32) * scale;
    let rgb_linear = pq_to_linear(rgb_rec2100);
    rec2100_to_scrgb(rgb_linear)
}

fn write_rec2100_rgb24(_data: &mut [u8], _rgb: Vec3) {
    panic!("not yet implemented");
}

fn read_rec2100_rgb48(data: &[u8]) -> Vec3 {
    let r = u16::from_be_bytes([data[0], data[1]]);
    let g = u16::from_be_bytes([data[2], data[3]]);
    let b = u16::from_be_bytes([data[4], data[5]]);
    let scale = Vec3::splat(1.0 / 65535.0);
    let rgb_rec2100 = Vec3::new(r as f32, g as f32, b as f32) * scale;
    let rgb_linear = pq_to_linear(rgb_rec2100);
    rec2100_to_scrgb(rgb_linear)
}

fn write_rec2100_rgb48(_data: &mut [u8], _rgb: Vec3) {
    panic!("not yet implemented");
}

fn read_f16_ne(data: &[u8]) -> f32 {
    f16::from_ne_bytes(*data.first_chunk().unwrap()).to_f32()
}

fn read_scrgb_rgb64half(data: &[u8]) -> Vec3 {
    let r = read_f16_ne(&data[0..]);
    let g = read_f16_ne(&data[2..]);
    let b = read_f16_ne(&data[4..]);
    Vec3::new(r, g, b)
}

fn write_f16_ne(data: &mut [u8], n: f32) {
    let bytes = f16::from_f32(n).to_ne_bytes();
    data[0..2].copy_from_slice(&bytes);
}

fn write_scrgb_rgb64half(data: &mut [u8], rgb: Vec3) {
    write_f16_ne(&mut data[0..], rgb.x);
    write_f16_ne(&mut data[2..], rgb.y);
    write_f16_ne(&mut data[4..], rgb.z);
}

fn read_f32_ne(data: &[u8]) -> f32 {
    f32::from_ne_bytes(*data.first_chunk().unwrap())
}

fn read_scrgb_rgb128float(data: &[u8]) -> Vec3 {
    let r = read_f32_ne(&data[0..]);
    let g = read_f32_ne(&data[4..]);
    let b = read_f32_ne(&data[8..]);
    Vec3::new(r, g, b)
}

fn write_f32_ne(data: &mut [u8], n: f32) {
    let bytes = n.to_ne_bytes();
    data[0..4].copy_from_slice(&bytes);
}

fn write_scrgb_rgb128float(data: &mut [u8], rgb: Vec3) {
    write_f32_ne(&mut data[0..], rgb.x);
    write_f32_ne(&mut data[4..], rgb.y);
    write_f32_ne(&mut data[8..], rgb.z);
}

fn read_rec2100_rgb32101010(data: &[u8]) -> Vec3 {
    let data = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let b = (data) & 0x03ff;
    let g = (data >> 10) & 0x03ff;
    let r = (data >> 20) & 0x03ff;
    let max = 1023.0f32;
    let pq = Vec3::new(r as f32 / max, g as f32 / max, b as f32 / max);
    let linear = pq_to_linear(pq);
    rec2100_to_scrgb(linear)
}

fn write_rec2100_rgb32101010(_data: &mut [u8], _rgb: Vec3) {
    panic!("not yet implemented");
}
