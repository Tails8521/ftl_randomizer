//! Time the parts of a run, to see what is worth making faster.
//!
//! `cargo run --release --bin probe_profile -- [SEED] [--runs N]`
//!
//! It walks the same passes as `pipeline::process`, with a clock around each, then
//! weighs the image work those passes do one image at a time: decode, hue turn and
//! png encode, at each of the compression settings `ship_art::encode` could be
//! given. It changes nothing and writes nothing.
//!
//! The run encodes at `CompressionType::Level(3)`, which measured 3 times faster
//! than the level 9 it replaced, for 9% more bytes. The other settings are here to
//! show what else was on the table.

use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{DynamicImage, ImageReader, RgbaImage};

use ftl_randomizer::ftl_mod::{FtlMod, Meta};
use ftl_randomizer::rng::Rng;
use ftl_randomizer::{FtlArchive, loadout, rooms, ship_art};

/// Named laps, in the order they were taken.
struct Laps(Vec<(String, Duration)>);

impl Laps {
    fn new() -> Self {
        Laps(Vec::new())
    }

    fn lap(&mut self, what: &str, since: Instant) {
        self.0.push((what.to_string(), since.elapsed()));
    }

    /// The laps as lines, `total` being the share they are measured against.
    fn print(&self, total: Duration) {
        for (what, took) in &self.0 {
            println!(
                "  {:>9.1} ms  {:>5.1}%  {what}",
                took.as_secs_f64() * 1000.0,
                took.as_secs_f64() / total.as_secs_f64() * 100.0
            );
        }
    }
}

/// The compression settings an encode can be given, and what they are called. The
/// first is what `ship_art::encode` uses.
const SETTINGS: [(&str, CompressionType, FilterType); 6] = [
    ("level(3)/adaptive", CompressionType::Level(3), FilterType::Adaptive),
    ("best/adaptive", CompressionType::Best, FilterType::Adaptive),
    ("level(9)/adaptive", CompressionType::Level(9), FilterType::Adaptive),
    ("balanced/adaptive", CompressionType::Default, FilterType::Adaptive),
    ("fast/adaptive", CompressionType::Fast, FilterType::Adaptive),
    ("fast/up", CompressionType::Fast, FilterType::Up),
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut seed = 1234;
    let mut runs = 1;
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut at = 0;
    while at < argv.len() {
        match argv[at].as_str() {
            "--runs" => {
                at += 1;
                runs = argv[at].parse()?;
            }
            other => seed = other.parse()?,
        }
        at += 1;
    }

    let mut laps = Laps::new();
    let mut whole = Vec::new();
    let mut layouts = Vec::new();
    for _ in 0..runs {
        whole.push(one(seed, &mut laps, &mut layouts)?);
    }

    let total: Duration = whole.iter().sum();
    println!("\npasses, {} run(s) of seed {seed}", whole.len());
    laps.print(total);
    println!("  {:>9.1} ms  100.0%  TOTAL", total.as_secs_f64() * 1000.0);
    if runs > 1 {
        println!(
            "  fastest {:?}, slowest {:?}",
            whole.iter().min().unwrap(),
            whole.iter().max().unwrap()
        );
    }

    image_work(&layouts)
}

