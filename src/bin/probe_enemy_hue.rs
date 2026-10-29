//! Checks the colours `enemy_art` gives the enemy ships of an archive.
//!
//! The pass turns an image by moving its hue and nothing else, so the checks are on
//! what an image is allowed to keep and what it has to give up:
//!
//! * every pixel keeps its alpha, so a ship neither gains nor loses paint;
//! * a pixel with no hue - grey, black, white - keeps its color, and a pixel with one
//!   moves;
//! * the ten files of a ship - the hull, its four gibs and the five of the glow - all
//! * move by the same angle, which is checked by the color one of them shares with
//!   another turning to the same color in both;
//! * a ship drawn from a copied art has all ten files of the copy and its cloak
//!   sprite, and the copy is turned rather than pasted;
//! * every `img=` of an enemy blueprint names an art the archive can draw;
//! * and nothing of a player hull is touched, which `ship_art.rs` owns.
//!
//! ```text
//! cargo run --release --bin probe_enemy_hue -- [SEED]
//! ```
//!
//! Probes the archive and writes nothing.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;

use ftl_randomizer::blueprints;
use ftl_randomizer::enemy_art;
use ftl_randomizer::rng::Rng;
use ftl_randomizer::FtlArchive;
use image::RgbaImage;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let seed = std::env::args()
        .nth(1)
        .map(|word| word.parse::<u64>())
        .transpose()?
        .unwrap_or(1);

    let mut raw = Vec::new();
    std::fs::File::open("data/ftl.dat")?.read_to_end(&mut raw)?;
    let stock = FtlArchive::from_bytes(&raw)?;

    // What the archive holds before the pass is let near it.
    let hulls = player_hulls(&stock);
    let before = images(&stock);

    let mut archive = FtlArchive::from_bytes(&raw)?;
    let turned = enemy_art::recolor(&mut archive, &mut Rng::new(seed))?;

    println!("seed {seed}");
    println!(
        "{:<22}{:<24}{:>6}{:>6}  blueprints",
        "layout", "art", "angle", "files"
    );
    for ship in &turned.ships {
        println!(
            "{:<22}{:<24}{:>5}°{:>6}  {}",
            ship.layout,
            if ship.into == ship.art {
                ship.art.clone()
            } else {
                format!("{} -> {}", ship.art, ship.into)
            },
            ship.degrees,
            ship.files,
            ship.blueprints.join(", "),
        );
    }

    let mut faults: Vec<String> = Vec::new();
    // Every image a ship is drawn from, by the file it is held in, so a picture two
    // ships were turned at two angles is caught rather than overwritten.
    let mut claimed: BTreeMap<String, (String, i32)> = BTreeMap::new();
    for ship in &turned.ships {
        let angle = ship.degrees;
        let mut shared: BTreeMap<(u8, u8, u8), (u8, u8, u8)> = BTreeMap::new();
        for (from, into) in enemy_art::pairs(&ship.art, &ship.into) {
            let (Some(stock_bytes), Some(now_bytes)) = (stock.get_file(&from), archive.get_file(&into)) else {
                faults.push(format!("{} lost {into}, the file of its art", ship.layout));
                continue;
            };
            match claimed.insert(into.clone(), (ship.layout.clone(), angle)) {
                Some((other, other_angle)) if other_angle != angle => {
                    faults.push(format!(
                        "{into}: {other} was turned there at {other_angle}° and {} at {angle}°",
                        ship.layout
                    ));
                }
                _ => {}
            }
            let (was, now) = (decode(stock_bytes, &from)?, decode(now_bytes, &into)?);
            if was.dimensions() != now.dimensions() {
                faults.push(format!("{into}: it changed size"));
                continue;
            }
            if ship.into != ship.art && was == now {
                faults.push(format!(
                    "{}: {into} is the art it was copied from, pixel for pixel",
                    ship.layout
                ));
            }
            let (mut painted, mut moved) = (0usize, 0usize);
            for (was, now) in was.pixels().zip(now.pixels()) {
                let (color, alpha) = (rgb(&was.0), was.0[3]);
                let (turned, alpha_now) = (rgb(&now.0), now.0[3]);
                if alpha != alpha_now {
                    faults.push(format!("{into}: a pixel changed its alpha"));
                    break;
                }
                if alpha == 0 {
                    continue;
                }
                painted += 1;
                moved += usize::from(color != turned);
                // A color of one value in all three channels has no hue to move, and
                // has to come back as it went in. A color with a hue of its own
                // usually moves, but a spread of a level or two can round back onto
                // itself, so that half is only counted, not required.
                if hueless(color) && color != turned {
                    faults.push(format!("{into}: the grey {} went to {}", hex(color), hex(turned)));
                    break;
                }
                // The ten files of one ship share one color table, so a color they
                // have in common has to have turned to one answer.
                match shared.insert(color, turned) {
                    Some(known) if known != turned => {
                        faults.push(format!("{into}: {} turns two ways at {angle}°", hex(color)));
                        break;
                    }
                    _ => {}
                }
            }
            if painted > 0 && moved == 0 {
                faults.push(format!("{into}: none of its {painted} painted pixels changed color"));
            }
        }
    }

    // Every blueprint has to be able to find the art it now names. An art the game
    // itself leaves without images - `rebel_long_hard` - is its own affair, so only a
    // name this pass wrote is held against the archive.
    let arts = arts_of(&archive);
    let stock_ships = blueprints_of(&stock);
    let mut notes: BTreeSet<String> = BTreeSet::new();
    for (file, name, img) in blueprints_of(&archive) {
        if arts.contains(&img) {
            continue;
        }
        let shipped = stock_ships
            .iter()
            .any(|(was_file, was_name, was_img)| *was_file == file && *was_name == name && *was_img == img);
        if shipped {
            notes.insert(format!("{name} names art {img}, which the game ships no images for"));
        } else {
            faults.push(format!("{file}: {name} is pointed at art {img}, which has no images"));
        }
    }

    // The player hulls are `ship_art.rs`'s, and their art carries the layout name.
    let after = images(&archive);
    for (path, was) in &before {
        if !hulls.iter().any(|hull| path.contains(&format!("{hull}_"))) {
            continue;
        }
        match after.get(path) {
            Some(now) if now == was => {}
            Some(_) => faults.push(format!("{path}: an enemy pass turned a player image")),
            None => faults.push(format!("{path}: a player image is gone")),
        }
    }

    for note in &notes {
        println!("note: {note}");
    }
    for warning in &turned.warnings {
        println!("warning: {warning}");
    }
    let writes: usize = turned.ships.iter().map(|ship| ship.files).sum();
    let copies = turned.ships.iter().filter(|ship| ship.into != ship.art).count();
    println!(
        "\n{} ships turned, {writes} images written, {} of them drawn from a copy of an art \
         the game shares, {} faults",
        turned.ships.len(),
        copies,
        faults.len(),
    );
    for fault in faults {
        println!("fault: {fault}");
    }
    Ok(())
}

