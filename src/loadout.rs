//! What the player ships start with: systems, weapons, drones, crew and augments.
//!
//! [`crate::randomizer`] holds the numbers - the pools, the weights and the rules
//! that call a loadout playable - and knows nothing about the archive. This module
//! is its reader and its writer:
//!
//! * the pools come out of the game's own definitions - `<weaponBlueprint>`,
//!   `<droneBlueprint>` and `<augBlueprint>` - so what is drawn is what the data
//!   holds, and nothing invented here;
//! * what is drawn goes back into the `<shipBlueprint>` of each player ship: the
//!   `power` and `start` of every system, its `<weaponList>` and `<droneList>`, its
//!   `<crewCount>` lines, its `<aug>` lines, its `<maxPower>` and the `weapon` of
//!   its `<artillery>`.
//!
//! Every hull of four weapon slots carries a gun, and a gun needs two things the
//! game only ships on the three Fed hulls: an `<artillery>` tag in the system list,
//! standing in one of the spare rooms of the layout, and a fifth `<mount>` in the
//! `.xml` file of the layout, which is where the gun is drawn and from where it
//! fires. Neither comes from [`crate::rooms`], which knows nothing of what a
//! loadout asks for: the tag is filed by [`add_gun`] and the mount is put by
//! [`fit_gun`], at the nose of the hull, halfway between its two forward turrets.
//!
//! Like [`crate::blueprints`] nothing is re-serialized: every change is a byte
//! range of the file, so the comments, the spacing and the order of the blueprints
//! come back as they went in.
//!
//! The rooms are drawn by [`crate::rooms`] before this runs, and this changes
//! nothing about them: the gun of a hull the game ships without one is filed in a
//! room no other system of the hull was handed, of the three the layout has spare.
//! A player ship declares every system it can ever
//! have - the game draws a system badly when the blueprint of a player ship leaves
//! it out, and a ship can then never buy it - so a loadout only decides which of
//! the declared systems the ship starts with and at what level, and the room the
//! rooms pass chose for each of them stands.
//!
//! The two blueprints of the tutorial and the easy start are left alone, the same
//! two [`crate::blueprints::SKIP`] keeps out of the room rules: a player does not
//! pick them, and they declare no clone bay, no battery, no mind control and no
//! hacking, so they could not house what is drawn for them.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::ops::Range;

use crate::blueprints::{self, Blueprint, Element};
use crate::ship_txt::ShipTxt;
use crate::randomizer::{
    ALLOWED_STARTING_DRONES_NAMES, ALLOWED_STARTING_WEAPONS_NAMES, Augment, RandomizedShip,
    Randomizer, StartingDrone, StartingWeapon, WeaponType,
};
use crate::rng::Rng;
use crate::FtlArchive;

type Patch = (Range<usize>, String);