/// One run of the whole randomizer, with a clock around every pass.
fn one(seed: u64, laps: &mut Laps, layouts: &mut Vec<String>) -> Result<Duration, Box<dyn std::error::Error>> {
    let start = Instant::now();

    let t = Instant::now();
    let raw = std::fs::read("data/ftl.dat")?;
    laps.lap("read data/ftl.dat", t);

    let t = Instant::now();
    let mut archive = FtlArchive::from_bytes(&raw)?;
    laps.lap("FtlArchive::from_bytes", t);

    // The tag scan that both `rooms::randomize` and `loadout::randomize` walk the
    // whole of `data/*.xml` for, once each.
    let t = Instant::now();
    let xml: Vec<String> = archive
        .paths()
        .filter(|path| path.starts_with("data/") && path.ends_with(".xml"))
        .map(str::to_string)
        .collect();
    let xml_bytes: usize = xml.iter().map(|path| archive.get_file(path).unwrap().len()).sum();
    let mut found = 0;
    for path in &xml {
        let bytes = archive.get_file(path).unwrap();
        for needle in [
            b"<shipBlueprint".as_slice(),
            b"<weaponBlueprint",
            b"<droneBlueprint",
            b"<augBlueprint",
        ] {
            if bytes.windows(needle.len()).any(|window| window == needle) {
                found += 1;
            }
        }
    }
    let xml_kib = xml_bytes / 1024;
    laps.lap(&format!(
        "{xml_kib} KiB of data/*.xml, 4 tag scans ({found} hits)"
    ), t);

    let mut rng = Rng::new(seed);

    let t = Instant::now();
    let drawn = rooms::randomize(&mut archive, &mut rng)?;
    laps.lap("rooms::randomize (layout + deck art)", t);

    let t = Instant::now();
    let gear = loadout::randomize(&mut archive, &mut rng)?;
    laps.lap("loadout::randomize", t);

    *layouts = drawn.ships.iter().map(|ship| ship.layout.clone()).collect();
    let t = Instant::now();
    let art = ship_art::recolor(&mut archive, &mut rng, layouts)?;
    laps.lap("ship_art::recolor (hull + gib art)", t);

    let mut changed: BTreeSet<String> = drawn.changed.iter().cloned().collect();
    changed.extend(gear.changed.iter().cloned());
    changed.extend(art.changed.iter().cloned());

    let t = Instant::now();
    let mut module = FtlMod::new();
    for path in &changed {
        module.add(path, archive.get_file(path).unwrap());
    }
    module.describe(&Meta {
        title: "profile",
        thread_url: "",
        author: "",
        version: "",
        description: "",
    });
    let ftl = module.to_bytes();
    laps.lap("pack the .ftl mod (copy + crc32)", t);

    let t = Instant::now();
    let packed = archive.to_bytes()?;
    laps.lap("FtlArchive::to_bytes (rebuild 267 MiB)", t);

    println!(
        "\n  ({} files, {} changed, mod {} KiB, archive {} MiB)",
        archive.len(),
        changed.len(),
        ftl.len() / 1024,
        packed.len() / 1024 / 1024
    );

    Ok(start.elapsed())
}

/// What an image costs to move through the codec, and what it weighs after.
struct Cost {
    count: usize,
    stock: usize,
    pixels: u64,
    decode: Duration,
    turn: Duration,
    /// Colors with a pixel of them showing.
    colors: usize,
    /// One encode per entry of `SETTINGS`.
    encode: [Duration; SETTINGS.len()],
    /// Encoded size per entry of `SETTINGS`.
    bytes: [usize; SETTINGS.len()],
}

impl Cost {
    fn new() -> Self {
        Cost {
            count: 0,
            stock: 0,
            pixels: 0,
            decode: Duration::ZERO,
            turn: Duration::ZERO,
            colors: 0,
            encode: [Duration::ZERO; SETTINGS.len()],
            bytes: [0; SETTINGS.len()],
        }
    }

    fn add(&mut self, other: &Cost) {
        self.count += other.count;
        self.stock += other.stock;
        self.pixels += other.pixels;
        self.decode += other.decode;
        self.turn += other.turn;
        self.colors += other.colors;
        for (mine, theirs) in self.encode.iter_mut().zip(other.encode) {
            *mine += theirs;
        }
        for (mine, theirs) in self.bytes.iter_mut().zip(other.bytes) {
            *mine += theirs;
        }
    }

    fn line(&self, what: &str) {
        println!(
            "\n  {what}: {} images, {} px, {:.0} KiB in the archive",
            self.count,
            self.pixels,
            self.stock / 1024
        );
        println!(
            "    decode {:>7.1} ms   hue turn {:>7.1} ms   {} colors between them",
            self.decode.as_secs_f64() * 1000.0,
            self.turn.as_secs_f64() * 1000.0,
            self.colors
        );
        for ((name, _, _), (took, bytes)) in SETTINGS
            .iter()
            .zip(self.encode.iter().zip(self.bytes.iter()))
        {
            println!(
                "    encode {name:<18}{:>8.1} ms  -> {:>7.0} KiB",
                took.as_secs_f64() * 1000.0,
                bytes / 1024
            );
        }
    }
}

