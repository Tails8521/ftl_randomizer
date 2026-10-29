//! Checks the shuffled enemy layouts of a run against the layouts the game ships.
//!
//! Every layout the pass rewrote is read back and held against the shipped one: the
//! same rooms of the same shapes in the same order, inside the hull, none on top of
//! another, the layout still beginning on the two lines it began on, the clusters of
//! rooms the shipped doors joined still joined and the ones it sealed still sealed,
//! the gun pods of a boss still in their left to right order and clear of one
//! another, and every door lying in a wall the two rooms it names really share.
//!
//! ```text
//! cargo run --release --bin probe_enemy_rooms -- [SEED] [--out DIR] [LAYOUT ...]
//! ```
//!
//! Probes the archive. `--out DIR` writes the shuffled `data/<layout>.txt` files
//! there, which is what `temp/render_enemy.py` draws from.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;

use ftl_randomizer::blueprints;
use ftl_randomizer::enemy_rooms;
use ftl_randomizer::layout::{self, Door, Room, Tile};
use ftl_randomizer::rng::Rng;
use ftl_randomizer::ship_txt::ShipTxt;
use ftl_randomizer::{rooms, FtlArchive};

/// One `<shipBlueprint>` of the archive, as far as this probe cares.
struct Ship {
    layout: String,
    /// The art the ship is drawn with.
    img: Option<String>,
    /// The room of each gun pod of a boss, by the place of the pod in the boss's gun
    /// list.
    pods: BTreeMap<usize, usize>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut argv = std::env::args().skip(1).peekable();
    let mut out: Option<std::path::PathBuf> = None;
    let mut seed = 1;
    let mut first = true;
    let mut only: Vec<String> = Vec::new();
    while let Some(word) = argv.next() {
        match word.as_str() {
            "--out" => out = Some(std::path::PathBuf::from(argv.next().ok_or("--out needs a dir")?)),
            "--help" | "-h" => {
                println!("usage: probe_enemy_rooms [SEED] [--out DIR] [LAYOUT ...]");
                return Ok(());
            }
            other => {
                if first && other.parse::<u64>().is_ok() {
                    seed = other.parse()?;
                } else {
                    only.push(other.to_string());
                }
            }
        }
        first = false;
    }

    let mut raw = Vec::new();
    std::fs::File::open("data/ftl.dat")?.read_to_end(&mut raw)?;
    let stock = FtlArchive::from_bytes(&raw)?;
    let filed = ships(&stock)?;

    // What the pass has to keep, read before it is let near the archive.
    let mut shipped: BTreeMap<String, ShipTxt> = BTreeMap::new();
    let mut masks: BTreeMap<String, BTreeSet<Tile>> = BTreeMap::new();
    for path in stock.paths().map(str::to_string).collect::<Vec<_>>() {
        let Some(name) = path.strip_prefix("data/").and_then(|file| file.strip_suffix(".txt")) else {
            continue;
        };
        // A player hull files art of its own in `img/ship` and is not this pass's.
        if stock.get_file(&format!("img/ship/{name}_base.png")).is_some() {
            continue;
        }
        let Some(bytes) = stock.get_file(&path) else { continue };
        let Ok(txt) = ShipTxt::parse(bytes.to_vec()) else { continue };
        let xml = stock
            .get_file(&format!("data/{name}.xml"))
            .ok_or(format!("data/{name}.xml is not in the archive"))?;
        match hull(&stock, name, xml, &txt, &filed) {
            Ok(mask) => {
                masks.insert(name.to_string(), mask);
            }
            Err(why) => println!("{name}: no hull to measure against: {why}"),
        }
        shipped.insert(name.to_string(), txt);
    }

