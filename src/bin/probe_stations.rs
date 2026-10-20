//! Probe: how the ships the game ships with fill their system rooms.
//!
//! The randomizer gives every system a room of its own, and the 30 player
//! blueprints are the only sample of what a system room may look like. This reads
//! their rooms, doors, `<slot>`s and `<img>`s, and prints, per kind of system:
//!
//! * the shapes of room the kind is given (1x2, 2x1, 2x2),
//! * the tile of the room the station stands on and the wall it faces, which with
//!   the shape make the room's *configuration*,
//! * what stands behind the wall the station faces, and whether a door sits in it,
//! * the art the kind draws, since the art of a system is cut for the rooms it was
//!   drawn in,
//! * every configuration as a `const`, which is the whitelist `src/blueprints.rs`
//!   enforces.
//!
//! A blueprint files a `<slot>` for some systems only, and the game then falls back
//! on a default. `ship_modding.txt` names those defaults and they are counted here
//! as if they had been filed, marked `d` - the station the player sees is that
//! default, glitch or no glitch. Such a default can name a tile the room does not
//! have, which is a bug of the shipped data rather than a configuration to copy, so
//! those are counted apart and kept out of the whitelist.
//!
//! The medbay and the clone bay are one room holding one machine at a time, so both
//! are counted as `bay`.
//!
//! Run `cargo run --release --bin probe_stations [KIND] [--root DIR]`. With a kind -
//! `pilot` for instance - it prints one line per ship instead of one block per kind.
//! With `--root DIR` it reads `DIR/data` and `DIR/img/ship` rather than the shipped
//! `extracted`, which is how the result of a run is held against the ships the game
//! ships with.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::{env, fs, process};

use ftl_randomizer::blueprints::{self, Element};
use ftl_randomizer::layout::{Door, Room, Side, Tile};
use ftl_randomizer::ship_txt::ShipTxt;

/// Where the game's own files were unpacked to.
const ROOT: &str = "extracted";

/// The files that hold a `<shipBlueprint>`.
const FILES: [&str; 3] = ["blueprints.xml", "dlcBlueprints.xml", "dlcBlueprintsOverwrite.xml"];

/// The station the game draws when the blueprint filed none, from
/// `ship_modding.txt`. Doors, sensors and hacking are always filed by hand, so they
/// are missing here on purpose.
const DEFAULTS: [(&str, i32, Side); 4] = [
    ("pilot", 0, Side::Right),
    ("weapons", 1, Side::Up),
    ("shields", 0, Side::Left),
    ("engines", 2, Side::Down),
];

/// The tile the game blocks when a medbay files no `<slot>`.
const BAY_DEFAULT: i32 = 1;

/// The systems with a station, which is what a wall to face means anything for: the
/// six the crew stand at, and hacking, whose blueprint files a station too.
const STATIONED: [&str; 7] = ["pilot", "weapons", "shields", "engines", "doors", "sensors", "hacking"];

/// One configuration of a system room: the shape of the room, the tile of the
/// station or of the machine, and the wall a station faces. `side` is `None` for a
/// machine nobody stands at, and a bay blocks no tile with -2.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct Plan {
    w: i32,
    h: i32,
    number: i32,
    side: Option<Side>,
}

impl Plan {
    /// The same configuration as `src/blueprints.rs` writes it.
    fn lib(&self) -> blueprints::Plan {
        blueprints::Plan { w: self.w, h: self.h, number: self.number, side: self.side }
    }

    /// `2x1 1 right`, `2x2 -2 none`.
    fn label(&self) -> String {
        format!(
            "{}x{} {} {}",
            self.w,
            self.h,
            self.number,
            self.side.map(|side| side.name()).unwrap_or("none")
        )
    }

    /// true when the tile is not in a room of the plan's own shape, which leaves a
    /// station the player cannot reach.
    fn out_of_room(&self) -> bool {
        self.number >= self.w * self.h
    }
}

/// How often a configuration was seen: how many ships draw it, how many of those
/// filed the slot, and how many of those look through a door.
#[derive(Default)]
struct Seen {
    ships: usize,
    filed: usize,
    door: usize,
}

