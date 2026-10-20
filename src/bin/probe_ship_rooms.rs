//! Probe: draw and measure the layouts `ftl_randomizer::layout` makes for the 28
//! player ships.
//!
//! The 28 player ships are the ships that own a `<ship>_base.png` image. For each
//! one this probe reads the tile grid out of `extracted/data/<ship>.txt` and
//! `extracted/data/<ship>.xml`, works out which 35x35 tiles the hull allows, hands
//! that to `layout::generate`, then draws what came back and checks it.
//!
//! The generation itself lives in `src/layout.rs`. It is deliberately unchecked,
//! so the checks live here, together with the measurements that say the layouts
//! read like the ones the game ships:
//! * every room is 1x2, 2x1 or 2x2 tiles, and a 2x2 block is tried first;
//! * every tile of every room is fully inside the hull, which means all 1225
//!   pixels of that tile have alpha 255 in the base image;
//! * rooms never overlap;
//! * the room count follows the size of the hull, from 15 rooms on a small ship
//!   to 25 on a big one;
//! * the room graph is connected, so no room is cut off from the rest. A room is
//!   reachable from another when the two share a wall, which is where a door can
//!   go;
//! * every room can be reached through built doors, every door sits on the tiles
//!   of the two rooms it names, and an airlock on a side two tiles wide comes as
//!   a pair.
//!
//! Run `cargo run --release --bin probe_ship_rooms [seed]`. Images land in
//! `temp/ship_rooms`.

use image::{Rgba, RgbaImage};
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fs::{create_dir_all, read_dir};
use std::path::Path;
use std::{env, fs, process};

use ftl_randomizer::layout::{self, Door, Room, Tile, TILE};
use ftl_randomizer::rng::Rng;

/// Every layout must reach this room count.
const MIN_ROOMS: usize = 15;
/// A big hull never gets more rooms than this.
const MAX_ROOMS: usize = 25;