/// Weigh the image work of the passes one image at a time.
///
/// The images of a run are the hull art and the gibs `ship_art::recolor` turns, and
/// the deck art `rooms::randomize` draws. The stock bytes of a deck are what its
/// replacement is measured against: the new art is a flat shape and should compress
/// no worse than the art it replaces.
fn image_work(layouts: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let raw = std::fs::read("data/ftl.dat")?;
    let archive = FtlArchive::from_bytes(&raw)?;

    println!("\nimage work, per image class");
    let mut hull = Cost::new();
    let mut gib = Cost::new();
    let mut deck = Cost::new();
    for layout in layouts {
        for path in art_of(&archive, layout) {
            let stock = archive.get_file(&path).unwrap();
            let is_gib = path.contains("_gib");
            let cost = weigh(stock)?;
            if is_gib {
                gib.add(&cost);
            } else {
                hull.add(&cost);
            }
        }
        // The deck of a layout may be absent (the C hulls borrow the A deck),
        // in which case there is nothing to weigh.
        let floor = format!("img/ship/{layout}_floor.png");
        if let Some(stock) = archive.get_file(&floor) {
            deck.add(&weigh_stock(stock));
        }
    }
    // How the same turn does with one table kept over the art of a whole ship,
    // which is turned by one angle: the gibs are cut out of the hull, so their
    // colors are mostly colors the hull already paid for.
    let mut shared = Shared::new();
    for layout in layouts {
        let mut art: Vec<RgbaImage> = Vec::new();
        for path in art_of(&archive, layout) {
            art.push(decode(archive.get_file(&path).unwrap())?);
        }
        shared.add_layout(&art);
    }
    shared.print();
    println!();
    hull.line("hull art (base)");
    gib.line("gib art");
    deck.line("deck art (encode only, no hue turn)");

    println!("\nwhat the whole run costs at each setting (hue turn + encode)");
    let turn = hull.turn + gib.turn + deck.turn;
    for (index, (name, _, _)) in SETTINGS.iter().enumerate() {
        let encode = hull.encode[index] + gib.encode[index] + deck.encode[index];
        println!(
            "  {name:<18}{:>8.0} ms encode + {:>5.0} ms turn = {:>6.0} ms   mod art {:>6.0} KiB",
            encode.as_secs_f64() * 1000.0,
            turn.as_secs_f64() * 1000.0,
            (encode + turn).as_secs_f64() * 1000.0,
            (hull.bytes[index] + gib.bytes[index] + deck.bytes[index]) / 1024
        );
    }
    Ok(())
}

/// The art `ship_art::recolor` turns for one layout: the hull and its gibs.
fn art_of(archive: &FtlArchive, layout: &str) -> Vec<String> {
    let mut art: Vec<String> = archive
        .paths()
        .filter(|path| {
            let Some(name) = path
                .strip_prefix("img/ship/")
                .and_then(|name| name.strip_suffix(".png"))
            else {
                return false;
            };
            let Some(tail) = name.strip_prefix(&format!("{layout}_")) else {
                return false;
            };
            match tail.strip_prefix("gib") {
                Some(number) => !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()),
                None => tail == "base",
            }
        })
        .map(str::to_string)
        .collect();
    art.sort();
    art
}

/// Decode, turn and encode one image at every setting.
fn weigh(stock: &[u8]) -> Result<Cost, Box<dyn std::error::Error>> {
    let mut cost = Cost::new();
    let t = Instant::now();
    let image = decode(stock)?;
    cost.decode = t.elapsed();
    cost.count = 1;
    cost.stock = stock.len();
    cost.pixels = (image.width() as u64) * (image.height() as u64);

    let t = Instant::now();
    let turned = turn(&image, 137.0);
    cost.turn = t.elapsed();
    cost.colors = colors(&image);

    for (index, (_, compression, filter)) in SETTINGS.iter().enumerate() {
        let t = Instant::now();
        let out = encode(&turned, *compression, *filter);
        cost.encode[index] = t.elapsed();
        cost.bytes[index] = out.len();
    }
    Ok(cost)
}

/// The deck art, which is drawn rather than turned: only its size is known ahead
/// of the run, so its stock bytes stand in for the new ones.
fn weigh_stock(stock: &[u8]) -> Cost {
    let mut cost = match weigh(stock) {
        Ok(cost) => cost,
        Err(_) => return Cost::new(),
    };
    // The hue turn of a deck is not something the randomizer does.
    cost.turn = Duration::ZERO;
    cost
}