    let mut archive = FtlArchive::from_bytes(&raw)?;
    let drawn = enemy_rooms::randomize(&mut archive, &mut Rng::new(seed))?;
    if let Some(dir) = &out {
        std::fs::create_dir_all(dir)?;
        for ship in &drawn.ships {
            let name = format!("data/{}.txt", ship.layout);
            let bytes = archive.get_file(&name).expect("the pass wrote it");
            std::fs::write(dir.join(format!("{}.txt", ship.layout)), bytes)?;
        }
    }

    println!("seed {seed}");
    println!(
        "{:<22}{:>6}{:>6}{:>6}{:>6}{:>7}{:>7}  art",
        "layout", "rooms", "hull", "clust", "doors", "moved", "airloc"
    );
    for ship in &drawn.ships {
        if !only.is_empty() && !only.iter().any(|want| want == &ship.layout) {
            continue;
        }
        println!(
            "{:<22}{:>6}{:>6}{:>6}{:>6}{:>7}{:>7}  {}",
            ship.layout,
            ship.rooms,
            ship.hull,
            ship.clusters,
            ship.doors,
            ship.moved,
            airlocks(&archive, &ship.layout),
            ship.art.join("+"),
        );
    }
    for warning in &drawn.warnings {
        println!("warning: {warning}");
    }

    // Then every layout is read back and checked against the shipped one.
    let mut faults = 0;
    for ship in &drawn.ships {
        let (Some(before), Some(mask)) = (shipped.get(&ship.layout), masks.get(&ship.layout)) else {
            continue;
        };
        let Some(bytes) = archive.get_file(&format!("data/{}.txt", ship.layout)) else {
            continue;
        };
        let after = ShipTxt::parse(bytes.to_vec())?;
        let pods: BTreeMap<usize, usize> = filed
            .iter()
            .filter(|filed| filed.layout == ship.layout)
            .flat_map(|filed| filed.pods.clone())
            .collect();
        for fault in faults_of(before, &after, mask, &pods) {
            faults += 1;
            println!("{}: {fault}", ship.layout);
        }
    }
    println!(
        "\n{} layouts shuffled, {} of them moved ({} rooms of {}), {} faults",
        drawn.ships.len(),
        drawn.ships.iter().filter(|ship| ship.moved > 0).count(),
        drawn.ships.iter().map(|ship| ship.moved).sum::<usize>(),
        drawn.ships.iter().map(|ship| ship.rooms).sum::<usize>(),
        faults,
    );
    Ok(())
}

/// Every `<shipBlueprint>` of the archive, in path order.
fn ships(archive: &FtlArchive) -> Result<Vec<Ship>, String> {
    let mut out = Vec::new();
    for path in archive.paths().map(str::to_string).collect::<Vec<_>>() {
        if !path.ends_with(".xml") {
            continue;
        }
        let bytes = archive.get_file(&path).expect("path came from the archive");
        if !holds(bytes, b"<shipBlueprint") {
            continue;
        }
        let text = String::from_utf8_lossy(bytes).into_owned();
        for element in blueprints::elements(&text, 0)? {
            if element.name != "shipBlueprint" {
                continue;
            }
            let tag = &text[element.tag.clone()];
            let Some((_, layout)) = blueprints::attribute(tag, "layout") else { continue };
            let img = blueprints::attribute(tag, "img").map(|(_, value)| value.to_string());
            let mut pods = BTreeMap::new();
            let inside = blueprints::system_tags(&text[element.span.clone()], element.span.start)?;
            for system in inside {
                let system = &text[system.tag.clone()];
                let rank = blueprints::attribute(system, "weapon")
                    .and_then(|(_, value)| value.strip_prefix("ARTILLERY_BOSS_"))
                    .and_then(|value| value.parse::<usize>().ok());
                let room = blueprints::attribute(system, "room")
                    .and_then(|(_, value)| value.parse::<usize>().ok());
                if let (Some(rank), Some(room)) = (rank, room) {
                    pods.insert(rank - 1, room);
                }
            }
            out.push(Ship { layout: layout.to_string(), img, pods });
        }
    }
    Ok(out)
}