const SHIP_IMG_DIR: &str = "extracted/img/ship";
const DATA_DIR: &str = "extracted/data";
const OUT_DIR: &str = "temp/ship_rooms";
const SHEET_COLUMNS: usize = 4;
const CELL_MARGIN: u32 = 14;

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
    let seed = env::args()
        .nth(1)
        .map(|arg| {
            arg.parse::<u64>()
                .map_err(|e| -> Box<dyn std::error::Error> { format!("bad seed {arg}: {e}").into() })
        })
        .unwrap_or(Ok(1))?;

    let ships = find_ships()?;
    println!("found {} player ships, seed {seed}", ships.len());

    let out = Path::new(OUT_DIR);
    create_dir_all(out)?;

    let mut rng = Rng::new(seed);
    let mut drawn: Vec<(String, RgbaImage)> = Vec::new();
    let mut dump = String::new();
    let mut failures = 0;

    println!(
        "{:<20}{:>7}{:>7}{:>7}{:>7}{:>7}{:>9}  check",
        "ship", "hull", "orig", "rooms", "tiles", "doors", "airlocks"
    );
    let mut wall_counts: BTreeMap<(i32, i32), usize> = BTreeMap::new();
    let mut run_counts: BTreeMap<(i32, char, String), usize> = BTreeMap::new();
    let mut airlock_total = 0;
    for name in &ships {
        let ship = load_ship(name)?;
        let anchor = (
            ship.mask.iter().map(|(x, _)| *x).min().unwrap_or(0),
            ship.mask.iter().map(|(_, y)| *y).min().unwrap_or(0),
        );
        let layout = layout::generate(&ship.mask, &mut rng, layout::MIN_ROOMS, anchor);
        let rooms = layout.rooms;
        let doors = layout.doors;

        let mut report = check_layout(&ship.mask, &rooms);
        let (unreachable, bad_doors) = check_doors(&rooms, &doors);
        report.unreachable = unreachable;
        report.bad_doors = bad_doors;
        report.lone_airlocks = lone_airlocks(&rooms, &ship.mask, &doors);

        let tiles: usize = rooms.iter().map(|room| (room.w * room.h) as usize).sum();
        let interior: Vec<Door> = doors.iter().copied().filter(|door| !door.airlock()).collect();
        let airlocks = doors.len() - interior.len();
        airlock_total += airlocks;
        for (length, count) in door_spread(&rooms, &interior) {
            *wall_counts.entry((length, count)).or_default() += 1;
        }
        for (length, kind, pattern) in airlock_patterns(&rooms, &ship.mask, &doors) {
            *run_counts.entry((length, kind, pattern)).or_default() += 1;
        }

        println!(
            "{:<20}{:>7}{:>7}{:>7}{:>7}{:>7}{:>9}  {}",
            name,
            ship.mask.len(),
            ship.original.len(),
            rooms.len(),
            tiles,
            interior.len(),
            airlocks,
            report
        );
        if !report.ok() {
            failures += 1;
        }

        let image = draw_ship(&ship, &rooms, &doors);
        image.save(out.join(format!("{}.png", ship.name)))?;
        drawn.push((name.clone(), image));

        dump.push_str(&format!("{name}: {} rooms, {} doors, {airlocks} airlocks",
            rooms.len(), interior.len()));
        for room in &rooms {
            dump.push_str(&format!(" ({}x{} at {},{})", room.w, room.h, room.x, room.y));
        }
        dump.push('\n');
        for door in &doors {
            dump.push_str(&format!(
                "  door {} {} {} {} {}",
                door.x,
                door.y,
                door.left_top,
                door.right_bottom,
                if door.vertical { 1 } else { 0 }
            ));
            dump.push('\n');
        }
    }

    let sheet = compose_sheet(&drawn);
    sheet.save(out.join("sheet.png"))?;
    fs::write(out.join("layouts.txt"), dump)?;
    println!("wrote {} single images, sheet.png and layouts.txt to {OUT_DIR}", drawn.len());

    println!("\ndoors per wall in these layouts, and what the shipped layouts do");
    for length in [1, 2] {
        let row: Vec<(i32, usize)> = wall_counts
            .iter()
            .filter(|((l, _), _)| *l == length)
            .map(|((_, c), n)| (*c, *n))
            .collect();
        let pairs: usize = row.iter().map(|(_, n)| n).sum();
        if pairs == 0 {
            continue;
        }
        let mine = row
            .iter()
            .map(|(c, n)| format!("{c}: {:.0}%", 100.0 * *n as f64 / pairs as f64))
            .collect::<Vec<_>>()
            .join(", ");
        let shipped = WALL_DOOR_ODDS
            .iter()
            .find(|(l, _)| *l == length)
            .map(|(_, weights)| {
                weights
                    .iter()
                    .enumerate()
                    .map(|(count, w)| format!("{count}: {:.0}%", *w as f64 / 10.0))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        println!("  wall {length} tile{}: {pairs} walls, made {mine} | shipped {shipped}",
            if length == 1 { "" } else { "s" });
    }
    println!(
        "airlocks: {} over {} ships, {:.1} per ship, the shipped layouts average 7.0",
        airlock_total,
        ships.len(),
        airlock_total as f64 / ships.len() as f64
    );

    println!("\nairlocks on an open run of a room side, `d` = inner hull deck, `s` = outside the ship");
    let mut keys: Vec<(i32, char)> = run_counts.keys().map(|(l, k, _)| (*l, *k)).collect();
    keys.sort();
    keys.dedup();
    for (length, kind) in keys {
        let row: Vec<(String, usize)> = run_counts
            .iter()
            .filter(|((l, k, _), _)| *l == length && *k == kind)
            .map(|((_, _, pattern), count)| (pattern.clone(), *count))
            .collect();
        let total: usize = row.iter().map(|(_, count)| count).sum();
        let made = row
            .iter()
            .map(|(pattern, count)| format!("{pattern}: {:.0}%", 100.0 * *count as f64 / total as f64))
            .collect::<Vec<_>>()
            .join(", ");
        let shipped = SHIPPED_AIRLOCK_ODDS
            .iter()
            .find(|(l, k, _, _)| *l == length && *k == kind)
            .map(|(_, _, patterns, weights)| {
                patterns
                    .iter()
                    .zip(weights.iter())
                    .map(|(pattern, w)| format!("{pattern}: {:.0}%", *w as f64 / 10.0))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        println!("  run {length} {kind}: {total} runs, made {made} | shipped {shipped}");
    }

    if failures > 0 {
        return Err(format!("{failures} ship(s) broke a rule, see the table above").into());
    }
    println!("all layouts are inside the hull, and all room graphs are connected");
    Ok(())
}

// ---------------------------------------------------------------- shipping --

/// One ship: its tile grid, the tiles the hull allows, and its base image.
struct Ship {
    name: String,
    /// Pixel of tile (0,0) inside the base image.
    origin: (i32, i32),
    /// Tiles a room may cover: every pixel is fully opaque.
    mask: BTreeSet<Tile>,
    base: RgbaImage,
    /// Rooms of the layout the game ships with, only used for the report.
    original: Vec<Room>,
}

/// Every ship that owns a base image, sorted by name.
fn find_ships() -> Result<Vec<String>, Box<dyn std::error::Error>> {
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
        let stem = stem.to_string();
        if Path::new(DATA_DIR).join(format!("{stem}.txt")).exists() {
            names.push(stem);
        } else {
            eprintln!("warning: {stem} has a base image but no layout file, skipped");
        }
    }
    names.sort();
    if names.len() != 28 {
        eprintln!("warning: expected 28 player ships, found {}", names.len());
    }
    Ok(names)
}

fn load_ship(name: &str) -> Result<Ship, Box<dyn std::error::Error>> {
    let base = image::open(Path::new(SHIP_IMG_DIR).join(format!("{name}_base.png")))?.to_rgba8();
    let (horizontal, vertical, original) = parse_layout(&Path::new(DATA_DIR).join(format!("{name}.txt")))?;
    let (img_x, img_y) = parse_image_offset(&Path::new(DATA_DIR).join(format!("{name}.xml")))?;

    // Tile (0,0) sits at the image corner moved back to the ship origin, then by
    // the pixel offsets the layout file asks for.
    let origin = (-img_x + horizontal, -img_y + vertical);
    let mask = layout::hull_mask(&base, origin);
    Ok(Ship { name: name.to_string(), origin, mask, base, original })
}

/// Read HORIZONTAL, VERTICAL and every ROOM block of a `<ship>.txt` file.
fn parse_layout(path: &Path) -> Result<(i32, i32, Vec<Room>), Box<dyn std::error::Error>> {
    let text = fs::read_to_string(path)?;
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();

    let mut horizontal = 0;
    let mut vertical = 0;
    let mut rooms = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let number = |i: usize| lines.get(i).and_then(|l| l.parse::<i32>().ok()).unwrap_or(0);
        match lines[i] {
            "HORIZONTAL" => {
                horizontal = number(i + 1);
                i += 2;
            }
            "VERTICAL" => {
                vertical = number(i + 1);
                i += 2;
            }
            "ROOM" => {
                rooms.push(Room { x: number(i + 2), y: number(i + 3), w: number(i + 4), h: number(i + 5) });
                i += 6;
            }
            // One door is a position, two room ids and a direction.
            "DOOR" => i += 6,
            // X_OFFSET, Y_OFFSET and ELLIPSE move the ship on screen, not the
            // rooms inside it, so the probe ignores them.
            _ => i += 1,
        }
    }
    Ok((horizontal, vertical, rooms))
}

/// Read the `<img x= y=>` corner of the base image inside `<ship>.xml`.
fn parse_image_offset(path: &Path) -> Result<(i32, i32), Box<dyn std::error::Error>> {
    let mut reader = Reader::from_file(path)?;
    let mut corner = None;
    let mut buffer = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(element)) | Ok(Event::Empty(element)) if element.name().as_ref() == b"img" => {
                let mut x = None;
                let mut y = None;
                for attribute in element.attributes().flatten() {
                    let value = String::from_utf8_lossy(&attribute.value).parse::<i32>().ok();
                    match attribute.key.as_ref() {
                        b"x" => x = value,
                        b"y" => y = value,
                        _ => {}
                    }
                }
                corner = Some((x.ok_or("<img> has no x")?, y.ok_or("<img> has no y")?));
                break;
            }
            Ok(Event::Eof) => break,
            Err(error) => return Err(Box::new(error)),
            _ => buffer.clear(),
        }
    }
    corner.ok_or_else(|| "<img> not found".into())
}

