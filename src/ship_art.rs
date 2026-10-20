//! Turn the colour of the player ships.
//!
//! Each layout gets one angle, and every image that carries its name - the hull
//! art and the six gibs - is turned by that angle, so a ship and the wreck it
//! breaks into stay the same colour. Only the hue moves: saturation and lightness
//! are left alone, so a ship keeps its shading and its dark plating. Angles too
//! small to notice are not drawn.
//!
//! Like the rest of the library this works on an archive in memory.

use std::collections::BTreeMap;
use std::io::Cursor;

use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::ImageReader;

use crate::rng::Rng;
use crate::{Error, FtlArchive};

/// The angles a ship may turn by. Below one, or above the other, the ship looks
/// exactly as the game shipped it, which is not what a randomiser is for.
const TURN_MIN: i32 = 21;
const TURN_MAX: i32 = 339;

/// What the recolour changed.
#[derive(Debug)]
pub struct Report {
    /// The angle each layout was turned by, in degrees.
    pub degrees: BTreeMap<String, i32>,
    /// The images that were rewritten.
    pub changed: Vec<String>,
}

/// Turn every layout of `layouts` by its own random angle. Angles too small to
/// see are never drawn, so every ship comes back a different colour. A layout
/// without art of its own is left alone, and its angle is not reported.
pub fn recolour(
    archive: &mut FtlArchive,
    rng: &mut Rng,
    layouts: &[String],
) -> crate::Result<Report> {
    let paths: Vec<String> = archive.paths().map(str::to_string).collect();
    let mut report = Report {
        degrees: BTreeMap::new(),
        changed: Vec::new(),
    };

    for layout in layouts {
        let angle = TURN_MIN + rng.below((TURN_MAX - TURN_MIN + 1) as u64) as i32;
        let mut art: Vec<&String> = paths
            .iter()
            .filter(|path| paints(path, layout))
            .collect();
        art.sort();
        if art.is_empty() {
            continue;
        }
        for path in art {
            let bytes = archive
                .get_file(path)
                .ok_or_else(|| Error::Format(format!("{path} vanished from the archive")))?
                .to_vec();
            let turned = turn(&bytes, angle)
                .map_err(|why| Error::Format(format!("cannot recolour {path}: {why}")))?;
            if archive.set_file(path, turned) {
                report.changed.push(path.clone());
            }
        }
        report.degrees.insert(layout.clone(), angle);
    }

    Ok(report)
}

/// true when the image is the hull art or a gib of this layout. The names carry
/// the layout, so `kestral_base.png` is never taken for `kestral_2_base.png`.
fn paints(path: &str, layout: &str) -> bool {
    let name = match path
        .strip_prefix("img/ship/")
        .and_then(|name| name.strip_suffix(".png"))
    {
        Some(name) => name,
        None => return false,
    };
    let Some(tail) = name.strip_prefix(&format!("{layout}_")) else {
        return false;
    };
    match tail.strip_prefix("gib") {
        Some(number) => !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()),
        None => tail == "base",
    }
}

/// The image with its hue moved by `degrees`. Grey and black pixels have no hue,
/// and are handed back unchanged.
fn turn(bytes: &[u8], degrees: i32) -> Result<Vec<u8>, String> {
    let mut image = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| error.to_string())?
        .decode()
        .map_err(|error| error.to_string())?
        .to_rgba8();

    for pixel in image.pixels_mut() {
        let [red, green, blue, alpha] = pixel.0;
        if alpha == 0 {
            continue;
        }
        let [red, green, blue] = rotate([red, green, blue], degrees as f32);
        pixel.0 = [red, green, blue, alpha];
    }

    let mut out = Vec::new();
    // The stock art is packed tight, and `image` compresses loosely by default,
    // which would leave a mod twice the size it needs to be.
    let encoder = PngEncoder::new_with_quality(&mut out, CompressionType::Best, FilterType::Adaptive);
    image
        .write_with_encoder(encoder)
        .map_err(|error| error.to_string())?;
    Ok(out)
}

/// The hue of a colour moved along the wheel, keeping its saturation and its
/// lightness. Both halves of the conversion are the usual HSL ones.
fn rotate(rgb: [u8; 3], degrees: f32) -> [u8; 3] {
    let (red, green, blue) = (rgb[0] as f32 / 255., rgb[1] as f32 / 255., rgb[2] as f32 / 255.);
    let high = red.max(green).max(blue);
    let low = red.min(green).min(blue);
    let spread = high - low;
    if spread < 1.0 / 512.0 {
        return rgb;
    }

    let lightness = (high + low) / 2.;
    let saturation = if lightness > 0.5 {
        spread / (2. - high - low)
    } else {
        spread / (high + low)
    };
    let sector = if (high - red).abs() < f32::EPSILON {
        (green - blue) / spread
    } else if (high - green).abs() < f32::EPSILON {
        (blue - red) / spread + 2.
    } else {
        (red - green) / spread + 4.
    };
    let hue = (sector * 60. + degrees).rem_euclid(360.) / 60.;

    let top = if lightness < 0.5 {
        lightness * (1. + saturation)
    } else {
        lightness + saturation - lightness * saturation
    };
    let bottom = 2. * lightness - top;
    let channel = |offset: f32| {
        let place = (hue + offset).rem_euclid(6.);
        let value = if place < 1. {
            bottom + (top - bottom) * place
        } else if place < 3. {
            top
        } else if place < 4. {
            bottom + (top - bottom) * (4. - place)
        } else {
            bottom
        };
        (value.clamp(0., 1.) * 255.).round() as u8
    };
    [channel(2.), channel(0.), channel(-2.)]
}