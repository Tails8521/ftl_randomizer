//! Probe: how the shipped ship layouts place doors and airlocks.
//!
//! Three questions, answered by reading every layout the game ships with:
//! * two rooms share a wall: how many doors go into that wall?
//! * a room side faces open space: how often is there an airlock, and do
//!   airlocks come in pairs on the 2 tile wide side?
//! * is the space behind an airlock outside the hull, or inner hull deck that no
//!   room uses?
//!
//! Door format in a `<ship>.txt` file, checked against the tiles in this probe:
//! `x`, `y`, room id left or top, room id right or bottom, then 1 for a vertical
//! door and 0 for a horizontal one. A door sits on the left wall of tile (x, y)
//! when it is vertical, and on the top wall of that tile when it is horizontal,
//! so it separates tile (x-1, y) from (x, y), or (x, y-1) from (x, y). A room id
//! of -1 means the other side is open space, which makes the door an airlock.
//!
//! Run `cargo run --release --bin probe_door_stats [all]`. Without `all` it only
//! reads the 28 player ships, which is what the randomizer copies.


use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{read_dir, File};
use std::io::Read;
use std::path::Path;
use std::{env, fs, process};

const TILE: i32 = 35;

/// A pair of tiles that share a wall: one tile of a room, and the tile on the
/// other side of that wall.
type Edge = ((i32, i32), (i32, i32));
const SHIP_IMG_DIR: &str = "extracted/img/ship";
const DATA_DIR: &str = "extracted/data";

#[derive(Clone, Copy, Debug)]
struct Room {
    id: i32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl Room {
    fn tiles(self) -> impl Iterator<Item = (i32, i32)> {
        (self.y..self.y + self.h).flat_map(move |y| (self.x..self.x + self.w).map(move |x| (x, y)))
    }

    /// The four sides of a room. Each side is a list of the room tile and the
    /// tile right outside it, in order along that side.
    fn sides(self) -> Vec<Vec<Edge>> {
        vec![
            (0..self.w).map(|i| ((self.x + i, self.y), (self.x + i, self.y - 1))).collect(),
            (0..self.w).map(|i| ((self.x + i, self.y + self.h - 1), (self.x + i, self.y + self.h))).collect(),
            (0..self.h).map(|i| ((self.x, self.y + i), (self.x - 1, self.y + i))).collect(),
            (0..self.h).map(|i| ((self.x + self.w - 1, self.y + i), (self.x + self.w, self.y + i))).collect(),
        ]
    }
}

#[derive(Clone, Copy, Debug)]
struct Door {
    x: i32,
    y: i32,
    left_top: i32,
    right_bottom: i32,
    vertical: bool,
}

impl Door {
    /// The two tiles this door sits between.
    fn tiles(self) -> Edge {
        if self.vertical {
            ((self.x - 1, self.y), (self.x, self.y))
        } else {
            ((self.x, self.y - 1), (self.x, self.y))
        }
    }