// ------------------------------------------------------------------ checks --

/// A pair of tiles that share a wall: one tile of a room, and the tile on the
/// other side of that wall.
type Edge = (Tile, Tile);

/// The wall between two rooms, keyed by the two room ids, and the tiles that
/// touch across it. This is the probe's own reading of the layout, written again
/// so the checks do not lean on the code they check.
fn walls_between(rooms: &[Room]) -> BTreeMap<(usize, usize), Vec<Edge>> {
    let mut owner: HashMap<Tile, usize> = HashMap::new();
    for (index, room) in rooms.iter().enumerate() {
        for tile in room.tiles() {
            owner.insert(tile, index);
        }
    }
    let mut walls: BTreeMap<(usize, usize), Vec<Edge>> = BTreeMap::new();
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
    // The loop above walks a hash map, so the slots of a wall arrive in any
    // order. Sort them, or the same seed measures a different layout in every run.
    for slots in walls.values_mut() {
        slots.sort();
    }
    walls
}

/// Result of the checks a layout has to pass.
struct Report {
    rooms: usize,
    outside: usize,
    overlap: usize,
    bad_shape: usize,
    linked: bool,
    /// Rooms that cannot be reached from room 0 through built doors.
    unreachable: usize,
    /// Doors that do not sit between the two rooms they name.
    bad_doors: usize,
    /// Airlocks that stand alone on a side two tiles wide, which is not allowed.
    lone_airlocks: usize,
}