/// One system of one ship.
struct Record {
    ship: String,
    /// The system tag.
    name: String,
    /// The kind of room, with `medbay` and `clonebay` counted as `bay`.
    kind: String,
    img: String,
    /// The shape of the room the system sits in.
    shape: (i32, i32),
    /// The station or the blocked tile, when the kind has one.
    plan: Option<Plan>,
    /// The wall a station looks through: what stands behind it - a room id, `-1`
    /// for open space, `-2` for an airlock - and whether a door sits in that wall.
    wall: Option<(i32, bool)>,
    /// true when the blueprint wrote a `<number>` rather than leaving it to a
    /// default.
    filed: bool,
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
    let mut root = ROOT.to_string();
    let mut only: Option<String> = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => root = args.next().ok_or("--root needs a directory")?,
            other => only = Some(other.to_string()),
        }
    }
    let data = format!("{root}/data");
    let images = format!("{root}/img/ship");

    let mut records: Vec<Record> = Vec::new();
    let mut layouts: BTreeMap<String, (Vec<Room>, Vec<Door>)> = BTreeMap::new();
    let mut ships = 0;

    for file in FILES {
        let Ok(text) = fs::read_to_string(format!("{data}/{file}")) else {
            continue;
        };
        for ship in blueprints::blueprints(&text)? {
            // A player ship is a layout that owns a base image. A mod holds what
            // changed, and the hull art rarely does, so the shipped files answer for
            // a base image the mod leaves out.
            if !Path::new(&format!("{images}/{}_base.png", ship.layout)).exists()
                && !Path::new(&format!("{ROOT}/img/ship/{}_base.png", ship.layout)).exists()
            {
                continue;
            }
            if !layouts.contains_key(&ship.layout) {
                let bytes = fs::read(format!("{data}/{}.txt", ship.layout))?;
                let mut txt =
                    ShipTxt::parse(bytes).map_err(|why| format!("{}.txt: {why}", ship.layout))?;
                let doors = std::mem::take(&mut txt.doors);
                layouts.insert(ship.layout.clone(), (txt.rooms, doors));
            }
            ships += 1;
            let (rooms, doors) = &layouts[&ship.layout];
            let inside = &text[ship.span.clone()];
            // `0` as the base, so the ranges come back relative to `inside`, which
            // is what `read` indexes.
            for system in blueprints::system_tags(inside, 0)? {
                records.push(read(&ship.name, rooms, doors, inside, &system));
            }
        }
    }

    println!("{ships} player blueprints of {root}, {} systems", records.len());
    match only {
        Some(kind) => per_ship(&records, &kind),
        None => {
            summary(&records);
            whitelist(&records);
            art_map(&records);
            audit(&records, &root);
            disagreements(&records);
        }
    }
    Ok(())
}

/// One system of one ship, read out of the blueprint and the layout.
fn read(ship: &str, rooms: &[Room], doors: &[Door], text: &str, system: &Element) -> Record {
    let name = system.name.clone();
    let kind = kind(&name).to_string();
    let tag = &text[system.tag.start..system.tag.end];
    let room = blueprints::attribute(tag, "room").and_then(|(_, value)| value.parse::<usize>().ok());
    let img = blueprints::attribute(tag, "img")
        .map(|(_, value)| value.to_string())
        .unwrap_or_else(|| "-".to_string());
    let Some(where_) = room.and_then(|room| rooms.get(room)) else {
        // The system names a room the layout does not draw, which no shipped
        // blueprint does.
        return none(ship, name, kind, img, (0, 0), false);
    };

    let inside = &text[system.span.clone()];
    let number = between(inside, "number").and_then(|text| text.parse::<i32>().ok());
    let side = between(inside, "direction").and_then(|text| Side::parse(&text));
    let filed = number.is_some();

    let Some((number, side)) = station(&kind, number, side) else {
        // No station of its own and no default to fall back on: only the shape of
        // the room says anything about this system.
        return none(ship, name, kind, img, (where_.w, where_.h), filed);
    };
    let plan = Plan { w: where_.w, h: where_.h, number, side };
    let wall = side.filter(|_| !plan.out_of_room()).map(|side| behind(rooms, doors, where_, number, side));

    Record {
        ship: ship.to_string(),
        name,
        kind,
        img,
        shape: (where_.w, where_.h),
        plan: Some(plan),
        wall,
        filed,
    }
}