/// The images of the archive, path and decoded picture. Only the ones a ship is drawn
/// from, which is what the pass can write.
fn images(archive: &FtlArchive) -> BTreeMap<String, RgbaImage> {
    let mut out = BTreeMap::new();
    for path in archive.paths().map(str::to_string).collect::<Vec<_>>() {
        let folder = path.strip_prefix("img/ships_noglow/").or(path.strip_prefix("img/ships_glow/"));
        if folder.is_none() {
            continue;
        }
        let Some(bytes) = archive.get_file(&path) else { continue };
        if let Ok(image) = image::load_from_memory(bytes) {
            out.insert(path, image.to_rgba8());
        }
    }
    out
}

/// The layouts that file art of their own in `img/ship`, which are the player hulls.
fn player_hulls(archive: &FtlArchive) -> Vec<String> {
    archive
        .paths()
        .filter_map(|path| path.strip_prefix("img/ship/"))
        .filter_map(|name| name.strip_suffix("_base.png"))
        .map(str::to_string)
        .collect()
}

/// The art names the archive can draw.
fn arts_of(archive: &FtlArchive) -> Vec<String> {
    archive
        .paths()
        .filter_map(|path| path.strip_prefix("img/ships_noglow/"))
        .filter_map(|name| name.strip_suffix("_base.png"))
        .map(str::to_string)
        .collect()
}

/// The color of a pixel, without its alpha.
fn rgb(pixel: &[u8; 4]) -> (u8, u8, u8) {
    (pixel[0], pixel[1], pixel[2])
}

/// A color as the six hex digits it is written with.
fn hex((red, green, blue): (u8, u8, u8)) -> String {
    format!("#{red:02x}{green:02x}{blue:02x}")
}

/// true when a color has no hue to move: its three channels are one value.
fn hueless((red, green, blue): (u8, u8, u8)) -> bool {
    red == green && green == blue
}

/// The enemy `<shipBlueprint>`s of an archive: the file that holds one, its name and
/// the art it names. A blueprint of a player hull is left out, as `ship_art.rs` owns
/// those, and a blueprint that names no art is named by its layout.
fn blueprints_of(archive: &FtlArchive) -> Vec<(String, String, String)> {
    let hulls = player_hulls(archive);
    let mut out = Vec::new();
    for path in archive.paths().map(str::to_string).collect::<Vec<_>>() {
        if !path.starts_with("data/") || !path.ends_with(".xml") {
            continue;
        }
        let Some(bytes) = archive.get_file(&path) else { continue };
        let Ok(text) = String::from_utf8(bytes.to_vec()) else { continue };
        if !text.contains("<shipBlueprint") {
            continue;
        }
        let Ok(inside) = blueprints::elements(&text, 0) else { continue };
        for element in inside
            .into_iter()
            .filter(|element| element.name == "shipBlueprint")
        {
            let tag = &text[element.tag.clone()];
            let Some((_, name)) = blueprints::attribute(tag, "name") else { continue };
            let Some((_, layout)) = blueprints::attribute(tag, "layout") else { continue };
            if hulls.iter().any(|hull| hull == layout) {
                continue;
            }
            let img = blueprints::attribute(tag, "img").map_or(layout, |(_, img)| img);
            out.push((path.clone(), name.to_string(), img.to_string()));
        }
    }
    out
}

fn decode(bytes: &[u8], path: &str) -> Result<RgbaImage, String> {
    image::load_from_memory(bytes)
        .map(|image| image.to_rgba8())
        .map_err(|error| format!("{path}: {error}"))
}