impl Report {
    fn ok(&self) -> bool {
        self.rooms >= MIN_ROOMS
            && self.rooms <= MAX_ROOMS
            && self.outside == 0
            && self.overlap == 0
            && self.bad_shape == 0
            && self.linked
            && self.unreachable == 0
            && self.bad_doors == 0
            && self.lone_airlocks == 0
    }
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", if self.rooms >= MIN_ROOMS { "rooms ok" } else { "TOO FEW ROOMS" })?;
        if self.outside == 0 {
            write!(f, ", in hull")?;
        } else {
            write!(f, ", {} TILE(S) OUTSIDE", self.outside)?;
        }
        if self.overlap == 0 {
            write!(f, ", no overlap")?;
        } else {
            write!(f, ", {} OVERLAP", self.overlap)?;
        }
        if self.bad_shape == 0 {
            write!(f, ", shapes ok")?;
        } else {
            write!(f, ", {} BAD SHAPE", self.bad_shape)?;
        }
        if self.linked {
            write!(f, ", connected")?;
        } else {
            write!(f, ", NOT CONNECTED")?;
        }
        if self.unreachable == 0 {
            write!(f, ", doors reach all")?;
        } else {
            write!(f, ", {} ROOM(S) UNREACHABLE", self.unreachable)?;
        }
        if self.bad_doors == 0 {
            write!(f, ", doors ok")?;
        } else {
            write!(f, ", {} BAD DOOR(S)", self.bad_doors)?;
        }
        if self.lone_airlocks == 0 {
            write!(f, ", airlocks paired")
        } else {
            write!(f, ", {} LONE AIRLOCK(S)", self.lone_airlocks)
        }
    }
}