/// A record of a system with no station, with what the probe still knows of it.
fn none(
    ship: &str,
    name: String,
    kind: String,
    img: String,
    shape: (i32, i32),
    filed: bool,
) -> Record {
    Record { ship: ship.to_string(), name, kind, img, shape, plan: None, wall: None, filed }
}

/// The medbay and the clone bay are one room, so here they are one kind.
fn kind(name: &str) -> &str {
    if matches!(name, "medbay" | "clonebay") { "bay" } else { name }
}

/// The station the player sees: the one the blueprint filed, or the one the game
/// falls back on. `None` when the kind has neither.
fn station(kind: &str, number: Option<i32>, side: Option<Side>) -> Option<(i32, Option<Side>)> {
    if kind == "bay" {
        return Some((number.unwrap_or(BAY_DEFAULT), None));
    }
    let default = DEFAULTS.iter().find(|(name, _, _)| *name == kind);
    match (number, side) {
        (Some(number), Some(side)) => Some((number, Some(side))),
        (Some(number), None) => Some((number, default.map(|(_, _, side)| *side))),
        (None, Some(side)) => Some((default_number(kind)?, Some(side))),
        (None, None) => default.map(|(_, number, side)| (*number, Some(*side))),
    }
}

/// The tile the game falls back on for a system with no `<number>`.
fn default_number(kind: &str) -> Option<i32> {
    if kind == "bay" {
        return Some(BAY_DEFAULT);
    }
    DEFAULTS.iter().find(|(name, _, _)| *name == kind).map(|(_, number, _)| *number)
}

/// What a station looks through: the room beyond its wall, `-1` for space no room
/// took or `-2` for an airlock, and whether a door sits in that wall.
fn behind(rooms: &[Room], doors: &[Door], room: &Room, number: i32, side: Side) -> (i32, bool) {
    let step = match side {
        Side::Up => (0, -1),
        Side::Down => (0, 1),
        Side::Left => (-1, 0),
        Side::Right => (1, 0),
    };
    let tile = (room.x + number % room.w, room.y + number / room.w);
    let other = (tile.0 + step.0, tile.1 + step.1);
    let door = doors.iter().any(|door| door.separates(tile, other));
    let airlock = doors.iter().any(|door| door.airlock() && door.separates(tile, other));
    (if airlock { -2 } else { owner(rooms, other) }, door)
}

/// Which room owns a tile, `-1` when none does. A room id is its place in the list.
fn owner(rooms: &[Room], tile: Tile) -> i32 {
    rooms
        .iter()
        .position(|room| {
            (room.x..room.x + room.w).contains(&tile.0) && (room.y..room.y + room.h).contains(&tile.1)
        })
        .map_or(-1, |room| room as i32)
}

/// The text of a `<child>` of an element, trimmed. Simple on purpose: this is a
/// probe, and the reader the randomizer uses is checked by `probe_xml_edit`.
fn between(text: &str, child: &str) -> Option<String> {
    let open = text.find(&format!("<{child}>"))? + child.len() + 2;
    let close = open + text[open..].find(&format!("</{child}>"))?;
    Some(text[open..close].trim().to_string())
}

