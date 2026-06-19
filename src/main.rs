use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::num;
use std::path::Path;
use std::sync::mpsc::{channel, RecvError};
use std::time::Duration;

use clap::{App, Arg, ArgMatches};
use thiserror::Error;
use time::OffsetDateTime;

use hdrfix::{
    AutoExposure, ColorMap, HdrFixOptions, Level, PixelFormat, ToneMap,
};
use notify::Watcher;

#[derive(Error, Debug)]
enum LocalError {
    #[error("I/O error: {0}")]
    IoError(#[from] io::Error),
    #[error("numeric format error: {0}")]
    ParseFloatError(#[from] num::ParseFloatError),
    #[error("PNG decoding error: {0}")]
    PNGDecodingError(#[from] png::DecodingError),
    #[error("PNG input must be in 8bpp true color")]
    PNGFormatError,
    #[error("JPEG XR decoding error: {0}")]
    JXRError(#[from] jpegxr::JXRError),
    #[error("Invalid input file type")]
    InvalidInputFile,
    #[error("Invalid output file type")]
    InvalidOutputFile,
    #[error("Unsupported pixel format")]
    UnsupportedPixelFormat,
    #[error("Folder watch error")]
    NotifyError(#[from] notify::Error),
    #[error("Recv error")]
    RecvError(#[from] RecvError),
    #[error("Image format error")]
    ImageError(#[from] image::ImageError),
    #[error("JPEG write failure")]
    JpegWriteFailure,
    #[error("HDR fix error: {0}")]
    HdrFixError(#[from] hdrfix::HdrFixError),
}
use LocalError::*;

type Result<T> = std::result::Result<T, LocalError>;

fn time_func<F, G>(msg: &str, func: F) -> Result<G>
where
    F: FnOnce() -> Result<G>,
{
    let start = OffsetDateTime::now_utc();
    let result = func()?;
    let delta = OffsetDateTime::now_utc() - start;
    println!("{} in {} ms", msg, delta.as_seconds_f64() * 1000.0);
    Ok(result)
}

fn read_png(filename: &Path) -> Result<hdrfix::PixelBuffer> {
    use png::Decoder;
    use png::Transformations;

    let mut decoder = Decoder::new(File::open(filename)?);
    decoder.set_transformations(Transformations::IDENTITY);

    let mut reader = decoder.read_info()?;
    let info = reader.info();

    let format = match (info.bit_depth, info.color_type) {
        (png::BitDepth::Eight, png::ColorType::Rgb) => PixelFormat::HDR8bit,
        (png::BitDepth::Sixteen, png::ColorType::Rgb) => PixelFormat::HDR16bit,
        (_, _) => {
            return Err(PNGFormatError);
        }
    };

    let mut buffer = hdrfix::PixelBuffer::new(info.width as usize, info.height as usize, format);
    reader.next_frame(buffer.bytes_mut())?;

    Ok(buffer)
}

fn read_jxr(filename: &Path) -> Result<hdrfix::PixelBuffer> {
    use jpegxr::ImageDecode;
    use jpegxr::PixelFormat::*;
    use jpegxr::Rect;

    let input = File::open(filename)?;
    let mut decoder = ImageDecode::with_reader(input)?;

    let (width, height) = decoder.get_size()?;
    let format = decoder.get_pixel_format()?;
    let (bytes_per_pixel, buf_fmt) = match format {
        PixelFormat128bppRGBAFloat => (16, PixelFormat::HDRFloat32),
        PixelFormat64bppRGBAHalf => (8, PixelFormat::HDRFloat16),
        PixelFormat32bppRGB101010 => (4, PixelFormat::HDR10bit),
        _ => {
            println!("Pixel format: {:?}", format);
            return Err(UnsupportedPixelFormat);
        }
    };

    let stride = width as usize * bytes_per_pixel;
    let mut buffer = hdrfix::PixelBuffer::new(width as usize, height as usize, buf_fmt);

    let rect = Rect::new(0, 0, width, height);
    decoder.copy(&rect, buffer.bytes_mut(), stride)?;

    Ok(buffer)
}

fn write_png(filename: &Path, width: usize, height: usize, data: &[u8]) -> Result<()> {
    use mtpng::encoder::{Encoder, Options};
    use mtpng::ColorType;
    use mtpng::{CompressionLevel, Header};

    let writer = File::create(filename)?;

    let mut options = Options::new();
    options.set_compression_level(CompressionLevel::High)?;

    let mut header = Header::new();
    header.set_size(width as u32, height as u32)?;
    header.set_color(ColorType::Truecolor, 8)?;

    let mut encoder = Encoder::new(writer, &options);

    encoder.write_header(&header)?;
    encoder.write_image_rows(data)?;
    encoder.finish()?;

    Ok(())
}

fn write_jpeg(filename: &Path, width: usize, height: usize, data: &[u8]) -> Result<()> {
    // @todo allow setting jpeg quality
    // mozjpeg is much faster than image crate's encoder
    std::panic::catch_unwind(|| {
        use mozjpeg::{ColorSpace, Compress};
        let mut c = Compress::new(ColorSpace::JCS_EXT_RGB);
        c.set_size(width, height);
        c.set_quality(95.0);

        let writer = File::create(filename).expect("error creating output file");
        let mut comp = c
            .start_compress(writer)
            .expect("error starting compression");
        comp.write_scanlines(data)
            .expect("error writing scanlines");
        comp.finish()
            .expect("error compressing or writing output file");
    })
    .map_err(|_| JpegWriteFailure)
}

fn extension(input_filename: &Path) -> &str {
    input_filename.extension().unwrap().to_str().unwrap()
}

fn level_from_arg(arg: &str) -> Result<Level> {
    Ok(Level::with_str(arg)?)
}

fn auto_exposure_from_arg(arg: &str) -> Result<AutoExposure> {
    Ok(AutoExposure::with_str(arg)?)
}

fn hdrfix(input_filename: &Path, output_filename: &Path, args: &ArgMatches) -> Result<()> {
    println!(
        "{} -> {}",
        input_filename.to_str().unwrap(),
        output_filename.to_str().unwrap()
    );

    let source = time_func("read_input", || {
        let ext = extension(input_filename);
        match ext {
            "png" => read_png(input_filename),
            "jxr" => read_jxr(input_filename),
            _ => Err(InvalidInputFile),
        }
    })?;
    let width = source.width;
    let height = source.height;

    let pixels: Vec<glam::f32::Vec3> = time_func("decode_pixels", || {
        use rayon::prelude::*;
        Ok(source.pixels().collect())
    })?;

    let options = HdrFixOptions {
        pre_gamma: args
            .value_of("pre-gamma")
            .expect("pre-gamma arg")
            .parse()?,
        pre_levels_min: level_from_arg(
            args.value_of("pre-levels-min")
                .expect("pre-levels-min arg"),
        )?,
        pre_levels_max: level_from_arg(
            args.value_of("pre-levels-max")
                .expect("pre-levels-max arg"),
        )?,
        exposure: args
            .value_of("exposure")
            .expect("exposure arg")
            .parse()?,
        auto_exposure: auto_exposure_from_arg(
            args.value_of("auto-exposure")
                .expect("auto-exposure arg"),
        )?,
        saturation: args
            .value_of("saturation")
            .expect("saturation arg")
            .parse()?,
        tone_map: match args.value_of("tone-map").expect("tone-map arg") {
            "linear" => ToneMap::Linear,
            "reinhard" => ToneMap::Reinhard,
            "reinhard-rgb" => ToneMap::ReinhardRgb,
            "aces" => ToneMap::Aces,
            "uncharted2" => ToneMap::Uncharted2,
            "hable" => ToneMap::Hable,
            _ => unreachable!("bad tone-map option"),
        },
        hdr_max: level_from_arg(args.value_of("hdr-max").expect("hdr-max arg"))?,
        color_map: match args.value_of("color-map").expect("color-map arg") {
            "clip" => ColorMap::Clip,
            "darken" => ColorMap::Darken,
            "desaturate" | "desaturate-oklab" => ColorMap::Desaturate,
            _ => unreachable!("bad color-map option"),
        },
        post_gamma: args
            .value_of("post-gamma")
            .expect("post-gamma arg")
            .parse()?,
        post_levels_min: level_from_arg(
            args.value_of("post-levels-min")
                .expect("post-levels-min arg"),
        )?,
        post_levels_max: level_from_arg(
            args.value_of("post-levels-max")
                .expect("post-levels-max arg"),
        )?,
    };

    let output_data = time_func("hdr_to_sdr", || {
        Ok(hdrfix::convert_scrgb_to_srgb(&pixels, width, height, &options)?)
    })?;

    time_func("write output", || {
        let ext = extension(output_filename);
        match ext {
            "png" => write_png(output_filename, width, height, &output_data),
            "jpg" | "jpeg" => write_jpeg(output_filename, width, height, &output_data),
            _ => Err(InvalidOutputFile),
        }
    })?;

    Ok(())
}

fn run(args: &ArgMatches) -> Result<()> {
    match args.value_of("watch") {
        Some(folder) => {
            let (tx, rx) = channel::<notify::DebouncedEvent>();
            let mut watcher =
                notify::RecommendedWatcher::new(tx, Duration::from_secs(2))?;
            watcher.watch(folder, notify::RecursiveMode::Recursive)?;

            loop {
                let event = rx.recv()?;
                if let notify::DebouncedEvent::Create(input_path) = event {
                    let ext = extension(&input_path);
                    if ext == "jxr" {
                        let mut output_filename: OsString =
                            input_path.file_stem().unwrap().to_os_string();
                        output_filename.push("-sdr.jpg");
                        let output_path = input_path.with_file_name(output_filename);
                        if !output_path.exists() {
                            hdrfix(&input_path, &output_path, args)?;
                        }
                    }
                }
            }
        }
        None => {
            let input_filename =
                Path::new(args.value_of("input").expect("input filename missing"));
            let output_filename =
                Path::new(args.value_of("output").expect("output filename missing"));
            hdrfix(input_filename, output_filename, args)
        }
    }
}

fn main() {
    let args = App::new("hdrfix converter for HDR screenshots")
        .version("1.0.7")
        .author("Brooke Vibber <bvibber@pobox.com>")
        .arg(
            Arg::with_name("input")
                .help("Input filename, must be .jxr or .png as saved by NVIDIA capture overlay.")
                .index(1),
        )
        .arg(
            Arg::with_name("output")
                .help("Output filename, must be .png.")
                .index(2),
        )
        .arg(
            Arg::with_name("auto-exposure")
                .help("Input level or percentile of input data to average to re-expose to neutral 50% mid-tone on input. Default is 0.5, which passes input through unchanged.")
                .long("auto-exposure")
                .default_value("0.5"),
        )
        .arg(
            Arg::with_name("exposure")
                .help("Exposure adjustment in stops, applied after any auto exposure adjustment. May be positive or negative in stops; defaults to 0, which does not change the exposure.")
                .long("exposure")
                .default_value("0"),
        )
        .arg(
            Arg::with_name("tone-map")
                .help("Method for mapping HDR into SDR domain.")
                .long("tone-map")
                .possible_values(&[
                    "linear",
                    "reinhard",
                    "reinhard-rgb",
                    "aces",
                    "uncharted2",
                    "hable",
                ])
                .default_value("hable"),
        )
        .arg(
            Arg::with_name("hdr-max")
                .help("Max HDR luminance level for Reinhard algorithm, in nits or a percentile to be calculated from input data. The default is 100%, which represents the highest input value.")
                .long("hdr-max")
                .default_value("100%"),
        )
        .arg(
            Arg::with_name("saturation")
                .help("Coefficient for how to scale saturation in tone mapping. 1.0 will desaturate linearly to the compression ratio; smaller values will desaturate more aggressively.")
                .long("saturation")
                .default_value("1"),
        )
        .arg(
            Arg::with_name("color-map")
                .help("Method for mapping and fixing out of gamut colors.")
                .long("color-map")
                .possible_values(&["clip", "darken", "desaturate", "desaturate-oklab"])
                .default_value("clip"),
        )
        .arg(
            Arg::with_name("pre-gamma")
                .help("Gamma power applied on input.")
                .long("pre-gamma")
                .default_value("1.0"),
        )
        .arg(
            Arg::with_name("pre-levels-min")
                .help("Minimum input level to normalize to 0 when expanding input for processing. May be an absolute value in -infinity..infinity range or a percentile from 0% to 100%.")
                .long("pre-levels-min")
                .default_value("0.0"),
        )
        .arg(
            Arg::with_name("pre-levels-max")
                .help("Maximum input level to normalize to 1 when expanding input for processing. May be an absolute value in -infinity..infinity range or a percentile from 0% to 100%.")
                .long("pre-levels-max")
                .default_value("1.0"),
        )
        .arg(
            Arg::with_name("post-gamma")
                .help("Gamma power applied on output.")
                .long("post-gamma")
                .default_value("1.0"),
        )
        .arg(
            Arg::with_name("post-levels-min")
                .help("Minimum output level to save when expanding final SDR output for saving. May be an absolute value in 0..1 range or a percentile from 0% to 100%.")
                .long("post-levels-min")
                .default_value("0.0"),
        )
        .arg(
            Arg::with_name("post-levels-max")
                .help("Maximum output level to save when expanding final SDR output for saving. May be an absolute value in 0..1 range or a percentile from 0% to 100%.")
                .long("post-levels-max")
                .default_value("1.0"),
        )
        .arg(
            Arg::with_name("watch")
                .help("Watch a folder and convert any *.jxr files that appear into *-sdr.jpg versions. Provide a folder name.")
                .long("watch")
                .takes_value(true),
        )
        .get_matches();

    match run(&args) {
        Ok(_) => println!("Done."),
        Err(e) => eprintln!("Error: {}", e),
    }
}