/// Re-check a finished layout without trusting the generator.
fn check_layout(mask: &BTreeSet<Tile>, rooms: &[Room]) -> Report {
    let mut seen: HashMap<Tile, usize> = HashMap::new();
    let mut outside = 0;
    let mut overlap = 0;
    let mut bad_shape = 0;
    for (index, room) in rooms.iter().enumerate() {
        if !matches!((room.w, room.h), (1, 2) | (2, 1) | (2, 2)) {
            bad_shape += 1;
        }
        for tile in room.tiles() {
            if !mask.contains(&tile) {
                outside += 1;
            }
            if seen.insert(tile, index).is_some() {
                overlap += 1;
            }
        }
    }
    Report {
        rooms: rooms.len(),
        outside,
        overlap,
        bad_shape,
        linked: is_connected(rooms),
        unreachable: 0,
        bad_doors: 0,
        lone_airlocks: 0,
    }
}

/// True when one walk over the rooms reaches all of them. Two rooms are linked
/// when a tile of one touches a tile of the other across a wall, which is where a
/// door can be built.
fn is_connected(rooms: &[Room]) -> bool {
    if rooms.len() < 2 {
        return true;
    }
    let mut owner: HashMap<Tile, usize> = HashMap::new();
    for (index, room) in rooms.iter().enumerate() {
        for tile in room.tiles() {
            owner.insert(tile, index);
        }
    }

    let mut seen = vec![false; rooms.len()];
    let mut queue = VecDeque::from([0usize]);
    seen[0] = true;
    let mut reached = 1;
    while let Some(current) = queue.pop_front() {
        for tile in rooms[current].tiles() {
            for neighbour in [(tile.0 + 1, tile.1), (tile.0 - 1, tile.1), (tile.0, tile.1 + 1), (tile.0, tile.1 - 1)] {
                if let Some(&other) = owner.get(&neighbour)
                    && !seen[other]
                {
                    seen[other] = true;
                    reached += 1;
                    queue.push_back(other);
                }
            }
        }
    }
    reached == rooms.len()
}

/// Check the doors of a layout: every room has to be reachable through doors, and
/// every door has to sit between the two rooms it names, or between a room and
/// open space when it is an airlock. A vertical door at `(x, y)` divides tile
/// `(x - 1, y)` from `(x, y)`, a horizontal one divides `(x, y - 1)` from
/// `(x, y)`, and `-1` is open space.
fn check_doors(rooms: &[Room], doors: &[Door]) -> (usize, usize) {
    let mut owner: HashMap<Tile, i32> = HashMap::new();
    for (index, room) in rooms.iter().enumerate() {
        for tile in room.tiles() {
            owner.insert(tile, index as i32);
        }
    }

    let mut bad_doors = 0;
    for door in doors {
        let (one, two) = door.tiles();
        let id = |tile: Tile| owner.get(&tile).copied().unwrap_or(-1);
        if id(one) != door.left_top || id(two) != door.right_bottom {
            bad_doors += 1;
        }
    }

    if rooms.len() < 2 {
        return (0, bad_doors);
    }
    let mut seen = vec![false; rooms.len()];
    let mut queue = VecDeque::from([0usize]);
    seen[0] = true;
    let mut reached = 1;
    while let Some(current) = queue.pop_front() {
        for door in doors.iter().filter(|door| !door.airlock()) {
            let (left, right) = (door.left_top as usize, door.right_bottom as usize);
            let other = if left == current {
                right
            } else if right == current {
                left
            } else {
                continue;
            };
            if !seen[other] {
                seen[other] = true;
                reached += 1;
                queue.push_back(other);
            }
        }
    }
    (rooms.len() - reached, bad_doors)
}