/// One block per kind of system: its shapes, its stations, their walls and their art.
fn summary(records: &[Record]) {
    println!("{}", "-".repeat(90));
    for kind in kinds(records) {
        let mine: Vec<&Record> = records.iter().filter(|record| record.kind == kind).collect();
        println!(
            "\n{kind}  {} systems, {} with a station or a machine",
            mine.len(),
            mine.iter().filter(|record| record.plan.is_some()).count()
        );

        let mut shapes: BTreeMap<(i32, i32), usize> = BTreeMap::new();
        for record in mine.iter() {
            *shapes.entry(record.shape).or_default() += 1;
        }
        line("rooms", list(shapes.iter().map(|((w, h), seen)| format!("{w}x{h} {seen}"))));

        let mut plans: BTreeMap<Plan, Seen> = BTreeMap::new();
        for record in mine.iter() {
            let Some(plan) = record.plan else { continue };
            let seen = plans.entry(plan).or_default();
            seen.ships += 1;
            seen.filed += usize::from(record.filed);
            seen.door += usize::from(record.wall.is_some_and(|(_, door)| door));
        }
        line(
            "stations",
            list(plans.iter().map(|(plan, seen)| {
                format!(
                    "{} x{}{}{}",
                    plan.label(),
                    seen.ships,
                    if seen.filed < seen.ships { " default" } else { "" },
                    if seen.door > 0 { " door" } else { "" }
                )
            })),
        );

        let unreachable: BTreeMap<String, usize> = plans
            .iter()
            .filter(|(plan, _)| plan.out_of_room())
            .map(|(plan, seen)| (format!("{} of a {}x{}", plan.number, plan.w, plan.h), seen.ships))
            .collect();
        if !unreachable.is_empty() {
            line("UNREACHABLE", list(unreachable.into_iter().map(|(what, seen)| format!("tile {what} {seen}"))));
        }

        let mut walls: BTreeMap<String, usize> = BTreeMap::new();
        for record in mine.iter().filter_map(|record| record.wall) {
            let (behind, door) = record;
            let what = match behind {
                -2 => "airlock",
                -1 => "open space",
                _ => "another room",
            };
            *walls.entry(format!("{what}{}", if door { " with a door" } else { "" })).or_default() += 1;
        }
        if !walls.is_empty() {
            line("behind", list(walls.iter().map(|(what, seen)| format!("{what} {seen}"))));
        }

        let mut art: BTreeMap<(&String, (i32, i32)), usize> = BTreeMap::new();
        for record in mine.iter() {
            *art.entry((&record.img, record.shape)).or_default() += 1;
        }
        line("art", list(art.iter().map(|((img, (w, h)), seen)| format!("{img} {w}x{h} {seen}"))));
    }
}

/// One labelled line of a block.
fn line(label: &str, body: String) {
    println!("  {label:11} {body}");
}

/// One line per ship, to look at one kind of system closely.
fn per_ship(records: &[Record], only: &str) {
    let only = kind(only);
    println!(
        "{:26} {:>6} {:>5} {:>7} {:>8} {:>12} {:>14}",
        "ship", "room", "tile", "wall", "slot", "behind", "art"
    );
    for record in records.iter().filter(|record| record.kind == only) {
        let (w, h) = record.shape;
        let plan = record.plan.unwrap_or(Plan { w, h, number: -9, side: None });
        let behind = match record.wall {
            Some((-2, door)) => format!("airlock{}", plus(door)),
            Some((-1, door)) => format!("open{}", plus(door)),
            Some((_, door)) => format!("room{}", plus(door)),
            None if plan.out_of_room() => "out of room".to_string(),
            None => "-".to_string(),
        };
        println!(
            "{:26} {:>6} {:>5} {:>7} {:>8} {:>12} {:>14}",
            record.ship,
            format!("{w}x{h}"),
            plan.number,
            plan.side.map(|side| side.name()).unwrap_or("-"),
            if record.filed { "filed" } else { "default" },
            behind,
            record.img,
        );
    }
}

/// `+door` when a door sits in the wall.
fn plus(door: bool) -> &'static str {
    if door { "+door" } else { "" }
}

/// Every configuration of every kind, as the list of `src/blueprints.rs` is written,
/// so the two can be read side by side. The unreachable ones are left out: they are
/// a bug of the shipped data, not a configuration to copy.
fn whitelist(records: &[Record]) {
    let found = kinds(records);
    println!("\nconfigurations these ships draw, as src/blueprints.rs writes its list");
    println!("const WHITELIST: [(&str, &[Plan]); {}] = [", found.len());
    for kind in found {
        let body = plans_of(records, kind)
            .into_iter()
            .map(|plan| match plan.side {
                Some(side) => {
                    format!("at({}, {}, {}, Side::{:?})", plan.w, plan.h, plan.number, side)
                }
                None => format!("machine({}, {}, {})", plan.w, plan.h, plan.number),
            })
            .collect::<Vec<_>>()
            .join(", ");
        println!("    (\"{kind}\", &[{body}]),");
    }
    println!("];");
}

