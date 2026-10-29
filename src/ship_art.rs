//! Turn the color of the player ships.
//!
//! Each layout gets one angle, and every image that carries its name - the hull
//! art, the six gibs and the small drawing of the ship select screen - is turned
//! by that angle, so a ship, the wreck it breaks into and its icon in the
//! customization menu all stay the same color. Only the hue moves: saturation
//! and lightness are left alone, so a ship keeps its shading and its dark
//! plating. Angles too small to notice are not drawn.
//!
//! Like the rest of the library this works on an archive in memory.

use std::collections::{BTreeMap, HashMap};
use std::hash::{BuildHasherDefault, Hasher};
use std::io::Cursor;

use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ImageReader, RgbaImage};

use crate::rng::Rng;
use crate::{Error, FtlArchive};

/// The angles a ship may turn by. Below one, or above the other, the ship looks
/// exactly as the game shipped it, which is not what a randomiser is for. The enemy
/// ships are turned by an angle of the same span, see `crate::enemy_art`.
pub(crate) const TURN_MIN: i32 = 21;
pub(crate) const TURN_MAX: i32 = 339;

/// What the recolor changed.
#[derive(Debug)]
pub struct Report {
    /// The angle each layout was turned by, in degrees.
    pub degrees: BTreeMap<String, i32>,
    /// The images that were rewritten.
    pub changed: Vec<String>,
}

/// Turn every layout of `layouts` by its own random angle. Angles too small to
/// see are never drawn, so every ship comes back a different color. A layout
/// without art of its own is left alone, and its angle is not reported.
pub fn recolor(
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
        // One angle turns the hull, all of its gibs and its miniship, and the
        // gibs are cut out of the same art, so the colors they hold are worked
        // out once between them.
        let mut hues: Turned = Turned::default();
        for path in art {
            let bytes = archive
                .get_file(path)
                .ok_or_else(|| Error::Format(format!("{path} vanished from the archive")))?
                .to_vec();
            let turned = turn(&bytes, angle, &mut hues)
                .map_err(|why| Error::Format(format!("cannot recolor {path}: {why}")))?;
            if archive.set_file(path, turned) {
                report.changed.push(path.clone());
            }
        }
        report.degrees.insert(layout.clone(), angle);
    }

    Ok(report)
}

/// true when the image is painted in the colors of this layout: the hull art, one
/// of its gibs, or the `miniship_` drawing of the ship select screen. The names
/// carry the layout, so `kestral_base.png` is never taken for
/// `kestral_2_base.png`, and `miniship_kestral.png` never for
/// `miniship_kestral_2.png`.
fn paints(path: &str, layout: &str) -> bool {
    if let Some(name) = path
        .strip_prefix("img/customizeUI/miniship_")
        .and_then(|name| name.strip_suffix(".png"))
    {
        return name == layout;
    }
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

/// The colors of one ship's art, each with the color it turns to, by a key of the
/// red, green and blue packed into one number. One per ship, shared between all of
/// its files, which `crate::enemy_art` does the same way.
///
/// [`rotate`] answers the same thing every time for one color and one angle, and
/// the art is nearly a palette - about 2000 colors to a hull of 260 000 pixels -
/// so a table takes the conversion off 20 million pixels and puts it on 60 000
/// colors. A pixel of a color the table has never seen gets exactly the conversion
/// it got before, so no pixel moves; the map is looked up and filled but never
/// walked, so its order cannot reach a pixel either. Measured by
/// `src/bin/probe_profile.rs`, which lands on the same images either way.
pub(crate) type Turned = HashMap<u32, [u8; 3], BuildHasherDefault<Spread>>;

/// A hash of a number that already spreads over its bits: one multiply, and the
/// top half is the answer. `std` hashes with SipHash, which costs more on this
/// path than the color conversion a miss pays for.
#[derive(Default)]
pub(crate) struct Spread(u64);

impl Hasher for Spread {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
    }

    fn write_u32(&mut self, number: u32) {
        self.0 = (u64::from(number).wrapping_mul(0x9E37_79B9_7F4A_7C15)) >> 32;
    }
}

/// The image with its hue moved by `degrees`. Grey and black pixels have no hue,
/// and are handed back unchanged.
pub(crate) fn turn(bytes: &[u8], degrees: i32, hues: &mut Turned) -> Result<Vec<u8>, String> {
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
        let key = (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue);
        let [red, green, blue] = *hues
            .entry(key)
            .or_insert_with(|| rotate([red, green, blue], degrees as f32));
        pixel.0 = [red, green, blue, alpha];
    }

    encode(&image)
}

/// The image as a PNG.
///
/// Level 3 is the balance the whole run wants: the 196 recolored images and the
/// 20 deck images are what a run spends its time on, and the encoder is most of
/// that. Against level 9, level 3 draws the same pixels three times faster for 9%
/// more bytes; level 6 sits between them and level 1 saves no time over the
/// fastest setting here. Measured by `src/bin/probe_profile.rs`.
pub fn encode(image: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let encoder = PngEncoder::new_with_quality(&mut out, CompressionType::Level(3), FilterType::Adaptive);
    image
        .write_with_encoder(encoder)
        .map_err(|error| error.to_string())?;
    Ok(out)
}

/// The hue of a color moved along the wheel, keeping its saturation and its
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