/// The hue turn of an image through whatever `turned` hands back for a color:
/// a table of colors already converted, or the plain conversion.
fn turn_via(image: &RgbaImage, mut turned: impl FnMut([u8; 3]) -> [u8; 3]) -> RgbaImage {
    let mut out = image.clone();
    for pixel in out.pixels_mut() {
        let [red, green, blue, alpha] = pixel.0;
        if alpha == 0 {
            continue;
        }
        let rgb = turned([red, green, blue]);
        pixel.0 = [rgb[0], rgb[1], rgb[2], alpha];
    }
    out
}

/// A color as one number, which is the key a map costs least to hash.
fn pack(rgb: [u8; 3]) -> u32 {
    (u32::from(rgb[0]) << 16) | (u32::from(rgb[1]) << 8) | u32::from(rgb[2])
}

/// A hash of a number that is already spread over its bits: one multiply and a
/// shift. `std`'s default hashes with SipHash, which costs more than the whole
/// color conversion it saves.
#[derive(Default)]
struct Spread(u64);

impl std::hash::Hasher for Spread {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
    }

    fn write_u32(&mut self, number: u32) {
        self.0 = (u64::from(number).wrapping_mul(0x9E37_79B9_7F4A_7C15)) >> 23;
    }
}

/// A color table over a packed key and the cheap hash above.
type Fast = HashMap<u32, [u8; 3], std::hash::BuildHasherDefault<Spread>>;

/// What the art of one ship costs to turn, by scope of the table and by key.
struct Shared {
    pixels: u64,
    images: usize,
    /// The loop of the library: one conversion per pixel.
    plain: Duration,
    /// One table per image, per ship; and within those, the three keys.
    image_rgb: Duration,
    image_u32: Duration,
    image_fast: Duration,
    ship_rgb: Duration,
    ship_u32: Duration,
    ship_fast: Duration,
    /// Colors a ship holds between its images, against the same count added up
    /// image by image: the gap is what a shared table never converts twice.
    union: usize,
    sum: usize,
}

impl Shared {
    fn new() -> Self {
        Shared {
            pixels: 0,
            images: 0,
            plain: Duration::ZERO,
            image_rgb: Duration::ZERO,
            image_u32: Duration::ZERO,
            image_fast: Duration::ZERO,
            ship_rgb: Duration::ZERO,
            ship_u32: Duration::ZERO,
            ship_fast: Duration::ZERO,
            union: 0,
            sum: 0,
        }
    }

    fn add_layout(&mut self, art: &[RgbaImage]) {
        // One angle per ship, as `ship_art::recolor` gives it one angle, which is
        // what makes a table shared over a ship's art sound.
        let degrees = 137.0;
        let plain: Vec<RgbaImage> = art.iter().map(|image| turn(image, degrees)).collect();

        let mut union: HashMap<[u8; 3], [u8; 3]> = Default::default();
        let mut union_fast: HashMap<u32, [u8; 3]> = Default::default();
        let mut union_spread: Fast = Default::default();

        let ship = Instant::now();
        let by_ship_rgb: Vec<RgbaImage> = art
            .iter()
            .map(|image| {
                turn_via(image, |rgb| {
                    *union
                        .entry(rgb)
                        .or_insert_with(|| rotate(rgb, degrees))
                })
            })
            .collect();
        let ship_rgb = ship.elapsed();

        let ship = Instant::now();
        let by_ship_u32: Vec<RgbaImage> = art
            .iter()
            .map(|image| {
                turn_via(image, |rgb| {
                    *union_fast
                        .entry(pack(rgb))
                        .or_insert_with(|| rotate(rgb, degrees))
                })
            })
            .collect();
        let ship_u32 = ship.elapsed();

        let ship = Instant::now();
        let by_ship_fast: Vec<RgbaImage> = art
            .iter()
            .map(|image| {
                turn_via(image, |rgb| {
                    *union_spread
                        .entry(pack(rgb))
                        .or_insert_with(|| rotate(rgb, degrees))
                })
            })
            .collect();
        let ship_fast = ship.elapsed();

        let mut image_rgb = Duration::ZERO;
        let mut image_u32 = Duration::ZERO;
        let mut image_fast = Duration::ZERO;
        let mut plain_time = Duration::ZERO;
        for (image, want) in art.iter().zip(&plain) {
            let t = Instant::now();
            let made = turn(image, degrees);
            plain_time += t.elapsed();
            assert_eq!(*want, made);

            let t = Instant::now();
            let mut table: HashMap<[u8; 3], [u8; 3]> = Default::default();
            let made = turn_via(image, |rgb| {
                *table.entry(rgb).or_insert_with(|| rotate(rgb, degrees))
            });
            image_rgb += t.elapsed();
            assert_eq!(*want, made, "a table must draw the pixels the loop draws");

            let t = Instant::now();
            let mut table: HashMap<u32, [u8; 3]> = Default::default();
            let made = turn_via(image, |rgb| {
                *table.entry(pack(rgb)).or_insert_with(|| rotate(rgb, degrees))
            });
            image_u32 += t.elapsed();
            assert_eq!(*want, made);

            let t = Instant::now();
            let mut table: Fast = Default::default();
            let made = turn_via(image, |rgb| {
                *table.entry(pack(rgb)).or_insert_with(|| rotate(rgb, degrees))
            });
            image_fast += t.elapsed();
            assert_eq!(*want, made, "the packed key must land on the same pixels");
        }
        assert_eq!(plain, by_ship_rgb, "a shared table must draw the same pixels");
        assert_eq!(plain, by_ship_u32);
        assert_eq!(plain, by_ship_fast);

        self.pixels += art.iter().map(|i| (i.width() as u64) * (i.height() as u64)).sum::<u64>();
        self.images += art.len();
        self.plain += plain_time;
        self.image_rgb += image_rgb;
        self.image_u32 += image_u32;
        self.image_fast += image_fast;
        self.ship_rgb += ship_rgb;
        self.ship_u32 += ship_u32;
        self.ship_fast += ship_fast;
        self.union += union_spread.len();
        self.sum += art.iter().map(|image| colors(image)).sum::<usize>();
    }