/// The art of a room, and what it is filed with: the art each kind files per shape
/// of room, and the configuration every piece of art is ever filed with. Answers
/// whether the art alone says where the crew stand.
fn art_map(records: &[Record]) {
    println!("\nthe art of a room, and what it is filed with");
    let mut tuples: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    for kind in kinds(records) {
        let mut by_shape: BTreeMap<(i32, i32), BTreeMap<&str, usize>> = BTreeMap::new();
        for record in records.iter().filter(|record| record.kind == kind) {
            *by_shape.entry(record.shape).or_default().entry(record.img.as_str()).or_default() += 1;
        }
        let what = list(by_shape.iter().map(|((w, h), imgs)| {
            let art = list(imgs.iter().map(|(img, seen)| format!("{img} x{seen}")));
            format!("{w}x{h} [{art}]")
        }));
        println!("  {kind:9} {what}");

        for record in records.iter().filter(|record| record.kind == kind) {
            let Some(plan) = record.plan.filter(|plan| !plan.out_of_room()) else { continue };
            if record.img == "-" {
                continue;
            }
            tuples
                .entry(record.img.as_str())
                .or_default()
                .insert(format!("{} {}", record.kind, plan.label()));
        }
    }
    let many = list(tuples.iter().filter(|(_, seen)| seen.len() > 1).map(|(img, seen)| {
        format!("{img}: {}", list(seen.iter().cloned()))
    }));
    println!("  {} pieces of art are filed, of which:", tuples.len());
    println!(
        "  art filed with more than one configuration: {}",
        if many.is_empty() { "none" } else { &many }
    );
}

/// The shape of room a system image was cut for. The art of a room is 35 pixels to
/// a tile - `room_pilot_2.png` is 70x35, art for a 2x1 room - so the size of the
/// file says what shape it fits. `None` when there is no file to read.
///
/// The art of a mod is only read from the mod when the mod carries it, so the
/// shipped files are looked at too: a mod holds what changed, and room art rarely
/// does.
fn art_shape(root: &str, img: &str) -> Option<(i32, i32)> {
    let path = |dir: &str| format!("{dir}/img/ship/interior/{img}.png");
    let root = if Path::new(&path(root)).exists() { root } else { ROOT };
    let bytes = fs::read(path(root)).ok()?;
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let tile = |at: usize| -> Option<i32> {
        let four: [u8; 4] = bytes[at..at + 4].try_into().ok()?;
        i32::try_from(u32::from_be_bytes(four) / 35).ok()
    };
    let shape = (tile(16)?, tile(20)?);
    (shape.0 > 0 && shape.1 > 0).then_some(shape)
}