    fn airlock(self) -> bool {
        self.left_top == -1 || self.right_bottom == -1
    }
}

fn main() -> process::ExitCode {
    match run() {
        Ok(()) => process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let every_ship = env::args().nth(1).is_some_and(|a| a == "all");
    let names = if every_ship {
        all_layouts()?
    } else {
        player_ships()?
    };
    println!(
        "reading {} {} layouts",
        names.len(),
        if every_ship { "ship" } else { "player ship" }
    );

    let mut geometry = [0usize; 2];
    let mut interior: BTreeMap<(i32, i32), usize> = BTreeMap::new();
    let mut runs: BTreeMap<(i32, char, String), usize> = BTreeMap::new();
    let mut per_ship: BTreeMap<&str, (usize, usize, usize)> = BTreeMap::new();
    let mut airlock_by_size: BTreeMap<(i32, i32), usize> = BTreeMap::new();
    let mut room_by_size: BTreeMap<(i32, i32), usize> = BTreeMap::new();

    for name in &names {
        let path = Path::new(DATA_DIR).join(format!("{name}.txt"));
        if !path.exists() {
            continue;
        }
        let (rooms, doors) = parse_layout(&path)?;
        if rooms.is_empty() {
            continue;
        }
        let owner = owner_of(&rooms);

        // Does the recorded room id match the tiles the door sits between?
        for door in &doors {
            let (a, b) = door.tiles();
            let left = *owner.get(&a).unwrap_or(&-1);
            let right = *owner.get(&b).unwrap_or(&-1);
            let same = (door.left_top == left && door.right_bottom == right)
                || (door.left_top == right && door.right_bottom == left);
            geometry[usize::from(same)] += 1;
        }

        // Doors inside the wall between two rooms.
        let mut walls: BTreeMap<(i32, i32), Vec<Edge>> = BTreeMap::new();
        for (&tile, &room) in &owner {
            for delta in [(1, 0), (0, 1)] {
                let other = (tile.0 + delta.0, tile.1 + delta.1);
                if let Some(&neighbour) = owner.get(&other)
                    && neighbour != room
                {
                    let pair = if room < neighbour { (room, neighbour) } else { (neighbour, room) };
                    walls.entry(pair).or_default().push((tile, other));
                }
            }
        }
        let mut doors_in_wall: BTreeMap<(i32, i32), usize> = BTreeMap::new();
        for door in &doors {
            if door.airlock() {
                continue;
            }
            let pair = if door.left_top < door.right_bottom {
                (door.left_top, door.right_bottom)
            } else {
                (door.right_bottom, door.left_top)
            };
            *doors_in_wall.entry(pair).or_default() += 1;
        }
        for (pair, slots) in &walls {
            let count = doors_in_wall.get(pair).copied().unwrap_or(0) as i32;
            *interior.entry((slots.len() as i32, count)).or_default() += 1;
        }

        // Airlocks, side by side. Open edges are grouped into runs, because a
        // pair of airlocks needs two open edges next to each other.
        let deck = inner_deck(name)?;
        for room in &rooms {
            *room_by_size.entry((room.w, room.h)).or_default() += 1;
            for side in room.sides() {
                // One edge of a side: open space, and whether an airlock sits
                // there, and whether the open space is hull deck.
                let edges: Vec<(bool, bool, bool)> = side
                    .iter()
                    .map(|(inside, outside)| {
                        let open = !owner.contains_key(outside);
                        let airlock = open
                            && doors.iter().any(|d| {
                                let (a, b) = d.tiles();
                                d.airlock()
                                    && ((a == *inside && b == *outside) || (a == *outside && b == *inside))
                            });
                        (open, airlock, deck.contains(outside))
                    })
                    .collect();
                for run in open_runs(&edges) {
                    let kind = match run.iter().filter(|(_, _, is_deck)| *is_deck).count() {
                        0 => 's',
                        all if all == run.len() => 'd',
                        _ => 'm',
                    };
                    let pattern: String = run.iter().map(|(_, lock, _)| if *lock { 'A' } else { '.' }).collect();
                    *airlock_by_size.entry((room.w, room.h)).or_default() += pattern.matches('A').count();
                    *runs.entry((run.len() as i32, kind, pattern)).or_default() += 1;
                }
            }
        }

        let airlocks = doors.iter().filter(|d| d.airlock()).count();
        per_ship.insert(name, (rooms.len(), doors.len(), airlocks));
    }

    report(&geometry, &interior, &runs, &per_ship, &airlock_by_size, &room_by_size);
    Ok(())
}

/// Stretches of open edges inside one side of a room. An edge blocked by another
/// room breaks the stretch, because a door there would not be an airlock.
fn open_runs(edges: &[(bool, bool, bool)]) -> Vec<Vec<(bool, bool, bool)>> {
    let mut runs = Vec::new();
    let mut current = Vec::new();
    for edge in edges {
        if !edge.0 {
            if !current.is_empty() {
                runs.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(*edge);
    }
    if !current.is_empty() {
        runs.push(current);
    }
    runs
}

fn report(
    geometry: &[usize; 2],
    interior: &BTreeMap<(i32, i32), usize>,
    runs: &BTreeMap<(i32, char, String), usize>,
    per_ship: &BTreeMap<&str, (usize, usize, usize)>,
    airlock_by_size: &BTreeMap<(i32, i32), usize>,
    room_by_size: &BTreeMap<(i32, i32), usize>,
) {
    println!(
        "\ndoor tiles match the recorded room ids: {} yes, {} no",
        geometry[1], geometry[0]
    );

    println!("\ndoors in the wall between two rooms");
    let mut lengths: Vec<i32> = interior.keys().map(|(l, _)| *l).collect();
    lengths.dedup();
    for length in lengths {
        let row: Vec<(i32, usize)> = interior
            .iter()
            .filter(|((l, _), _)| *l == length)
            .map(|((_, c), n)| (*c, *n))
            .collect();
        let pairs: usize = row.iter().map(|(_, n)| n).sum();
        let doors: usize = row.iter().map(|(c, n)| *c as usize * n).sum();
        let share = row
            .iter()
            .map(|(c, n)| format!("{c} door{}: {:.0}%", if *c == 1 { "" } else { "s" }, 100.0 * *n as f64 / pairs as f64))
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "  wall {length} tile{}: {pairs:5} pairs, {:.2} doors on average, {share}",
            if length == 1 { "" } else { "s" },
            doors as f64 / pairs as f64
        );
    }

    println!("\nopen runs on a room side, `d` = inner hull deck, `s` = outside the ship");
    let mut keys: Vec<(i32, char)> = runs.keys().map(|(l, k, _)| (*l, *k)).collect();
    keys.sort();
    keys.dedup();
    for (length, kind) in keys {
        let mut row: Vec<(String, usize)> = runs
            .iter()
            .filter(|((l, k, _), _)| *l == length && *k == kind)
            .map(|((_, _, p), n)| (p.clone(), *n))
            .collect();
        let total: usize = row.iter().map(|(_, n)| n).sum();
        let with: usize = row.iter().filter(|(p, _)| p.chars().any(|c| c.is_uppercase())).map(|(_, n)| n).sum();
        row.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let share = row
            .iter()
            .map(|(p, n)| format!("{p}: {:.0}%", 100.0 * *n as f64 / total as f64))
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "  run {length} {kind}: {total:5} runs, {:.1}% carry an airlock, {share}",
            100.0 * with as f64 / total as f64
        );
    }

    // The pair rule in one line: on a side two tiles wide with nothing in the
    // way, does an airlock come alone or together with its partner?
    let mut paired = 0;
    let mut alone = 0;
    for ((length, _, pattern), count) in runs {
        if *length == 2 {
            match pattern.matches('A').count() {
                2 => paired += count,
                1 => alone += count,
                _ => {}
            }
        }
    }
    println!(
        "\ntwo tile open side carrying an airlock: {paired} pairs, {alone} alone, a pair is {:.1} times as common",
        paired as f64 / alone.max(1) as f64
    );

    println!("\nairlocks by room size");
    for size in room_by_size.keys() {
        println!(
            "  {0}x{1}: {2:5} rooms, {3:4} airlocks",
            size.0,
            size.1,
            room_by_size[size],
            airlock_by_size.get(size).copied().unwrap_or(0)
        );
    }

    let rooms: Vec<usize> = per_ship.values().map(|v| v.0).collect();
    let doors: Vec<usize> = per_ship.values().map(|v| v.1).collect();
    let airlocks: Vec<usize> = per_ship.values().map(|v| v.2).collect();
    let span = |v: &[usize]| -> String {
        let sum: usize = v.iter().sum();
        format!("min {} / avg {:.1} / max {}", v.iter().min().unwrap_or(&0), sum as f64 / v.len().max(1) as f64, v.iter().max().unwrap_or(&0))
    };
    println!("\nper layout: rooms {}, doors {}, airlocks {}", span(&rooms), span(&doors), span(&airlocks));
    let worst = per_ship.iter().min_by_key(|(_, v)| v.2);
    if let Some((name, (_, _, airlocks))) = worst {
        println!("fewest airlocks: {name} with {airlocks}");
    }
}

// ------------------------------------------------------------------- input --

fn player_ships() -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut names = Vec::new();
    for entry in read_dir(SHIP_IMG_DIR)? {
        let path = entry?.path();
        let Some(stem) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix("_base.png"))
        else {
            continue;
        };
        names.push(stem.to_string());
    }
    names.sort();
    Ok(names)
}

