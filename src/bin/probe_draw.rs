//! Draw the rooms of a layout file over the hull art: probe_draw <txt> <base png> <out png> ...
//! Pairs of inputs are stacked side by side.

use image::{ImageBuffer, Rgba, RgbaImage};
use std::collections::BTreeMap;

const TILE: i32 = 35;
const ARITY: &[(&str, usize)] = &[
    ("X_OFFSET", 1),
    ("Y_OFFSET", 1),
    ("HORIZONTAL", 1),
    ("VERTICAL", 1),
    ("ELLIPSE", 4),
    ("ROOM", 5),
    ("DOOR", 5),
];
const PAINT: [[u8; 4]; 6] = [
    [220, 60, 60, 90],
    [60, 200, 90, 90],
    [70, 120, 240, 90],
    [240, 200, 60, 90],
    [200, 90, 230, 90],
    [70, 220, 220, 90],
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = args.last().cloned().ok_or("usage: probe_draw <txt> <png> [...] <out.png>")?;
    let pairs = args[..args.len() - 1]
        .chunks(2)
        .map(|c| c.to_vec())
        .collect::<Vec<_>>();

    let mut canvas: Option<RgbaImage> = None;
    for pair in pairs {
        let (txt, png) = match pair.as_slice() {
            [a, b] => (a.as_str(), b.as_str()),
            _ => return Err("inputs come in txt, png pairs".into()),
        };
        let one = draw(txt, png)?;
        let width = one.width();
        canvas = Some(match canvas {
            None => one,
            Some(wide) => stitch(wide, one),
        });
        let _ = width;
    }
    let canvas = canvas.ok_or("nothing to draw")?;
    canvas.save(&out)?;
    println!("wrote {out} ({}x{})", canvas.width(), canvas.height());
    Ok(())
}

fn stitch(wide: RgbaImage, next: RgbaImage) -> RgbaImage {
    let height = wide.height().max(next.height());
    let mut out: RgbaImage =
        ImageBuffer::from_pixel(wide.width() + 8 + next.width(), height, Rgba([20, 20, 24, 255]));
    for y in 0..wide.height() {
        for x in 0..wide.width() {
            out.put_pixel(x, y, *wide.get_pixel(x, y));
        }
    }
    let x0 = wide.width() + 8;
    for y in 0..next.height() {
        for x in 0..next.width() {
            out.put_pixel(x0 + x, y, *next.get_pixel(x, y));
        }
    }
    out
}

fn draw(txt: &str, png: &str) -> Result<RgbaImage, Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(txt)?;
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let mut rooms = Vec::new();
    let mut doors = Vec::new();
    let mut at = 0;
    while at < tokens.len() {
        let Some((_, taken)) = ARITY.iter().find(|(key, _)| *key == tokens[at]) else {
            break;
        };
        let value: Vec<i32> = tokens[at + 1..at + 1 + taken]
            .iter()
            .filter_map(|t| t.parse().ok())
            .collect();
        match tokens[at] {
            "ROOM" => rooms.push(value),
            "DOOR" => doors.push(value),
            _ => {}
        }
        at += 1 + taken;
    }

    let art = image::open(png)?.to_rgba8();
    // HORIZONTAL and VERTICAL slide the ship on screen, not the art against the
    // room grid, so they are not read here.
    let (img_x, img_y) = offset(png)?;
    let origin = (-img_x, -img_y);
    let mut out = art.clone();
    for (index, room) in rooms.iter().enumerate() {
        let (x, y, w, h) = (room[1], room[2], room[3], room[4]);
        let paint = Rgba(PAINT[index % PAINT.len()]);
        for ty in y..y + h {
            for tx in x..x + w {
                for py in 0..TILE {
                    for px in 0..TILE {
                        let (u, v) = (origin.0 + tx * TILE + px, origin.1 + ty * TILE + py);
                        if u < 0 || v < 0 || u >= out.width() as i32 || v >= out.height() as i32 {
                            continue;
                        }
                        let u = u as u32;
                        let v = v as u32;
                        let edge = px == 0 || py == 0 || px == TILE - 1 || py == TILE - 1;
                        let here = *out.get_pixel(u, v);
                        let blend = |a: u8, b: u8| ((a as u16 + b as u16) / 2) as u8;
                        out.put_pixel(
                            u,
                            v,
                            if edge {
                                Rgba([paint.0[0], paint.0[1], paint.0[2], 255])
                            } else {
                                Rgba([
                                    blend(here.0[0], paint.0[0]),
                                    blend(here.0[1], paint.0[1]),
                                    blend(here.0[2], paint.0[2]),
                                    here.0[3],
                                ])
                            },
                        );
                    }
                }
            }
        }
    }
    let owner: BTreeMap<(i32, i32), usize> = rooms
        .iter()
        .enumerate()
        .flat_map(|(index, room)| {
            (room[2]..room[2] + room[4]).flat_map(move |y| (room[1]..room[1] + room[3]).map(move |x| ((x, y), index)))
        })
        .collect();
    for door in &doors {
        let (x, y, vert) = (door[0], door[1], door[4] == 1);
        let (a, b) = if vert {
            ((x - 1, y), (x, y))
        } else {
            ((x, y - 1), (x, y))
        };
        let open = door[2] == -1 || door[3] == -1;
        let mark = if open { Rgba([255, 255, 255, 255]) } else { Rgba([0, 0, 0, 255]) };
        for tile in [a, b] {
            if !open || owner.contains_key(&tile) {
                continue;
            }
            let (u, v) = (origin.0 + tile.0 * TILE, origin.1 + tile.1 * TILE);
            for py in 12..23 {
                for px in 12..23 {
                    if u + px >= 0 && v + py >= 0 {
                        out.put_pixel((u + px) as u32, (v + py) as u32, mark);
                    }
                }
            }
        }
    }
    Ok(out)
}

fn offset(png: &str) -> Result<(i32, i32), Box<dyn std::error::Error>> {
    let xml = png.replace("img/ship/", "data/").replace("_base.png", ".xml");
    let text = std::fs::read_to_string(&xml)?;
    let at = text.find("<img ").ok_or("no <img>")? + 5;
    let rest = &text[at..];
    let number = |key: &str| -> Result<i32, Box<dyn std::error::Error>> {
        let at = rest.find(key).ok_or(format!("no {key}"))? + key.len() + 1;
        let end = rest[at..]
            .find(|c: char| c == '"' || c.is_whitespace())
            .ok_or("unterminated")?;
        Ok(rest[at..at + end].parse()?)
    };
    Ok((number("x=")?, number("y=")?))
}