/// The configurations one kind draws, shortest first, each named once.
fn plans_of(records: &[Record], kind: &str) -> Vec<Plan> {
    records
        .iter()
        .filter(|record| record.kind == kind)
        .filter_map(|record| record.plan)
        .filter(|plan| !plan.out_of_room())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// The list of `src/blueprints.rs` held against these ships: what they draw that the
/// list does not hold, and what the list holds that nothing draws.
///
/// Two rules of the randomizer drop configurations on purpose: a station never faces
/// into its own room, which is what the hacking system of the Circle C files, and a
/// bay of a thin room blocks no tile, while six shipped bays block one.
fn audit(records: &[Record], root: &str) {
    println!("\nthe list of src/blueprints.rs held against these ships");
    let mut dropped = 0;
    for kind in kinds(records) {
        let listed: Vec<blueprints::Plan> = blueprints::WHITELIST
            .iter()
            .find(|(name, _)| *name == kind)
            .map(|(_, plans)| plans.to_vec())
            .unwrap_or_default();
        let mut off: BTreeMap<Plan, usize> = BTreeMap::new();
        for plan in plans_of(records, kind) {
            if !listed.contains(&plan.lib()) {
                *off.entry(plan).or_default() += 1;
            }
        }
        if off.is_empty() {
            continue;
        }
        dropped += off.len();
        let what = list(
            off.iter().map(|(plan, drawn)| format!("{} x{}", plan.label(), drawn)),
        );
        println!("  {kind} is drawn as {what}, which is not on the list");
    }
    if dropped == 0 {
        println!("  nothing is drawn off the list");
    }

    let unused = list(blueprints::WHITELIST.iter().filter_map(|(kind, listed)| {
        let left: Vec<String> = listed
            .iter()
            .filter(|plan| {
                !records.iter().filter_map(|r| r.plan).any(|seen| seen.lib() == **plan)
            })
            .map(|plan| format!("{}x{} {}", plan.w, plan.h, plan.number))
            .collect();
        (!left.is_empty()).then(|| format!("{kind}: {}", list(left.into_iter())))
    }));
    println!(
        "  on the list but drawn by no ship: {}",
        if unused.is_empty() {
            "none"
        } else {
            &unused
        }
    );

    let mut shapes: BTreeMap<&String, BTreeSet<(i32, i32)>> = BTreeMap::new();
    for record in records.iter() {
        shapes.entry(&record.img).or_default().insert(record.shape);
    }

    // The art of a room is cut for its shape, so a system moved into a room of
    // another shape is drawn with art that does not fit the room.
    let cut = records
        .iter()
        .filter(|record| art_shape(root, &record.img).is_some_and(|shape| shape == record.shape))
        .count();
    let drawn = records.iter().filter(|record| art_shape(root, &record.img).is_some()).count();
    let unfit = list(records.iter().filter_map(|record| {
        let cut = art_shape(root, &record.img)?;
        (cut != record.shape)
            .then(|| format!("{} of {} in a {}x{}", record.img, record.kind, record.shape.0, record.shape.1))
    }));
    println!("  art: {cut} of {drawn} rooms hold the art they were cut for");
    if !unfit.is_empty() {
        println!("  art cut for another shape: {unfit}");
    }
    let pure = shapes.values().filter(|shape| shape.len() == 1).count();
    println!("  art: {pure} of {} system imgs are drawn in one shape of room", shapes.len());
    let mixed = list(
        shapes
            .iter()
            .filter(|(_, shape)| shape.len() > 1)
            .map(|(img, shape)| {
                format!("{img} ({})", list(shape.iter().map(|(w, h)| format!("{w}x{h}"))))
            }),
    );
    if !mixed.is_empty() {
        println!("  drawn in more than one: {mixed}");
    }
}

/// The bay is one room, so its medbay and its clone bay should number it one way.
fn disagreements(records: &[Record]) {
    let mut of_ship: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for record in records.iter().filter(|record| record.kind == "bay") {
        if let Some(plan) = record.plan {
            of_ship.entry(record.ship.as_str()).or_default().push(format!(
                "{} blocks {}{}",
                record.name,
                plan.number,
                if record.filed { "" } else { " (default)" }
            ));
        }
    }
    println!("\nbay rooms whose two machines number their tile differently");
    let mut found = 0;
    for (ship, filed) in of_ship {
        let numbers: BTreeSet<String> = filed.iter().map(|line| line.split(' ').nth(2).unwrap_or("").to_string()).collect();
        if numbers.len() > 1 {
            found += 1;
            println!("  {ship}: {}", filed.join(", "));
        }
    }
    if found == 0 {
        println!("  none");
    }
}

/// The kinds of the records, systems with a station first.
fn kinds(records: &[Record]) -> Vec<&str> {
    let mut found: Vec<&str> = records.iter().map(|record| record.kind.as_str()).collect();
    found.sort();
    found.dedup();
    found.sort_by_key(|kind| (!STATIONED.contains(kind), *kind));
    found
}

/// `a, b, c` out of an iterator.
fn list(items: impl Iterator<Item = String>) -> String {
    items.collect::<Vec<_>>().join(", ")
}