/// Tiles a room of the layout may stand on, measured the way the pass measures it:
/// the tiles all the hull art of the ship paints, plus the tiles its own rooms stand
/// on. The paint rule is the pass's own [`enemy_rooms::PAINT`].
fn hull(
    archive: &FtlArchive,
    name: &str,
    xml: &[u8],
    txt: &ShipTxt,
    filed: &[Ship],
) -> Result<BTreeSet<Tile>, String> {
    let offsets = rooms::art_offsets(xml)?;
    let mut painted: Option<BTreeSet<Tile>> = None;
    for art in art_names(name, filed) {
        let path = format!("img/ships_noglow/{art}_base.png");
        let Some(bytes) = archive.get_file(&path) else { continue };
        let image = image::load_from_memory(bytes).map_err(|error| error.to_string())?;
        let art = image.into_rgba8();
        let origin = layout::art_origin(offsets.img, offsets.size, art.dimensions());
        let tiles = layout::hull_paint(&art, origin, enemy_rooms::PAINT);
        painted = Some(match painted {
            Some(had) => had.intersection(&tiles).copied().collect(),
            None => tiles,
        });
    }
    let mut mask = painted.ok_or("no hull art of its own")?;
    mask.extend(txt.rooms.iter().flat_map(Room::tiles));
    Ok(mask)
}

/// The art names of the blueprints that fly a layout, or the layout's own name when
/// none of them names one.
fn art_names(name: &str, filed: &[Ship]) -> Vec<String> {
    let mut arts: BTreeSet<String> = filed
        .iter()
        .filter(|ship| ship.layout == name)
        .filter_map(|ship| ship.img.clone())
        .collect();
    if arts.is_empty() {
        arts.insert(name.to_string());
    }
    arts.into_iter().collect()
}

