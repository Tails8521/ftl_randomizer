//! Probe: the deck art the randomizer draws against the art the game ships.
//!
//! The rooms and doors of a layout are read as the game ships them - nothing is
//! randomized - so the drawn deck and the shipped deck describe the same rooms
//! and the same airlocks, and what differs is the rule, not the layout.
//!
//! Three files per layout land in `temp/floors`: the shipped art, the drawn art
//! and their difference, where red is a pixel one of them paints and the other
//! does not and orange a pixel both paint in a different color. A layout whose
//! ship ships no deck art cannot be compared and is only drawn.
//!
//! Run `cargo run --release --bin probe_floor [LAYOUT ...] [SEED]`.

use std::collections::BTreeSet;
use std::path::Path;

use ftl_randomizer::floor::{self, Deck};
use ftl_randomizer::layout::{Room, Side};
use ftl_randomizer::ship_txt::ShipTxt;
use ftl_randomizer::{rooms, FtlArchive};
use image::{ImageReader, Rgba, RgbaImage};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let bytes = match std::fs::read("data/ftl.dat") {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("data/ftl.dat: {error}");
            std::process::exit(1);
        }
    };
    let archive = match FtlArchive::from_bytes(&bytes) {
        Ok(archive) => archive,
        Err(error) => {
            eprintln!("data/ftl.dat: {error}");
            std::process::exit(1);
        }
    };

    let wanted: Vec<String> = if args.is_empty() {
        archive
            .paths()
            .filter_map(|path| path.strip_prefix("img/ship/")?.strip_suffix("_floor.png"))
            .map(str::to_string)
            .collect()
    } else {
        args.clone()
    };

    std::fs::create_dir_all("temp/floors").ok();
    println!(
        "{:<22}{:>8}{:>9}{:>9}{:>9}{:>9}{:>8}{:>8}  airlock walls",
        "layout", "rooms", "shipped", "drawn", "both", "iou", "diff", "paint"
    );
    for layout in wanted {
        match compare(&archive, &layout) {
            Ok(line) => println!("{line}"),
            Err(why) => println!("{layout}: {why}"),
        }
    }
}

/// Draw one layout and compare. The returned line is what the table shows.
fn compare(archive: &FtlArchive, layout: &str) -> Result<String, String> {
    let txt = archive
        .get_file(&format!("data/{layout}.txt"))
        .ok_or("no layout file")?;
    let txt = ShipTxt::parse(txt.to_vec())?;
    let xml = archive
        .get_file(&format!("data/{layout}.xml"))
        .ok_or("no layout xml")?;
    let at = rooms::deck_offset(xml)?;
    let path = format!("img/ship/{layout}_floor.png");
    let shipped = archive
        .get_file(&path)
        .ok_or("the ship ships no deck art, so there is nothing to compare")?;
    let deck = Deck::shipped(shipped, at)?;
    let made = floor::draw(&txt.rooms, &txt.doors, &deck)?;

    let drawn = decode(&made.png)?;
    let old = decode(shipped)?;
    let both = count(&drawn, &old, |a, b| a && b);
    let either = count(&drawn, &old, |a, b| a || b);
    let iou = both as f64 / either as f64;

    write("temp/floors", layout, "shipped", &old)?;
    write("temp/floors", layout, "drawn", &drawn)?;
    write("temp/floors", layout, "diff", &difference(&old, &drawn))?;

    let (wrong, _) = mispainted(&old, &drawn);
    let locks = walls(&txt.rooms, &txt.doors);
    Ok(format!(
        "{:<22}{:>8}{:>9}{:>9}{:>9}{:>9.4}{:>8}{:>8}  {}",
        layout,
        txt.rooms.len(),
        count(&old, &old, |a, _b| a),
        count(&drawn, &drawn, |a, _b| a),
        both,
        iou,
        either - both,
        wrong,
        locks
    ))
}