fn all_layouts() -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut names = Vec::new();
    for entry in read_dir(DATA_DIR)? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) == Some("txt")
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            names.push(stem.to_string());
        }
    }
    names.sort();
    Ok(names)
}

fn parse_layout(path: &Path) -> Result<(Vec<Room>, Vec<Door>), Box<dyn std::error::Error>> {
    let text = fs::read_to_string(path)?;
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let number = |i: usize| lines.get(i).and_then(|l| l.parse::<i32>().ok()).unwrap_or(0);

    let mut rooms = Vec::new();
    let mut doors = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        match lines[i] {
            "ROOM" => {
                rooms.push(Room { id: number(i + 1), x: number(i + 2), y: number(i + 3), w: number(i + 4), h: number(i + 5) });
                i += 6;
            }
            "DOOR" => {
                doors.push(Door {
                    x: number(i + 1),
                    y: number(i + 2),
                    left_top: number(i + 3),
                    right_bottom: number(i + 4),
                    vertical: number(i + 5) == 1,
                });
                i += 6;
            }
            _ => i += 1,
        }
    }
    Ok((rooms, doors))
}

fn owner_of(rooms: &[Room]) -> HashMap<(i32, i32), i32> {
    let mut owner = HashMap::new();
    for room in rooms {
        for tile in room.tiles() {
            owner.insert(tile, room.id);
        }
    }
    owner
}