/// What a shuffled layout gets wrong, empty when it is sound.
fn faults_of(
    before: &ShipTxt,
    after: &ShipTxt,
    mask: &BTreeSet<Tile>,
    pods: &BTreeMap<usize, usize>,
) -> Vec<String> {
    let mut faults = Vec::new();
    let (was, now) = (&before.rooms, &after.rooms);

    if was.len() != now.len() {
        faults.push(format!(
            "has {} rooms, the shipped layout has {}",
            now.len(),
            was.len()
        ));
        return faults;
    }
    for (at, (room, was)) in now.iter().zip(was).enumerate() {
        if (room.w, room.h) != (was.w, was.h) {
            faults.push(format!(
                "room {at} is {}x{}, it was {}x{}",
                room.w, room.h, was.w, was.h
            ));
        }
        for tile in room.tiles() {
            if !mask.contains(&tile) {
                faults.push(format!("room {at} stands on {tile:?}, outside the hull"));
            }
        }
    }
    for (one, a) in now.iter().enumerate() {
        for (two, b) in now.iter().enumerate().skip(one + 1) {
            if a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h {
                faults.push(format!("rooms {one} and {two} stand on one another"));
            }
        }
    }

    // The box the rooms fill has to begin where the shipped one did, or the hull art
    // slides off the ship.
    for (axis, line) in [(0, "left"), (1, "top")] {
        let low = |rooms: &[Room]| {
            rooms
                .iter()
                .map(|room| if axis == 0 { room.x } else { room.y })
                .min()
                .unwrap()
        };
        if low(now) != low(was) {
            faults.push(format!("its {line} line moved from {} to {}", low(was), low(now)));
        }
    }

    let (was_groups, now_groups) = (groups(was, &before.doors), groups(now, &after.doors));
    if was_groups != now_groups {
        faults.push(format!(
            "clusters its rooms differently: shipped {was_groups:?}, now {now_groups:?}"
        ));
    }
    let cluster_of = |room: usize| {
        was_groups
            .iter()
            .position(|cluster| cluster.contains(&room))
    };

    for (at, door) in after.doors.iter().enumerate() {
        let (first, second) = door.tiles();
        let holds = |room: &Room, tile: Tile| room.tiles().any(|held| held == tile);
        if door.airlock() {
            let number = if door.left_top == -1 { door.right_bottom } else { door.left_top };
            let Some(room) = usize::try_from(number).ok().and_then(|at| now.get(at)) else {
                faults.push(format!("airlock {at} names room {number} of {} rooms", now.len()));
                continue;
            };
            let taken = |tile: Tile| now.iter().any(|other| holds(other, tile));
            if holds(room, first) && holds(room, second) {
                faults.push(format!("airlock {at} lies inside its own room"));
            } else if !holds(room, first) && !holds(room, second) {
                faults.push(format!("airlock {at} is not on the wall of room {number}"));
            } else if taken(if holds(room, first) { second } else { first }) {
                faults.push(format!("airlock {at} faces a room, not open space"));
            }
            continue;
        }
        let (one, two) = (door.left_top as usize, door.right_bottom as usize);
        let (left, right) = match (now.get(one), now.get(two)) {
            (Some(left), Some(right)) => (left, right),
            _ => {
                faults.push(format!("door {at} names rooms out of the layout"));
                continue;
            }
        };
        if !holds(left, first) || !holds(right, second) {
            faults.push(format!(
                "door {at} lies between {first:?} and {second:?}, which are not rooms \
                 {one} and {two}"
            ));
        }
        if cluster_of(one) != cluster_of(two) {
            faults.push(format!("door {at} joins two clusters of rooms"));
        }
    }

    // The gun pods have to keep the left to right order they ship in, and stay clear
    // of one another wherever two of them were clear to begin with.
    let mut columns: Vec<(i32, i32)> = pods
        .values()
        .filter_map(|room| Some((was.get(*room)?.x, now.get(*room)?.x)))
        .collect();
    columns.sort_unstable();
    if columns.len() != pods.len() {
        faults.push(format!("{} gun pod rooms are gone", pods.len() - columns.len()));
    }
    for pair in columns.windows(2) {
        if pair[0].0 != pair[1].0 && pair[0].1 >= pair[1].1 {
            faults.push(format!(
                "gun pods left the order they ship in, or stand on one column: {columns:?}"
            ));
            break;
        }
    }
    faults
}

/// The clusters of rooms a door list joins, smallest room id first in each of them.
fn groups(rooms: &[Room], doors: &[Door]) -> Vec<Vec<usize>> {
    let mut near: Vec<Vec<usize>> = vec![Vec::new(); rooms.len()];
    for door in doors.iter().filter(|door| !door.airlock()) {
        let (one, two) = (door.left_top as usize, door.right_bottom as usize);
        if one >= rooms.len() || two >= rooms.len() {
            continue;
        }
        near[one].push(two);
        near[two].push(one);
    }
    let mut seen = vec![false; rooms.len()];
    let mut out = Vec::new();
    for start in 0..rooms.len() {
        if seen[start] {
            continue;
        }
        let mut cluster = Vec::new();
        let mut queue = vec![start];
        seen[start] = true;
        while let Some(at) = queue.pop() {
            cluster.push(at);
            for &other in &near[at] {
                if !seen[other] {
                    seen[other] = true;
                    queue.push(other);
                }
            }
        }
        cluster.sort_unstable();
        out.push(cluster);
    }
    out.sort();
    out
}

/// How many airlocks a layout files.
fn airlocks(archive: &FtlArchive, name: &str) -> usize {
    archive.get_file(&format!("data/{name}.txt")).map_or(0, |bytes| {
        ShipTxt::parse(bytes.to_vec())
            .map_or(0, |txt| txt.doors.iter().filter(|door| door.airlock()).count())
    })
}

/// true when `hay` holds `needle`.
fn holds(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|window| window == needle)
}