/// The pixels both arts paint, and how many of them one calls edge and the other
/// deck. The count is what the `paint` column shows.
fn mispainted(old: &RgbaImage, drawn: &RgbaImage) -> (u64, u64) {
    let mut wrong = 0;
    let mut both = 0;
    for y in 0..old.height().min(drawn.height()) {
        for x in 0..old.width().min(drawn.width()) {
            if !painted(old, x, y) || !painted(drawn, x, y) {
                continue;
            }
            both += 1;
            if dark(old, x, y) != dark(drawn, x, y) {
                wrong += 1;
            }
        }
    }
    (wrong, both)
}

/// true where the art paints the dark color of the two it is made of.
fn dark(image: &RgbaImage, x: u32, y: u32) -> bool {
    let pixel = image.get_pixel(x, y);
    let sum = pixel[0] as u32 + pixel[1] as u32 + pixel[2] as u32;
    sum < 120
}

/// How the airlock walls of the shipped layout fall out, as a tally of sides.
fn walls(rooms: &[Room], doors: &[ftl_randomizer::layout::Door]) -> String {
    let mut tally = [("up", 0), ("down", 0), ("left", 0), ("right", 0)];
    let mut seen: BTreeSet<(i32, Side)> = BTreeSet::new();
    for door in doors.iter().filter(|door| door.airlock()) {
        let id = if door.left_top == -1 {
            door.right_bottom
        } else {
            door.left_top
        };
        let Ok(index) = usize::try_from(id) else {
            continue;
        };
        let Some(&room) = rooms.get(index) else {
            continue;
        };
        let (low, high) = door.tiles();
        let side = if touches(room, high) {
            if door.vertical { Side::Left } else { Side::Up }
        } else if touches(room, low) {
            if door.vertical { Side::Right } else { Side::Down }
        } else {
            continue;
        };
        if !seen.insert((id, side)) {
            continue;
        }
        let name = match side {
            Side::Up => 0,
            Side::Down => 1,
            Side::Left => 2,
            Side::Right => 3,
        };
        tally[name].1 += 1;
    }
    let mut out = String::new();
    for (name, count) in tally {
        out.push_str(&format!("{name} {count} "));
    }
    out
}

fn touches(room: Room, tile: (i32, i32)) -> bool {
    tile.0 >= room.x && tile.0 < room.x + room.w && tile.1 >= room.y && tile.1 < room.y + room.h
}

fn decode(bytes: &[u8]) -> Result<RgbaImage, String> {
    ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| error.to_string())?
        .decode()
        .map(|image| image.to_rgba8())
        .map_err(|error| error.to_string())
}

/// Pixels where the two images agree under `test`, deck painted counted as true.
fn count(one: &RgbaImage, two: &RgbaImage, test: impl Fn(bool, bool) -> bool) -> u64 {
    let mut out = 0;
    for y in 0..one.height().max(two.height()) {
        for x in 0..one.width().max(two.width()) {
            if test(painted(one, x, y), painted(two, x, y)) {
                out += 1;
            }
        }
    }
    out
}

/// true where the image has deck painted.
fn painted(image: &RgbaImage, x: u32, y: u32) -> bool {
    x < image.width() && y < image.height() && image.get_pixel(x, y)[3] > 127
}

fn difference(old: &RgbaImage, drawn: &RgbaImage) -> RgbaImage {
    let width = old.width().max(drawn.width());
    let height = old.height().max(drawn.height());
    let mut out = RgbaImage::from_pixel(width, height, Rgba([16, 16, 22, 255]));
    for y in 0..height {
        for x in 0..width {
            let a = painted(old, x, y);
            let b = painted(drawn, x, y);
            if a == b {
                if a && dark(old, x, y) != dark(drawn, x, y) {
                    // The shape agrees and the two colors do not: orange.
                    out.put_pixel(x, y, Rgba([240, 170, 60, 255]));
                } else {
                    let grey = if a { [150, 150, 160, 255] } else { [16, 16, 22, 255] };
                    out.put_pixel(x, y, Rgba(grey));
                }
            } else {
                out.put_pixel(x, y, Rgba([220, 60, 60, 255]));
            }
        }
    }
    out
}

fn write(dir: &str, layout: &str, kind: &str, image: &RgbaImage) -> Result<(), String> {
    let path = Path::new(dir).join(format!("{layout}_{kind}.png"));
    image.save(&path).map_err(|error| error.to_string())
}