/// What went wrong with an archive.
#[derive(Debug)]
pub enum Error {
    /// A file that could not be read, or a definition this module cannot model.
    /// `why` says what was not as expected and where to look.
    Unreadable { path: String, why: String },
    /// The archive itself.
    Archive(crate::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unreadable { path, why } => {
                if path.is_empty() {
                    write!(out, "{why}")
                } else {
                    write!(out, "{path}: {why}")
                }
            }
            Error::Archive(error) => write!(out, "{error}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<crate::Error> for Error {
    fn from(error: crate::Error) -> Self {
        Error::Archive(error)
    }
}

type Result<T> = std::result::Result<T, Error>;

/// Every system a loadout has a level for, which is every system a player ship
/// declares. A system tag of another name is left exactly as the game filed it.
const SYSTEMS: [&str; 16] = [
    "pilot", "weapons", "shields", "engines", "oxygen", "drones", "teleporter", "cloaking",
    "medbay", "clonebay", "doors", "sensors", "mind", "hacking", "battery", "artillery",
];

/// The level a system the loadout does not start is left at: every system of a
/// shipped ship that starts uninstalled is filed at 1.
const UNINSTALLED: usize = 1;

/// One ship that was given a loadout.
#[derive(Debug)]
pub struct Ship {
    pub name: String,
    pub layout: String,
    /// The crew as race and number, in the order they were drawn.
    pub crew: Vec<(String, usize)>,
    pub weapons: Vec<String>,
    pub drones: Vec<String>,
    pub augments: Vec<String>,
    pub systems: Vec<(String, usize)>,
    pub reactor: usize,
    pub missiles: i8,
    pub drone_parts: i8,
    /// The gun of a hull that starts with one, empty on every other.
    pub artillery: String,
    /// The room the gun stands in, on the hulls that carry one.
    pub gun_room: Option<usize>,
}

impl Ship {
    /// The crew as `3 human 1 rock`.
    pub fn crew_text(&self) -> String {
        let held: Vec<String> = self
            .crew
            .iter()
            .map(|(race, number)| format!("{number} {race}"))
            .collect();
        held.join(", ")
    }
}

/// What a run changed.
pub struct Report {
    pub ships: Vec<Ship>,
    /// Archive paths that were written.
    pub changed: Vec<String>,
    /// Anything that could not be done cleanly.
    pub warnings: Vec<String>,
}

/// Give every player ship of `archive` a new loadout.
pub fn randomize(archive: &mut FtlArchive, rng: &mut Rng) -> Result<Report> {
    let randomizer = pools(archive)?;
    let mut ships = Vec::new();
    let mut changed = Vec::new();
    let mut warnings = Vec::new();
    // The layouts of the hulls that carry a gun of their own, which is one per
    // layout and not one per blueprint: the mount belongs to the hull.
    let mut guns: BTreeSet<String> = BTreeSet::new();

    for path in archive.paths().map(str::to_string).collect::<Vec<_>>() {
        if !path.starts_with("data/") || !path.ends_with(".xml") {
            continue;
        }
        let bytes = archive.get_file(&path).expect("path came from the archive");
        if !holds(bytes, b"<shipBlueprint") {
            continue;
        }
        let text = String::from_utf8(bytes.to_vec()).map_err(|error| Error::Unreadable {
            path: path.clone(),
            why: format!("not utf8: {error}"),
        })?;
        let found = blueprints::blueprints(&text).map_err(|why| Error::Unreadable {
            path: path.clone(),
            why,
        })?;

        let before = ships.len();
        let mut patches = Patches::default();
        for blueprint in found {
            // Only the ships a player picks, and only the ones whose rooms this
            // mod draws - the same test of a layout that `rooms::randomize` makes.
            if blueprints::SKIP.contains(&blueprint.name.as_str())
                || archive.get_file(&base_image(&blueprint.layout)).is_none()
            {
                continue;
            }
            ships.push(loadout(
                &text,
                &blueprint,
                &randomizer,
                archive,
                rng,
                &mut patches,
                &mut warnings,
                &mut guns,
            )?);
        }

        if ships.len() > before && !patches.is_empty() {
            archive.set_file(&path, blueprints::rewrite(&text, &patches.into_edits()));
            changed.push(path);
        }
    }

    // Then the hulls themselves: a gun is drawn at the fifth weapon mount of the
    // layout, and only the Fed hulls ship one.
    for layout in &guns {
        if fit_gun(archive, layout, &mut warnings) {
            changed.push(layout_xml(layout));
        }
    }

    Ok(Report {
        ships,
        changed,
        warnings,
    })
}

/// Draw one ship and note the edits that write it. `archive` is only read, for the
/// layout file the rooms of the hull were written in, and `guns` collects the
/// layouts that end up carrying a gun, so that their mount is written once the
/// blueprints are.
fn loadout(
    text: &str,
    blueprint: &Blueprint,
    randomizer: &Randomizer,
    archive: &FtlArchive,
    rng: &mut Rng,
    patches: &mut Patches,
    warnings: &mut Vec<String>,
    guns: &mut BTreeSet<String>,
) -> Result<Ship> {
    // One walk of the blueprint, and every range it hands back is in the file.
    let inside =
        blueprints::elements(&text[blueprint.span.clone()], blueprint.span.start).map_err(|why| {
            Error::Unreadable {
                path: blueprint.name.clone(),
                why,
            }
        })?;
    // The walk is rooted on the blueprint, so its children are one deep.
    let parts = below(&inside, 1);
    let systems = match parts.iter().find(|element| element.name == "systemList") {
        Some(list) => below(&inside, list.depth + 1)
            .into_iter()
            .filter(|element| {
                element.span.start > list.span.start && element.span.end < list.span.end
            })
            .collect(),
        None => Vec::new(),
    };
    // `<weaponSlots>4</weaponSlots>` numbers itself between its own two tags, and a
    // blueprint that files neither tag is read as a ship of no slots at all.
    let slots = |tag: &str| -> usize {
        parts
            .iter()
            .find(|element| element.name == tag)
            .and_then(|element| between(text, element))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    };
    let four_slots = slots("weaponSlots") >= 4;
    let drawn = randomizer.randomize_ship(rng, four_slots, slots("droneSlots") >= 3);

    // A hull of four weapon slots carries a gun. The three Fed hulls ship the
    // `<artillery>` tag and the mount that goes with it, at their nose; the rest
    // ship neither, so the tag is filed here, in a room no other system of the
    // ship was handed, and the mount is written into the layout by `fit_gun`.
    let shipped_gun = systems.iter().find(|element| element.name == "artillery");
    let mut gun_room = shipped_gun.and_then(|element| room_of(text, element));
    let mut declares_gun = shipped_gun.is_some();
    if four_slots && !declares_gun {
        gun_room = add_gun(
            text,
            &systems,
            &blueprint.layout,
            archive,
            &drawn,
            patches,
            warnings,
        );
        declares_gun = gun_room.is_some();
        if declares_gun {
            guns.insert(blueprint.layout.clone());
        }
    }

    // The systems first: a room and an art were chosen for each of them already,
    // so only `power` and `start` move here.
    for element in &systems {
        // A system this module has no level for is left exactly as the game filed
        // it: a player ship holds a few odd ones that are not on the map at all.
        if !SYSTEMS.contains(&element.name.as_str()) {
            continue;
        }
        let started = drawn.systems.iter().find(|(name, _)| *name == element.name);
        let (level, on) = match started {
            Some((_, level)) => (*level, true),
            None => (UNINSTALLED, false),
        };
        write_attribute(text, element, "power", &level.to_string(), patches);
        let state = if on { "true" } else { "false" };
        if !write_attribute(text, element, "start", state, patches) {
            add_attribute(text, element, "start", state, patches);
        }
        if element.name == "artillery"
            && on
            && !write_attribute(text, element, "weapon", drawn.artillery_weapon, patches)
        {
            add_attribute(text, element, "weapon", drawn.artillery_weapon, patches);
        }
    }
    // A player ship declares all sixteen. A system that is not declared cannot be
    // bought later and its room is drawn wrong, so it is said out loud, not fixed.
    // The gun is the one system a hull may be given here rather than declare.
    for (name, _) in &drawn.systems {
        if *name == "artillery" && declares_gun {
            continue;
        }
        if !systems.iter().any(|element| element.name == *name) {
            warnings.push(format!(
                "{} starts a {name}, which its blueprint does not declare: the ship can neither fit nor buy it",
                blueprint.name
            ));
        }
    }

    // Then the weapons and the drones, one list of each. The Fed hulls ship two
    // `<droneList>`s and a `count` that is not the number of entries in them, so
    // both are written to say the same thing.
    let weapons: Vec<String> = drawn.weapons.iter().map(|w| w.name.to_string()).collect();
    let drones: Vec<String> = drawn.drones.iter().map(|d| d.name.to_string()).collect();
    write_list(
        text,
        &parts,
        "weaponList",
        "weapon",
        &weapons,
        &[("missiles", &drawn.missiles.max(0).to_string())],
        patches,
    );
    write_list(
        text,
        &parts,
        "droneList",
        "drone",
        &drones,
        &[("drones", &drawn.drone_parts.max(0).to_string())],
        patches,
    );

    // Then the crew, one line per race, since the game reads a mixed ship as one
    // line of each.
    let mut crew: Vec<(String, usize)> = Vec::new();
    for race in &drawn.crew {
        match crew.iter_mut().find(|(held, _)| held == race) {
            Some((_, number)) => *number += 1,
            None => crew.push((race.to_string(), 1)),
        }
    }
    let lines: Vec<String> = crew
        .iter()
        .map(|(race, number)| format!("<crewCount amount = \"{number}\" class=\"{race}\"/>"))
        .collect();
    write_lines(text, &parts, "crewCount", &lines, patches);

    let augments: Vec<String> = drawn
        .augments
        .iter()
        .map(|name| format!("<aug name=\"{name}\"/>"))
        .collect();
    write_lines(text, &parts, "aug", &augments, patches);

    // Then the reactor: the loadout's own total, its weapon and drone power with
    // it, which is how `randomizer.rs` counts it.
    if let Some(power) = parts.iter().find(|element| element.name == "maxPower") {
        write_attribute(text, power, "amount", &drawn.reactor.to_string(), patches);
    }

    Ok(Ship {
        name: blueprint.name.clone(),
        layout: blueprint.layout.clone(),
        crew,
        weapons,
        drones,
        augments: drawn.augments.clone(),
        systems: drawn
            .systems
            .iter()
            .map(|(name, level)| (name.to_string(), *level))
            .collect(),
        reactor: drawn.reactor,
        missiles: drawn.missiles,
        drone_parts: drawn.drone_parts,
        // The draw picks a gun name as soon as it rolls for one, even when the
        // level it rolls with is 0, so the report follows the system list.
        artillery: match drawn.systems.iter().any(|(name, _)| *name == "artillery") {
            true => drawn.artillery_weapon.to_string(),
            false => String::new(),
        },
        gun_room,
    })
}

/// File an `<artillery>` in the `<systemList>` of a hull the game ships without
/// one, and hand back the room it was put in: a room of the layout no other system
/// of the ship stands in, the shape the art of a gun room was cut for when the
/// layout has one free.
///
/// The tag is filed for every hull of four weapon slots, whether the draw started
/// the gun or not: the room is taken either way, and a system the ship does not
/// start is filed `start="false"` like the twelve other systems of a shipped hull
/// that are. The `weapon` is named even then, since the game reads it off the
/// system when the ship is loaded.
fn add_gun(
    text: &str,
    systems: &[Element],
    layout: &str,
    archive: &FtlArchive,
    drawn: &RandomizedShip,
    patches: &mut Patches,
    warnings: &mut Vec<String>,
) -> Option<usize> {
    // The system that ends last rather than the one that reads last: the walk
    // hands back a system that files a `<slot>` only once its children came to an
    // end, so the two orders are not the order of the file.
    let Some(last) = systems.iter().max_by_key(|element| element.span.end) else {
        warnings.push(format!(
            "the {layout} hull has no system list to file a gun in"
        ));
        return None;
    };
    let taken: Vec<usize> = systems.iter().filter_map(|element| room_of(text, element)).collect();
    let room = free_room(archive, layout, &taken, warnings)?;
    // A gun the draw did not pick stands at the level of a system the ship does not
    // start, which is the same number as the lowest level one that it does: the
    // system list says which of the two it is, the level alone does not.
    let started = drawn.systems.iter().find(|(name, _)| *name == "artillery");
    let level = started.map_or(UNINSTALLED, |(_, level)| *level);
    let state = match started {
        Some(_) => "true",
        None => "false",
    };
    // The two guns the game ships; the draw leaves the name empty when it rolled no
    // gun at all, which is the common case of a hull that starts without one.
    let weapon = match drawn.artillery_weapon {
        "" => "ARTILLERY_FED",
        name => name,
    };
    let edge = blueprints::indent_before(text, last.span.start);
    let nl = newline(text);
    patches.insert(
        last.span.end,
        &format!("{nl}{edge}<artillery power=\"{level}\" room=\"{room}\" start=\"{state}\" weapon=\"{weapon}\"/>"),
    );
    Some(room)
}

/// A room of the layout that no system of the ship was filed in, `None` when the
/// hull has none left.
///
/// Every layout is drawn with the three spare rooms of `rooms::SPARE_ROOMS` over
/// what the systems of its ships need, so a free one is nearly always going. The
/// gun stands in a 2x1 room when one is free and in a 2x2 after that, which are the
/// two shapes the Fed hulls file their gun in - `room_artillery.png` measures 70x35
/// pixels, two tiles by one, and it is the only art of a gun room the game holds.
/// A hull whose spare rooms are all thin takes one of those: the art leans over its
/// wall by a tile, and a room the gun does not stand in at all is the worse of the
/// two.
fn free_room(
    archive: &FtlArchive,
    layout: &str,
    taken: &[usize],
    warnings: &mut Vec<String>,
) -> Option<usize> {
    let path = layout_txt(layout);
    let bytes = match archive.get_file(&path) {
        Some(bytes) => bytes,
        None => {
            warnings.push(format!("{path} is not in the archive, so the gun of the {layout} hull has no room to stand in"));
            return None;
        }
    };
    let rooms = match ShipTxt::parse(bytes.to_vec()) {
        Ok(txt) => txt.rooms,
        Err(why) => {
            warnings.push(format!("{path}: {why}, so the gun of the {layout} hull has no room to stand in"));
            return None;
        }
    };
    let free: Vec<usize> = (0..rooms.len()).filter(|id| !taken.contains(id)).collect();
    let shaped = |want: (i32, i32)| {
        free.iter()
            .copied()
            .find(|id| (rooms[*id].w, rooms[*id].h) == want)
    };
    let room = shaped((2, 1)).or_else(|| shaped((2, 2))).or_else(|| free.first().copied());
    if room.is_none() {
        warnings.push(format!(
            "every room of the {layout} hull stands under a system, so it gets no gun"
        ));
    }
    room
}

/// The room number a system tag files, `None` when it files none or one that is
/// not a number.
fn room_of(text: &str, element: &Element) -> Option<usize> {
    let (_, value) = blueprints::attribute(&text[element.tag.clone()], "room")?;
    value.parse().ok()
}

/// Give the layout of a hull that carries a gun the mount the game draws it from,
/// and report whether the file was written.
///
/// The game takes the fifth `<mount>` of the layout file for the gun and the four
/// before it for the weapon slots, and a ship whose file holds four mounts fires
/// its gun from beyond the left of the screen (`reference/ship_modding.txt`). Only
/// the three Fed hulls ship a fifth one, at their nose; the mount of the rest is
/// put halfway between the two that are furthest forward, which is on the hull
/// between the two turrets it leans on. It slides neither way, since the two it
/// sits between slide apart, and it turns like the first mount of the file and
/// rides off with the same gib.
fn fit_gun(archive: &mut FtlArchive, layout: &str, warnings: &mut Vec<String>) -> bool {
    let path = layout_xml(layout);
    let Some(bytes) = archive.get_file(&path) else {
        warnings.push(format!("{path} is not in the archive, so the {layout} gun has no mount to be written in"));
        return false;
    };
    let Ok(text) = String::from_utf8(bytes.to_vec()) else {
        warnings.push(format!("{path} is not utf8, so the mount of the {layout} gun stayed where it was"));
        return false;
    };
    let text = text.as_str();
    let inside = match blueprints::elements(text, 0) {
        Ok(inside) => inside,
        Err(why) => {
            warnings.push(format!("{path}: {why}"));
            return false;
        }
    };
    let Some(list) = inside.iter().find(|element| element.name == "weaponMounts") else {
        warnings.push(format!("{path} files no <weaponMounts>, so the {layout} gun has nowhere to be put"));
        return false;
    };
    let mounts: Vec<&Element> = inside
        .iter()
        .filter(|mount| {
            mount.name == "mount" && mount.span.start > list.span.start && mount.span.end < list.span.end
        })
        .collect();
    let point = |mount: &Element| -> Option<(i32, i32)> {
        let tag = &text[mount.tag.clone()];
        let number = |key: &str| {
            blueprints::attribute(tag, key).and_then(|(_, value)| value.parse::<i32>().ok())
        };
        Some((number("x")?, number("y")?))
    };
    let pairs: Vec<((i32, i32), (i32, i32))> = mounts
        .chunks(2)
        .filter_map(|pair| {
            Some((pair.first().copied().and_then(point)?, pair.get(1).copied().and_then(point)?))
        })
        .collect();
    // The gun belongs at the nose, and a hull faces right, so the pair to stand
    // between is the two weapon mounts furthest along x - which is not always the
    // pair the file numbers first: the three Rock hulls file their rear pair before
    // their forward one, and every other hull files the forward pair first.
    let sum = |pair: &((i32, i32), (i32, i32))| pair.0 .0 + pair.1 .0;
    let (first, second) = match pairs.first() {
        Some(near) => *pairs.get(1).filter(|far| sum(far) > sum(near)).unwrap_or(near),
        None => {
            warnings.push(format!(
                "{path} holds no pair of mounts that both number their x and their y, so the {layout} gun has nowhere to be put"
            ));
            return false;
        }
    };
    let middle = |a: i32, b: i32| ((a + b) as f64 / 2.0).round() as i32;
    let (x, y) = (middle(first.0, second.0), middle(first.1, second.1));

    let mut patches = Patches::default();
    match mounts.get(4) {
        // A fifth mount is already filed there: the Kestrel and the Crystal hulls
        // ship four that the game never reads, left in the files by the testing of
        // the gun. The number the game reads for the gun is moved to the nose, and
        // the rest of its tag is left as it was typed.
        Some(fifth) => {
            write_attribute(text, fifth, "x", &x.to_string(), &mut patches);
            write_attribute(text, fifth, "y", &y.to_string(), &mut patches);
        }
        None if mounts.len() < 4 => {
            warnings.push(format!(
                "{path} files {} mounts, which is not the four weapon slots of the {layout} hull plus a gun",
                mounts.len()
            ));
            return false;
        }
        None => {
            let last = mounts.last().expect("a mount was read off the file");
            let edge = blueprints::indent_before(text, last.span.start);
            let nl = newline(text);
            // The turn and the gib of the mount that leads the file: every weapon of
            // this hull turns the same way and rides off with the same wreck.
            let first_tag = &text[mounts[0].tag.clone()];
            let keep = |key: &str, fallback: &str| {
                blueprints::attribute(first_tag, key)
                    .map(|(_, value)| value.to_string())
                    .unwrap_or_else(|| fallback.to_string())
            };
            patches.insert(
                last.span.end,
                &format!(
                    "{nl}{edge}<mount x=\"{x}\" y=\"{y}\" rotate=\"{}\" mirror=\"false\" gib=\"{}\" slide=\"no\"/>",
                    keep("rotate", "true"),
                    keep("gib", "1")
                ),
            );
        }
    }
    let whole = blueprints::rewrite(text, &patches.into_edits());
    // The Kestrel of a file that already stands the gun where the middle of its
    // first two mounts is, and every hull that needed no edit at all, is not a file
    // the run changed.
    if whole == text.as_bytes() {
        return false;
    }
    archive.set_file(&path, whole);
    true
}

/// The elements of a walk at one depth, in the order the file holds them.
fn below(inside: &[Element], depth: usize) -> Vec<Element> {
    inside
        .iter()
        .filter(|element| element.depth == depth)
        .cloned()
        .collect()
}

/// The text an element holds between its own two tags, trimmed. An element that
/// closes itself holds nothing.
fn between(text: &str, element: &Element) -> Option<String> {
    let close = format!("</{}>", element.name);
    let end = element.span.end.checked_sub(close.len())?;
    if text[end..element.span.end] != close || end < element.tag.end {
        return None;
    }
    Some(text[element.tag.end..end].trim().to_string())
}

/// The value of an attribute of an element, replaced. False when the tag holds no
/// such attribute, which is when [`add_attribute`] has to write one.
fn write_attribute(
    text: &str,
    element: &Element,
    key: &str,
    value: &str,
    patches: &mut Patches,
) -> bool {
    let Some((range, _)) = blueprints::attribute(&text[element.tag.clone()], key) else {
        return false;
    };
    patches.replace(
        element.tag.start + range.start..element.tag.start + range.end,
        value,
    );
    true
}

/// An attribute a tag does not have, written behind one that it always has. The
/// blueprints keep `room` next to the room number, and that is where a new
/// attribute goes too; a tag that has none of the anchors - a `<droneList>` that
/// names only its drone parts - gets it behind the attributes it does have.
fn add_attribute(
    text: &str,
    element: &Element,
    key: &str,
    value: &str,
    patches: &mut Patches,
) -> bool {
    let tag = &text[element.tag.clone()];
    for anchor in ["room", "power", "name"] {
        if let Some((range, _)) = blueprints::attribute(tag, anchor) {
            let at = element.tag.start + range.end + 1;
            patches.insert(at, &format!(" {key}=\"{value}\""));
            return true;
        }
    }
    // Behind the last attribute of the tag, which is where `/>` or `>` stands.
    let at = match tag.ends_with("/>") {
        true => element.tag.end - 2,
        false => element.tag.end - 1,
    };
    patches.insert(at, &format!(" {key}=\"{value}\""));
    true
}

/// One `<weaponList>` or `<droneList>` of a blueprint: the first of the file
/// written whole, the copies the game shipped beside it taken out, and one added
/// where the ships that have none keep theirs when the file holds none at all.
fn write_list(
    text: &str,
    parts: &[Element],
    list: &str,
    child: &str,
    names: &[String],
    extra: &[(&str, &str)],
    patches: &mut Patches,
) {
    let held: Vec<&Element> = parts.iter().filter(|element| element.name == list).collect();
    let mut wanted: Vec<(&str, String)> = vec![("count", names.len().to_string())];
    wanted.extend(extra.iter().map(|(key, value)| (*key, value.to_string())));

    let Some(first) = held.first() else {
        // A hull the game ships with no list of the kind - six of them have no
        // `<droneList>` - gets one behind the list it does have, or behind the
        // slots when it has neither.
        let other = if list == "droneList" { "weaponList" } else { "droneList" };
        let Some(anchor) = parts
            .iter()
            .find(|element| element.name == other)
            .or_else(|| parts.iter().find(|element| element.name == "droneSlots"))
            .or_else(|| parts.iter().find(|element| element.name == "weaponSlots"))
        else {
            return;
        };
        let edge = blueprints::indent_before(text, anchor.span.start);
        let nl = newline(text);
        let mut attributes = String::new();
        for (key, value) in &wanted {
            attributes.push_str(&format!(" {key}=\"{value}\""));
        }
        let element = if names.is_empty() {
            format!("<{list}{attributes}/>")
        } else {
            let entries: String = names
                .iter()
                .map(|name| format!("{nl}{edge}\t<{child} name=\"{name}\"/>"))
                .collect();
            format!("<{list}{attributes}>{entries}{nl}{edge}</{list}>")
        };
        patches.insert(anchor.span.end, &format!("{nl}{edge}{element}"));
        return;
    };

    for (key, value) in &wanted {
        if !write_attribute(text, first, key, value, patches) {
            add_attribute(text, first, key, value, patches);
        }
    }
    set_children(text, first, child, names, patches);
    for copy in held.iter().skip(1) {
        delete(text, copy, patches);
    }
}

/// The children of an element replaced by one `<child name="X"/>` per name.
fn set_children(
    text: &str,
    element: &Element,
    child: &str,
    names: &[String],
    patches: &mut Patches,
) {
    let edge = blueprints::indent_before(text, element.span.start);
    let nl = newline(text);
    let entries: String = names
        .iter()
        .map(|name| format!("{nl}{edge}\t<{child} name=\"{name}\"/>"))
        .collect();
    // What stands between the two tags is replaced whole, so no blank line is
    // left where the old entries leaned.
    let content = format!("{entries}{nl}{edge}");

    if text[element.tag.clone()].ends_with("/>") {
        // Self closing: the `/>` becomes an open tag, the entries and a closing
        // tag, or stays as it is when there is nothing to hold.
        if !names.is_empty() {
            patches.replace(
                element.tag.end - 2..element.tag.end,
                format!(">{content}</{}>", element.name),
            );
        }
        return;
    }

    let close = format!("</{}>", element.name);
    if let Some(closing) = element.span.end.checked_sub(close.len()) {
        patches.replace(element.tag.end..closing, content);
    }
}

/// A line per entry where the elements of `name` stood: the first replaced by the
/// whole block, the rest taken out with the line they stand on, and the block
/// added before `<health>` - where both of these sit in the files that hold them
/// - when the ship has none of them at all.
fn write_lines(text: &str, parts: &[Element], name: &str, lines: &[String], patches: &mut Patches) {
    let held: Vec<&Element> = parts.iter().filter(|element| element.name == name).collect();
    let edge = match held.first() {
        Some(first) => blueprints::indent_before(text, first.span.start),
        None => match parts.iter().find(|element| element.name == "health") {
            Some(health) => blueprints::indent_before(text, health.span.start),
            None => return,
        },
    };
    let nl = newline(text);
    let block = lines.join(&format!("{nl}{edge}"));

    let Some(first) = held.first() else {
        if !lines.is_empty() {
            let health = parts
                .iter()
                .find(|element| element.name == "health")
                .expect("the edge came from that element");
            patches.insert(health.span.start, &format!("{block}{nl}{edge}"));
        }
        return;
    };

    // Nothing left to write: every line of the kind goes, with the line it stands
    // on, so a ship that starts with no crew or no augment says so by itself.
    if lines.is_empty() {
        for element in held {
            delete(text, element, patches);
        }
        return;
    }

    patches.replace(first.span.clone(), block);
    for copy in held.iter().skip(1) {
        delete(text, copy, patches);
    }
}

/// An element taken out with the line it stands on, so no blank line is left.
fn delete(text: &str, element: &Element, patches: &mut Patches) {
    let bytes = text.as_bytes();
    let mut from = element.span.start;
    while from > 0 && matches!(bytes[from - 1], b' ' | b'\t') {
        from -= 1;
    }
    if from > 0 && bytes[from - 1] != b'\n' {
        // It shares its line with something else, so only the element goes.
        from = element.span.start;
    }
    let mut to = element.span.end;
    if bytes.get(to) == Some(&b'\r') && bytes.get(to + 1) == Some(&b'\n') {
        to += 2;
    } else if bytes.get(to) == Some(&b'\n') {
        to += 1;
    }
    patches.replace(from..to, String::new());
}

/// The line ending the file uses.
fn newline(text: &str) -> &'static str {
    match text.contains("\r\n") {
        true => "\r\n",
        false => "\n",
    }
}

/// The edits of one file, with the inserts merged so that two that land on one
/// spot cannot come out in an order that depends on the sort.
#[derive(Default)]
struct Patches {
    edits: Vec<Patch>,
    inserts: BTreeMap<usize, String>,
}

impl Patches {
    fn replace(&mut self, range: Range<usize>, with: impl Into<String>) {
        self.edits.push((range, with.into()));
    }