/// Tiles of inner hull deck: fully opaque in the base image, so part of the
/// ship, but no room covers them.
fn inner_deck(name: &str) -> Result<BTreeSet<(i32, i32)>, Box<dyn std::error::Error>> {
    let mut empty = BTreeSet::new();
    let image_path = Path::new(SHIP_IMG_DIR).join(format!("{name}_base.png"));
    if !image_path.exists() {
        return Ok(empty);
    }
    let base = image::open(&image_path)?.to_rgba8();
    let (horizontal, vertical) = offsets(&Path::new(DATA_DIR).join(format!("{name}.txt")))?;
    let (img_x, img_y) = image_corner(&Path::new(DATA_DIR).join(format!("{name}.xml")))?;
    let origin = (-img_x + horizontal, -img_y + vertical);
    let (width, height) = (base.width() as i32, base.height() as i32);
    for ty in -2..=height / TILE + 2 {
        for tx in -2..=width / TILE + 2 {
            let x0 = origin.0 + tx * TILE;
            let y0 = origin.1 + ty * TILE;
            if x0 < 0 || y0 < 0 || x0 + TILE > width || y0 + TILE > height {
                continue;
            }
            let opaque = (0..TILE).all(|dy| (0..TILE).all(|dx| base.get_pixel((x0 + dx) as u32, (y0 + dy) as u32)[3] == 255));
            if opaque {
                empty.insert((tx, ty));
            }
        }
    }
    Ok(empty)
}

fn offsets(path: &Path) -> Result<(i32, i32), Box<dyn std::error::Error>> {
    let text = fs::read_to_string(path)?;
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let mut out = (0, 0);
    for i in 0..lines.len() {
        let value = lines.get(i + 1).and_then(|l| l.parse::<i32>().ok()).unwrap_or(0);
        match lines[i] {
            "HORIZONTAL" => out.0 = value,
            "VERTICAL" => out.1 = value,
            _ => {}
        }
    }
    Ok(out)
}

fn image_corner(path: &Path) -> Result<(i32, i32), Box<dyn std::error::Error>> {
    let mut text = String::new();
    File::open(path)?.read_to_string(&mut text)?;
    let mut reader = Reader::from_reader(text.as_bytes());
    let mut corner = (0, 0);
    let mut buffer = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if e.name().as_ref() == b"img" => {
                for attribute in e.attributes().flatten() {
                    let value = String::from_utf8_lossy(&attribute.value).parse::<i32>().ok();
                    match attribute.key.as_ref() {
                        b"x" => corner.0 = value.unwrap_or(0),
                        b"y" => corner.1 = value.unwrap_or(0),
                        _ => {}
                    }
                }
                break;
            }
            Ok(Event::Eof) => break,
            Err(error) => return Err(Box::new(error)),
            _ => buffer.clear(),
        }
    }
    Ok(corner)
}