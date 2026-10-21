//! How much room the generator has left when rooms must stay inside the box the
//! vanilla layout used. Probes the archive, writes nothing.

use ftl_randomizer::layout;
use ftl_randomizer::{ship_txt::ShipTxt, FtlArchive};
use image::ImageReader;
use std::collections::BTreeSet;
use std::io::Read;

const KEYS: (&str, usize) = ("", 0);
const ARITY: &[(&str, usize)] = &[
    ("X_OFFSET", 1),
    ("Y_OFFSET", 1),
    ("HORIZONTAL", 1),
    ("VERTICAL", 1),
    ("ELLIPSE", 4),
    ("ROOM", 5),
    ("DOOR", 5),
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut raw = Vec::new();
    std::fs::File::open("data/ftl.dat")?.read_to_end(&mut raw)?;
    let archive = FtlArchive::from_bytes(&raw)?;

    let mut layouts: Vec<String> = archive
        .paths()
        .filter_map(|p| p.strip_prefix("img/ship/"))
        .filter_map(|p| p.strip_suffix("_base.png"))
        .map(String::from)
        .collect();
    layouts.sort();

    println!(
        "{:<22} {:<16} {:>5} {:>5} {:>6} {:>6}",
        "layout", "vanilla box", "mask", "box", "vrm", "98/90/75/50/25% box tiles / vanilla tiles they lose"
    );
    let _ = KEYS;
    for layout_name in layouts {
        let txt_path = format!("data/{layout_name}.txt");
        let Some(txt_bytes) = archive.get_file(&txt_path) else {
            continue;
        };
        // Parsed for its shape alone: an unreadable layout is worth a line of its own.
        ShipTxt::parse(txt_bytes.to_vec()).map_err(|_| txt_path.clone())?;
        let xml = archive
            .get_file(&format!("data/{layout_name}.xml"))
            .ok_or(txt_path.clone())?
            .to_vec();
        let art_bytes = archive
            .get_file(&format!("img/ship/{layout_name}_base.png"))
            .ok_or(txt_path.clone())?
            .to_vec();
        let art = ImageReader::new(std::io::Cursor::new(art_bytes))
            .with_guessed_format()?
            .decode()?
            .to_rgba8();

        // HORIZONTAL and VERTICAL slide the ship on screen, not the art against
        // the room grid, so they play no part in where a tile stands.
        let (img_x, img_y) = offset(&xml)?;
        let origin = (-img_x, -img_y);
        let mask = layout::hull_mask(&art, origin);

        let rooms = rooms_of(std::str::from_utf8(txt_bytes)?);
        let min_x = rooms.iter().map(|r| r[1]).min().unwrap();
        let min_y = rooms.iter().map(|r| r[2]).min().unwrap();
        let max_x = rooms.iter().map(|r| r[1] + r[3] - 1).max().unwrap();
        let max_y = rooms.iter().map(|r| r[2] + r[4] - 1).max().unwrap();

        let box_tiles: Vec<(i32, i32)> = (min_x..=max_x)
            .flat_map(|x| (min_y..=max_y).map(move |y| (x, y)))
            .collect();
        let in_box: BTreeSet<(i32, i32)> = box_tiles
            .iter()
            .copied()
            .filter(|tile| mask.contains(tile))
            .collect();
        let mut line = String::new();
        for part in [0.98, 0.9, 0.75, 0.5, 0.25] {
            let loose: Vec<(i32, i32)> = box_tiles
                .iter()
                .copied()
                .filter(|tile| coverage(&art, origin, *tile) >= part)
                .collect();
            let vanilla: Vec<[i32; 5]> = rooms.clone();
            let lost: usize = vanilla
                .iter()
                .map(|r| {
                    (0..r[3])
                        .flat_map(|dx| (0..r[4]).map(move |dy| (r[1] + dx, r[2] + dy)))
                        .filter(|t| !loose.contains(t))
                        .count()
                })
                .sum();
            line.push_str(&format!(" {:>3}/{:<4}", loose.len(), lost));
        }
        let vanilla_tiles: usize = rooms.iter().map(|r| (r[3] * r[4]) as usize).sum();
        let vanilla_in_mask: usize = rooms
            .iter()
            .map(|r| {
                (0..r[3])
                    .flat_map(|dx| (0..r[4]).map(move |dy| (r[1] + dx, r[2] + dy)))
                    .filter(|tile| mask.contains(tile))
                    .count()
            })
            .sum();

        let edge = |column: bool, value: i32| {
            box_tiles
                .iter()
                .filter(|(x, y)| if column { *x == value } else { *y == value })
                .filter(|tile| coverage(&art, origin, **tile) >= 0.75)
                .count()
        };
        let edges = format!(
            "  edges {} {} {} {}",
            edge(true, min_x),
            edge(true, max_x),
            edge(false, min_y),
            edge(false, max_y)
        );
        println!(
            "{:<20} ({:>2},{:>2},{:>2},{:>2}) {:>4} {:>4} {:>4}{}{}",
            layout_name,
            min_x,
            min_y,
            max_x,
            max_y,
            mask.len(),
            in_box.len(),
            vanilla_tiles,
            line + &edges,
            if vanilla_in_mask == vanilla_tiles {
                ""
            } else {
                " *"
            }
        );
    }
    Ok(())
}

/// The `<img x= y=>` of a layout xml.
fn offset(xml: &[u8]) -> Result<(i32, i32), String> {
    use quick_xml::events::Event;
    use quick_xml::reader::Reader;
    let mut reader = Reader::from_reader(xml);
    let mut buffer = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(tag)) | Ok(Event::Empty(tag))
                if tag.local_name().as_ref() == b"img" =>
            {
                let mut x = None;
                let mut y = None;
                for attribute in tag.attributes().flatten() {
                    let value = String::from_utf8_lossy(&attribute.value).parse::<i32>().ok();
                    match attribute.key.as_ref() {
                        b"x" => x = value,
                        b"y" => y = value,
                        _ => {}
                    }
                }
                if let (Some(x), Some(y)) = (x, y) {
                    return Ok((x, y));
                }
            }
            Ok(Event::Eof) => return Err("no <img>".to_string()),
            Err(error) => return Err(error.to_string()),
            _ => buffer.clear(),
        }
    }
}

/// The ROOM records of a layout file.
fn rooms_of(text: &str) -> Vec<[i32; 5]> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let mut out = Vec::new();
    let mut at = 0;
    while at < tokens.len() {
        let Some((_, taken)) = ARITY.iter().find(|(key, _)| *key == tokens[at]) else {
            break;
        };
        let values: Vec<i32> = tokens[at + 1..at + 1 + taken]
            .iter()
            .filter_map(|token| token.parse().ok())
            .collect();
        if tokens[at] == "ROOM" && values.len() == 5 {
            out.push([values[0], values[1], values[2], values[3], values[4]]);
        }
        at += 1 + taken;
    }
    out
}
/// Share of the 35x35 pixels of a tile that are fully opaque.
fn coverage(art: &image::RgbaImage, origin: (i32, i32), tile: (i32, i32)) -> f64 {
    let (x0, y0) = (origin.0 + tile.0 * 35, origin.1 + tile.1 * 35);
    let mut solid = 0u32;
    for dy in 0..35 {
        for dx in 0..35 {
            let (x, y) = (x0 + dx, y0 + dy);
            if x >= 0 && y >= 0 && (x as u32) < art.width() && (y as u32) < art.height() {
                if art.get_pixel(x as u32, y as u32).0[3] == 255 {
                    solid += 1;
                }
            }
        }
    }
    f64::from(solid) / 1225.0
}