/// Airlocks that stand alone on a side two tiles wide. Only a room in the way may
/// break a pair, and that shortens the side to one tile, so a lone airlock on a
/// two tile open run is a mistake.
fn lone_airlocks(rooms: &[Room], mask: &BTreeSet<Tile>, doors: &[Door]) -> usize {
    airlock_patterns(rooms, mask, doors)
        .into_iter()
        .filter(|(length, _, pattern)| *length == 2 && pattern.matches('A').count() == 1)
        .count()
}

/// The open runs of every room side, as `(length, kind, pattern)`. `kind` is `d`
/// when the open space is inner hull deck, `s` when it is outside the ship and
/// `m` when the run sees both. The pattern is one letter per tile of the run:
/// `A` where an airlock was built, `.` where it was not.
fn airlock_patterns(
    rooms: &[Room],
    mask: &BTreeSet<Tile>,
    doors: &[Door],
) -> Vec<(i32, char, String)> {
    let mut owner: HashMap<Tile, usize> = HashMap::new();
    for (index, room) in rooms.iter().enumerate() {
        for tile in room.tiles() {
            owner.insert(tile, index);
        }
    }
    let mut out = Vec::new();
    for room in rooms {
        for cells in [
            side(*room, Direction::Up),
            side(*room, Direction::Down),
            side(*room, Direction::Left),
            side(*room, Direction::Right),
        ] {
            for run in open_runs(&cells, &owner) {
                let kind = match run.iter().filter(|(_, outside)| mask.contains(outside)).count() {
                    0 => 's',
                    taken if taken == run.len() => 'd',
                    _ => 'm',
                };
                let mut pattern = String::new();
                for (inside, outside) in &run {
                    let built = doors.iter().any(|door| {
                        door.airlock()
                            && ((door.tiles() == (*inside, *outside)) || door.tiles() == (*outside, *inside))
                    });
                    pattern.push(if built { 'A' } else { '.' });
                }
                out.push((run.len() as i32, kind, pattern));
            }
        }
    }
    out
}

/// How many doors each wall between two rooms ended up with, as
/// `(wall length in tiles, doors)` for every wall of the layout.
fn door_spread(rooms: &[Room], doors: &[Door]) -> Vec<(i32, i32)> {
    walls_between(rooms)
        .iter()
        .map(|(pair, slots)| {
            let count = doors
                .iter()
                .filter(|door| {
                    let (a, b) = (door.left_top as usize, door.right_bottom as usize);
                    (a == pair.0 && b == pair.1) || (a == pair.1 && b == pair.0)
                })
                .count();
            (slots.len() as i32, count as i32)
        })
        .collect()
}

enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// One side of a room: each room tile and the tile outside the room next to it.
fn side(room: Room, direction: Direction) -> Vec<Edge> {
    match direction {
        Direction::Up => (0..room.w).map(|i| ((room.x + i, room.y), (room.x + i, room.y - 1))).collect(),
        Direction::Down => (0..room.w).map(|i| ((room.x + i, room.y + room.h - 1), (room.x + i, room.y + room.h))).collect(),
        Direction::Left => (0..room.h).map(|i| ((room.x, room.y + i), (room.x - 1, room.y + i))).collect(),
        Direction::Right => (0..room.h).map(|i| ((room.x + room.w - 1, room.y + i), (room.x + room.w, room.y + i))).collect(),
    }
}