    fn print(&self) {
        println!(
            "\n  the hue turn of the art of a ship: {} images, {} px",
            self.images, self.pixels
        );
        println!(
            "    one conversion per pixel                {:>7.1} ms",
            self.plain.as_secs_f64() * 1000.0
        );
        println!(
            "    a table per image, [u8;3] key           {:>7.1} ms  ({} colors converted)",
            self.image_rgb.as_secs_f64() * 1000.0,
            self.sum
        );
        println!(
            "    a table per image, packed key           {:>7.1} ms",
            self.image_u32.as_secs_f64() * 1000.0
        );
        println!(
            "    a table per image, packed + cheap hash  {:>7.1} ms",
            self.image_fast.as_secs_f64() * 1000.0
        );
        println!(
            "    a table per ship,   [u8;3] key          {:>7.1} ms  ({} colors converted, {} counted image by image)",
            self.ship_rgb.as_secs_f64() * 1000.0,
            self.union,
            self.sum
        );
        println!(
            "    a table per ship,   packed key          {:>7.1} ms",
            self.ship_u32.as_secs_f64() * 1000.0
        );
        println!(
            "    a table per ship,   packed + cheap hash {:>7.1} ms",
            self.ship_fast.as_secs_f64() * 1000.0
        );
    }
}

/// Colors with a pixel of them showing.
fn colors(image: &RgbaImage) -> usize {
    let mut seen: std::collections::BTreeSet<[u8; 3]> = Default::default();
    for pixel in image.pixels() {
        if pixel[3] != 0 {
            seen.insert([pixel[0], pixel[1], pixel[2]]);
        }
    }
    seen.len()
}

fn decode(bytes: &[u8]) -> Result<RgbaImage, Box<dyn std::error::Error>> {
    let image = ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()?
        .decode()?;
    Ok(image.to_rgba8())
}

fn turn(image: &RgbaImage, degrees: f32) -> RgbaImage {
    let mut out = image.clone();
    for pixel in out.pixels_mut() {
        let [red, green, blue, alpha] = pixel.0;
        if alpha == 0 {
            continue;
        }
        let [red, green, blue] = rotate([red, green, blue], degrees);
        pixel.0 = [red, green, blue, alpha];
    }
    out
}

fn encode(image: &RgbaImage, compression: CompressionType, filter: FilterType) -> Vec<u8> {
    let mut out = Vec::new();
    let encoder = PngEncoder::new_with_quality(&mut out, compression, filter);
    DynamicImage::ImageRgba8(image.clone())
        .write_with_encoder(encoder)
        .expect("encode failed");
    out
}

/// A copy of the hue math of `ship_art::rotate`, which is private.
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