    fn insert(&mut self, at: usize, with: &str) {
        self.inserts.entry(at).or_default().push_str(with);
    }

    fn is_empty(&self) -> bool {
        self.edits.is_empty() && self.inserts.is_empty()
    }

    fn into_edits(self) -> Vec<Patch> {
        let mut edits = self.edits;
        for (at, with) in self.inserts {
            edits.push((at..at, with));
        }
        edits
    }
}

/// The pools the randomizer draws from, read out of the game's own definitions.
///
/// The weapon and drone pools are held in the order of the name lists of
/// [`crate::randomizer`], which is what its own check of their length is about.
/// The augments are held in name order, each with the weight of its rarity beside
/// it, and the data defines each of them once.
pub fn pools(archive: &FtlArchive) -> Result<Randomizer> {
    let mut weapons: BTreeMap<String, StartingWeapon> = BTreeMap::new();
    let mut drones: BTreeMap<String, StartingDrone> = BTreeMap::new();
    let mut augments: BTreeMap<String, (i32, bool)> = BTreeMap::new();

    for path in archive.paths().map(str::to_string).collect::<Vec<_>>() {
        if !path.starts_with("data/") || !path.ends_with(".xml") {
            continue;
        }
        let bytes = archive.get_file(&path).expect("path came from the archive");
        if !holds(bytes, b"<weaponBlueprint")
            && !holds(bytes, b"<droneBlueprint")
            && !holds(bytes, b"<augBlueprint")
        {
            continue;
        }
        let text = String::from_utf8(bytes.to_vec()).map_err(|error| Error::Unreadable {
            path: path.clone(),
            why: format!("not utf8: {error}"),
        })?;
        // The files are walked in path order and a later definition wins, which is
        // what makes `dlcBlueprintsOverwrite.xml` overwrite.
        let inside = blueprints::elements(&text, 0).map_err(|why| Error::Unreadable {
            path: path.clone(),
            why,
        })?;
        for element in &inside {
            // A definition names itself. The `<weaponBlueprint>` of a
            // `<droneBlueprint>`, which points at one, holds no name attribute.
            let Some((_, name)) = blueprints::attribute(&text[element.tag.clone()], "name") else {
                continue;
            };
            let name = name.to_string();
            let field = |key: &str| child_named(&text, &inside, element, key);
            let number = |key: &str, default: i8| -> i8 {
                field(key).and_then(|value| value.parse().ok()).unwrap_or(default)
            };
            match element.name.as_str() {
                "weaponBlueprint" if ALLOWED_STARTING_WEAPONS_NAMES.contains(&name.as_str()) => {
                    let weapon_type = match field("type").map(|value| value.trim().to_string()) {
                        Some(value) if value == "BEAM" => WeaponType::BEAM,
                        Some(value) if value == "BOMB" => WeaponType::BOMB,
                        Some(value) if value == "BURST" => WeaponType::BURST,
                        Some(value) if value == "LASER" => WeaponType::LASER,
                        Some(value) if value == "MISSILES" => WeaponType::MISSILES,
                        other => {
                            return Err(Error::Unreadable {
                                path: path.clone(),
                                why: format!("{name} is a <weaponBlueprint> of type {other:?}, which the randomizer has no rule for"),
                            });
                        }
                    };
                    weapons.insert(
                        name.clone(),
                        StartingWeapon {
                            name: name.leak(),
                            weapon_type,
                            power: number("power", 0),
                            sp: number("sp", 0),
                            damage: number("damage", 0),
                            shots: number("shots", 1),
                            ion: number("ion", 0),
                            missiles: number("missiles", 0),
                            charge_levels: number("chargeLevels", 1),
                        },
                    );
                }
                "droneBlueprint" if ALLOWED_STARTING_DRONES_NAMES.contains(&name.as_str()) => {
                    drones.insert(
                        name.clone(),
                        StartingDrone { name: name.leak(), power: number("power", 0) },
                    );
                }
                "augBlueprint" => {
                    let rarity = field("rarity").and_then(|value| value.parse().ok());
                    let stackable = field("stackable").is_some_and(|value| value == "true");
                    match rarity {
                        Some(rarity @ 0..=4) => {
                            augments.insert(name, (rarity, stackable));
                        }
                        other => {
                            return Err(Error::Unreadable {
                                path: path.clone(),
                                why: format!("{name} is an <augBlueprint> of rarity {other:?}, which has no weight"),
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // The pools in the order of the name lists, so the length check inside
    // `randomize_ship` is a check that the archive holds what those names name.
    let pick = |held: &BTreeMap<String, StartingWeapon>, want: &str| -> Result<StartingWeapon> {
        held.get(want).copied().ok_or_else(|| Error::Unreadable {
            path: "the blueprint files of the archive".to_string(),
            why: format!("no <weaponBlueprint> named {want}"),
        })
    };
    let mut weapon_pool = Vec::new();
    for want in ALLOWED_STARTING_WEAPONS_NAMES {
        weapon_pool.push(pick(&weapons, want)?);
    }
    let mut drone_pool = Vec::new();
    for want in ALLOWED_STARTING_DRONES_NAMES {
        let drone = drones.get(want).copied().ok_or_else(|| Error::Unreadable {
            path: "the blueprint files of the archive".to_string(),
            why: format!("no <droneBlueprint> named {want}"),
        })?;
        drone_pool.push(drone);
    }
    let mut augment_pool = Vec::new();
    let mut weights = Vec::new();
    for (name, (rarity, stackable)) in &augments {
        augment_pool.push(Augment { name: name.clone(), stackable: *stackable });
        weights.push(rarity_weight(*rarity)?);
    }

    Ok(Randomizer {
        starting_augments_pool: augment_pool,
        augment_weights: weights,
        starting_weapons_pool: weapon_pool,
        starting_drones_pool: drone_pool,
    })
}

/// The share of the draw an augment of one rarity carries. `randomizer.rs` asks
/// for 12, 6, 4, 3 and 3 over the rarities 1, 2, 3, 4 and 0; rarity 0 is what the
/// game files the augments of a race and the stasis pod under.
fn rarity_weight(rarity: i32) -> Result<u32> {
    match rarity {
        1 => Ok(12),
        2 => Ok(6),
        3 => Ok(4),
        4 => Ok(3),
        0 => Ok(3),
        other => Err(Error::Unreadable {
            path: String::new(),
            why: format!("the rarity {other} of an <augBlueprint> has no weight"),
        }),
    }
}

/// The text between the tags of a direct child of `element` that carries `tag`.
fn child_named(text: &str, inside: &[Element], element: &Element, tag: &str) -> Option<String> {
    let child = inside.iter().find(|held| {
        held.name == tag
            && held.depth == element.depth + 1
            && held.span.start > element.span.start
            && held.span.end < element.span.end
    })?;
    between(text, child)
}

/// true when `hay` holds `needle`.
fn holds(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|window| window == needle)
}

/// The hull art of a player ship. Enemy ships have no base image, which is what
/// makes it a player ship test.
fn base_image(layout: &str) -> String {
    format!("img/ship/{layout}_base.png")
}

/// The rooms and doors of a layout, which is where the room of a new gun is read.
fn layout_txt(layout: &str) -> String {
    format!("data/{layout}.txt")
}

/// The hull art placement and the weapon mounts of a layout.
fn layout_xml(layout: &str) -> String {
    format!("data/{layout}.xml")
}