/// Stretches of a side that face open space. A tile blocked by another room
/// breaks the stretch, because a door there would not be an airlock.
fn open_runs(cells: &[Edge], owner: &HashMap<Tile, usize>) -> Vec<Vec<Edge>> {
    let mut runs = Vec::new();
    let mut current = Vec::new();
    for cell in cells {
        if owner.contains_key(&cell.1) {
            if !current.is_empty() {
                runs.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(*cell);
    }
    if !current.is_empty() {
        runs.push(current);
    }
    runs
}

/// Doors the game puts into a wall between two rooms, by the length of that wall
/// in tiles, in thousandths, measured on the 28 player layouts by
/// `probe_door_stats`: 12% of one tile walls have no door, 22% of two tile walls
/// have none, 27% of them have two. Printed next to what these layouts made.
const WALL_DOOR_ODDS: &[(i32, &[usize])] = &[(1, &[120, 880]), (2, &[220, 520, 270])];

/// Airlocks on an open run of a room side as the shipped layouts measure it, in
/// thousandths, lone airlocks on a two tile side included: `d` is inner hull deck
/// that no room uses, `s` is outside the ship, `m` is both. The generator folds
/// the odd singles into the pair, so this is only used to print the comparison.
const SHIPPED_AIRLOCK_ODDS: &[(i32, char, &[&str], &[usize])] = &[
    (1, 'd', &[".", "A"], &[966, 34]),
    (1, 's', &[".", "A"], &[940, 60]),
    (2, 'd', &["..", "AA", ".A", "A."], &[884, 107, 9, 0]),
    (2, 's', &["..", "AA", "A.", ".A"], &[770, 160, 40, 30]),
    (2, 'm', &["..", "AA", "A.", ".A"], &[740, 190, 50, 20]),
];

// ------------------------------------------------------------------ render --

/// Room fills, walls, doors and room numbers over the base image.
fn draw_ship(ship: &Ship, rooms: &[Room], doors: &[Door]) -> RgbaImage {
    let mut canvas = clear_behind(&ship.base);

    for (index, room) in rooms.iter().enumerate() {
        let fill = PALETTE[index % PALETTE.len()];
        for tile in room.tiles() {
            let (x0, y0) = (ship.origin.0 + tile.0 * TILE, ship.origin.1 + tile.1 * TILE);
            for y in y0..y0 + TILE {
                for x in x0..x0 + TILE {
                    blend(&mut canvas, x, y, fill, 110);
                }
            }
        }
        // Wall around the room, drawn on the tile lines.
        let (x0, y0) = (ship.origin.0 + room.x * TILE, ship.origin.1 + room.y * TILE);
        let (x1, y1) = (x0 + room.w * TILE - 1, y0 + room.h * TILE - 1);
        for (x, y) in frame(x0, y0, x1, y1) {
            blend(&mut canvas, x, y, [250, 250, 250, 255], 255);
        }
        let number = (index + 1).to_string();
        draw_text(&mut canvas, x0 + 3, y0 + 3, &number, [255, 255, 255, 255]);
    }

    // Doors. A door between two rooms is yellow, an airlock is cyan, so an
    // opening to open space stands out.
    for door in doors {
        let (_, two) = door.tiles();
        let corner = if door.vertical {
            (ship.origin.0 + two.0 * TILE, ship.origin.1 + two.1 * TILE + TILE / 2)
        } else {
            (ship.origin.0 + two.0 * TILE + TILE / 2, ship.origin.1 + two.1 * TILE)
        };
        let colour = if door.airlock() { [60, 220, 255, 255] } else { [255, 214, 0, 255] };
        for step in -9..=9 {
            for thick in -2..=2 {
                let (px, py) = if door.vertical {
                    (corner.0 + thick, corner.1 + step)
                } else {
                    (corner.0 + step, corner.1 + thick)
                };
                blend(&mut canvas, px, py, colour, 255);
            }
        }
    }

    canvas
}

/// Dark grey behind the ship so the transparent hull edges stay visible.
fn clear_behind(base: &RgbaImage) -> RgbaImage {
    let back = [46u8, 48, 54];
    let mut canvas = RgbaImage::from_pixel(base.width(), base.height(), Rgba([46, 48, 54, 255]));
    for (x, y, pixel) in base.enumerate_pixels() {
        let alpha = pixel[3];
        let mix = |a: u8, b: u8| (a as u16 * (255 - alpha as u16) / 255 + b as u16 * alpha as u16 / 255) as u8;
        canvas.put_pixel(x, y, Rgba([mix(back[0], pixel[0]), mix(back[1], pixel[1]), mix(back[2], pixel[2]), 255]));
    }
    canvas
}

const PALETTE: [[u8; 4]; 8] = [
    [214, 40, 214, 255],
    [40, 190, 240, 255],
    [90, 220, 90, 255],
    [240, 150, 40, 255],
    [150, 90, 240, 255],
    [240, 90, 110, 255],
    [80, 220, 190, 255],
    [230, 220, 70, 255],
];

fn blend(canvas: &mut RgbaImage, x: i32, y: i32, colour: [u8; 4], alpha: u8) {
    if x < 0 || y < 0 || x >= canvas.width() as i32 || y >= canvas.height() as i32 {
        return;
    }
    let under = canvas.get_pixel(x as u32, y as u32).0;
    let mix = |a: u8, b: u8| (a as u16 * (255 - alpha as u16) / 255 + b as u16 * alpha as u16 / 255) as u8;
    canvas.put_pixel(x as u32, y as u32, Rgba([mix(under[0], colour[0]), mix(under[1], colour[1]), mix(under[2], colour[2]), 255]));
}

/// Pixels of a 1 pixel rectangle outline.
fn frame(x0: i32, y0: i32, x1: i32, y1: i32) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    for x in x0..=x1 {
        out.push((x, y0));
        out.push((x, y1));
    }
    for y in y0..=y1 {
        out.push((x0, y));
        out.push((x1, y));
    }
    out
}

// ------------------------------------------------------------ tiny numbers --

/// 3 x 5 pixel digits, one bit per pixel, most significant bit first.
const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b010, 0b010, 0b010],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
];

/// Draw digits with a dark shadow so they read on any room colour.
fn draw_text(canvas: &mut RgbaImage, x: i32, y: i32, text: &str, colour: [u8; 4]) {
    let mut cursor = x;
    for ch in text.chars() {
        let Some(digit) = ch.to_digit(10) else { continue };
        let rows = DIGITS[digit as usize];
        for (row, bits) in rows.iter().enumerate() {
            for column in 0..3 {
                if bits & (0b100 >> column) != 0 {
                    blend(canvas, cursor + column + 1, y + row as i32 + 1, [0, 0, 0, 255], 200);
                    blend(canvas, cursor + column, y + row as i32, colour, 255);
                }
            }
        }
        cursor += 4;
    }
}

// ------------------------------------------------------------------- sheet --

/// All ships in one picture, numbered in the order the table printed them.
fn compose_sheet(drawn: &[(String, RgbaImage)]) -> RgbaImage {
    let cell_w = drawn.iter().map(|(_, i)| i.width()).max().unwrap_or(0) + CELL_MARGIN * 2;
    let cell_h = drawn.iter().map(|(_, i)| i.height()).max().unwrap_or(0) + CELL_MARGIN * 2 + 16;
    let rows = drawn.len().div_ceil(SHEET_COLUMNS);
    let mut sheet = RgbaImage::from_pixel(cell_w * SHEET_COLUMNS as u32, cell_h * rows as u32, Rgba([24, 24, 28, 255]));

    for (index, (_, image)) in drawn.iter().enumerate() {
        let column = (index % SHEET_COLUMNS) as u32;
        let row = (index / SHEET_COLUMNS) as u32;
        // Ships sit in the middle of their cell, under the cell number.
        let x0 = column * cell_w + (cell_w - image.width()) / 2;
        let y0 = row * cell_h + CELL_MARGIN + 16 + (cell_h - CELL_MARGIN * 2 - 16 - image.height()) / 2;
        for (dx, dy, pixel) in image.enumerate_pixels() {
            sheet.put_pixel(x0 + dx, y0 + dy, *pixel);
        }
        draw_text(
            &mut sheet,
            x0 as i32,
            (row * cell_h + 4) as i32,
            &(index + 1).to_string(),
            [255, 255, 255, 255],
        );
    }
    sheet
}