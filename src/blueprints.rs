//! The room assignments of the ship blueprints.
//!
//! A `<shipBlueprint>` names the layout it drives (`layout="kestral"`) and lists
//! its systems in a `<systemList>`, each one naming the room it sits in
//! (`room="4"`). Give a layout a new shape and those numbers have to follow it,
//! which is all this module does: read a blueprints file, move every system into a
//! room of its own, write the file back.
//!
//! Nothing is re-serialized. Every change is a byte range of the input, so the
//! comments, the spacing and the order of the blueprints come back exactly as they
//! went in, and a file that holds no player ship is not written at all.
//!
//! Two rules of the game decide what a room has to hold:
//! * every system takes a room of a shape it was drawn in, and the crew of a system
//!   with a station stand on a tile of it, facing a wall, as the ships that came
//!   with the game draw them. [`WHITELIST`] is that list of configurations, counted
//!   by `src/bin/probe_stations.rs`: the art of a system and the console of its
//!   crew are cut for the rooms they ship in, and a piloting station dropped into a
//!   2x2 room is the clearest example of what goes wrong otherwise. A station also
//!   never looks through a door: see [`crate::layout::stations`].
//! * the medbay and the clone bay are one room with one machine at a time - a
//!   player buys the second and it takes the place of the first - so they get one
//!   room and one `<slot>` between them, and every other system gets a room to
//!   itself. The `<slot>` names the tile the machine stands on, which the crew
//!   cannot use, so a room too small to spare a tile blocks nothing.
//! * the `img` of a system is the art of its room, cut for one shape of room, and
//!   shared by every hull: the 28 ships draw from one set of files. A system moved
//!   into a room of another shape therefore names another file of the same system,
//!   which [`RoomArt`] knows the shipped blueprints to have drawn in that shape.
//!
//! Artillery takes a room like any other system, but no station: the gun fires on
//! its own.

use std::collections::BTreeMap;
use std::ops::Range;

use quick_xml::events::Event;
use quick_xml::reader::Reader;

use crate::layout::{self, Door, Layout, Room, Side, Station};
use crate::rng::Rng;

/// A byte range of the file, and the text that replaces it.
type Patch = (Range<usize>, String);

/// The kind the medbay and the clone bay are counted as: one room, one machine at a
/// time, one `<slot>` between the two.
const BAY: &str = "bay";

/// What the game draws when a blueprint files no `<slot>`: the wall the crew face
/// and the tile they stand on, as `ship_modding.txt` gives them. The bay blocks
/// tile 1. The other systems are always filed by hand, so they have nothing to fall
/// back on here.
const DEFAULTS: [(&str, Option<Side>, Option<i32>); 5] = [
    ("pilot", Some(Side::Right), Some(0)),
    ("weapons", Some(Side::Up), Some(1)),
    ("shields", Some(Side::Left), Some(0)),
    ("engines", Some(Side::Down), Some(2)),
    (BAY, None, Some(1)),
];

/// One room a system was drawn in: the shape of the room, the tile of the station
/// or of the machine, and the wall a station faces. A wall of `None` is a machine
/// nobody stands at, and a tile of -2 blocks nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Plan {
    pub w: i32,
    pub h: i32,
    pub number: i32,
    pub side: Option<Side>,
}

impl Plan {
    /// The station of the configuration, `None` for a machine nobody stands at.
    fn station(&self) -> Option<Station> {
        self.side.map(|side| Station { number: self.number, side })
    }

    /// true when the room has the shape the configuration was drawn in.
    fn holds(&self, room: &Room) -> bool {
        self.w == room.w && self.h == room.h
    }
}

/// A configuration of a station the crew stand at, for the table below.
const fn at(w: i32, h: i32, number: i32, side: Side) -> Plan {
    Plan { w, h, number, side: Some(side) }
}

/// A configuration of a machine nobody stands at, for the table below.
const fn machine(w: i32, h: i32, number: i32) -> Plan {
    Plan { w, h, number, side: None }
}

/// Every room configuration the 30 player blueprints draw each system in, counted by
/// `src/bin/probe_stations.rs` over the ships the game ships with.
///
/// A system only takes a room of a shape off its list, because the art of a system
/// and the console of its crew are cut for the rooms they ship in: the game never
/// draws a piloting station in a 2x2 room, and the art of the room breaks when one
/// is put there. The walls are the ones the game files, which keeps a console off
/// the middle of the floor - of the 140 stations the game files, that of the Circle
/// C hacking system is the one that faces into its own room, and that configuration
/// is left off the list.
///
/// A system that is not listed here has no station and no machine, and takes a room
/// of any shape.
///
/// Public so that `src/bin/probe_stations.rs` can hold the table against the ships
/// the game ships with and say what it left out.
pub const WHITELIST: [(&str, &[Plan]); 8] = [
    // Every piloting station the game draws is thin and faces right: the crew sit
    // at the front of the ship, looking at the enemy.
    ("pilot", &[at(1, 2, 0, Side::Right), at(2, 1, 1, Side::Right)]),
    ("weapons", &[
        at(2, 1, 1, Side::Up),
        at(2, 2, 0, Side::Up),
        at(2, 2, 1, Side::Up),
        at(2, 2, 1, Side::Right),
    ]),
    ("shields", &[
        at(2, 1, 1, Side::Down),
        at(2, 2, 0, Side::Up),
        at(2, 2, 0, Side::Left),
        at(2, 2, 1, Side::Up),
        at(2, 2, 2, Side::Left),
        at(2, 2, 3, Side::Down),
    ]),
    ("engines", &[
        at(1, 2, 0, Side::Up),
        at(1, 2, 1, Side::Down),
        at(1, 2, 1, Side::Left),
        at(2, 1, 1, Side::Right),
        at(2, 2, 1, Side::Up),
        at(2, 2, 2, Side::Down),
        at(2, 2, 3, Side::Right),
    ]),
    ("doors", &[
        at(1, 2, 0, Side::Left),
        at(1, 2, 0, Side::Right),
        at(1, 2, 1, Side::Right),
        at(2, 1, 0, Side::Down),
        at(2, 1, 1, Side::Up),
        at(2, 1, 1, Side::Right),
    ]),
    ("sensors", &[
        at(1, 2, 1, Side::Left),
        at(1, 2, 1, Side::Right),
        at(2, 1, 0, Side::Up),
        at(2, 1, 1, Side::Down),
        at(2, 1, 1, Side::Right),
    ]),
    ("hacking", &[
        at(1, 2, 0, Side::Up),
        at(1, 2, 0, Side::Left),
        at(2, 1, 0, Side::Up),
        at(2, 1, 0, Side::Down),
        at(2, 1, 0, Side::Left),
        at(2, 2, 0, Side::Up),
    ]),
    // A 2x2 bay has a tile to spare for its machine; a thin one is where the crew
    // heal or are cloned, so nothing is blocked there.
    (BAY, &[
        machine(1, 2, -2),
        machine(2, 1, -2),
        machine(2, 2, 0),
        machine(2, 2, 1),
        machine(2, 2, 2),
        machine(2, 2, 3),
    ]),
];

/// The configurations the game files for a kind of room, empty for a system with no
/// station, which takes any room.
fn configurations(kind: &str) -> Vec<Plan> {
    WHITELIST
        .iter()
        .find(|(name, _)| *name == kind)
        .map(|(_, listed)| listed.to_vec())
        .unwrap_or_default()
}

/// How well a room suits a kind of room, best first.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Fit {
    /// A configuration fits and no station looks through a door.
    Clear,
    /// A configuration fits, but every one of them faces a door.
    Door,
    /// The room has a shape the kind was never drawn in.
    Wrong,
}

/// How well a room suits a kind of room.
fn fits(kind: &str, room: &Room, doors: &[Door]) -> Fit {
    let listed = configurations(kind);
    if listed.is_empty() {
        return Fit::Clear;
    }
    if !listed.iter().any(|plan| plan.holds(room)) {
        return Fit::Wrong;
    }
    if listed.iter().any(|plan| stands(plan, room, doors)) {
        Fit::Clear
    } else {
        Fit::Door
    }
}

/// true when a configuration stands in the room, and its station looks through a
/// wall rather than through a door.
fn stands(plan: &Plan, room: &Room, doors: &[Door]) -> bool {
    plan.holds(room)
        && match plan.station() {
            None => true,
            Some(station) => layout::stations(room, doors).contains(&station),
        }
}

/// One `<shipBlueprint>` of a blueprints file.
#[derive(Clone, Debug)]
pub struct Blueprint {
    pub name: String,
    /// The layout this ship drives, which is the stem of its `.txt` file.
    pub layout: String,
    /// Bytes of the whole element.
    pub span: Range<usize>,
}

/// What one blueprint was turned into.
#[derive(Debug)]
pub struct Change {
    pub blueprint: String,
    pub layout: String,
    /// A system tag, and the room it was moved into.
    pub systems: Vec<(String, usize)>,
    /// Anything the randomizer could not do cleanly.
    pub warnings: Vec<String>,
}

/// One edited blueprints file.
pub struct Edit {
    pub bytes: Vec<u8>,
    pub changes: Vec<Change>,
}

/// The room a system shares with another, and the kind of room it is: the clone bay
/// is a medbay with a different machine, in the same room, blocking the same tile.
fn share(name: &str) -> &str {
    if matches!(name, "medbay" | "clonebay") { BAY } else { name }
}

/// Every `<shipBlueprint>` of a blueprints file, in the order they appear.
pub fn blueprints(text: &str) -> Result<Vec<Blueprint>, String> {
    let mut found = Vec::new();
    for element in elements(text, 0)? {
        if element.name != "shipBlueprint" {
            continue;
        }
        let tag = &text[element.tag.clone()];
        let name = attribute(tag, "name").map(|(_, value)| value.to_string());
        let layout = attribute(tag, "layout").map(|(_, value)| value.to_string());
        match (name, layout) {
            (Some(name), Some(layout)) => found.push(Blueprint {
                name,
                layout,
                span: element.span,
            }),
            _ => {
                return Err(format!(
                    "<shipBlueprint> at {} has no name or no layout",
                    element.tag.start
                ));
            }
        }
    }
    Ok(found)
}

/// How many rooms one `<shipBlueprint>` needs: one per system, the medbay and the
/// clone bay counting once.
pub fn rooms_needed(blueprint: &str) -> Result<usize, String> {
    let mut groups: Vec<String> = Vec::new();
    for system in system_tags(blueprint, 0)? {
        let key = share(&system.name).to_string();
        if !groups.contains(&key) {
            groups.push(key);
        }
    }
    Ok(groups.len())
}

/// The system tags of one `<shipBlueprint>`: the children of its `<systemList>`
/// that name a room. `base` is where the slice starts in its file.
///
/// Public because `src/bin/probe_stations.rs` counts the rooms and the stations of
/// the ships the game ships with, and has to read a blueprint the same way the
/// randomizer does.
pub fn system_tags(text: &str, base: usize) -> Result<Vec<Element>, String> {
    let inside = elements(text, base)?;
    let Some(list) = inside
        .iter()
        .find(|element| element.name == "systemList")
        .cloned()
    else {
        return Ok(Vec::new());
    };
    Ok(inside
        .into_iter()
        .filter(|element| {
            element.depth == list.depth + 1
                && element.span.start > list.span.start
                && element.span.end < list.span.end
                && attribute(&text[element.tag.start - base..element.tag.end - base], "room")
                    .is_some()
        })
        .collect())
}

/// Note the room art of one shipped blueprint: every system that names art, with the
/// shape of the room it sits in, which is what says what that art was cut for.
/// `rooms` calls this over the archive before it changes a thing.
pub fn gather(
    text: &str,
    blueprint: &Blueprint,
    shapes: &[(i32, i32)],
    art: &mut RoomArt,
) -> Result<(), String> {
    for system in system_tags(&text[blueprint.span.clone()], blueprint.span.start)? {
        let tag = &text[system.tag.clone()];
        let (Some((_, room)), Some((_, img))) = (attribute(tag, "room"), attribute(tag, "img"))
        else {
            continue;
        };
        let Ok(room) = room.parse::<usize>() else { continue };
        let Some(&(w, h)) = shapes.get(room) else { continue };
        art.file(&system.name, (w, h), img);
    }
    Ok(())
}

/// Move the systems of every blueprint whose layout is in `layouts` into a room of
/// its own, and name the room art that fits the room it was handed. `Ok(None)` means
/// the file holds none of them.
pub fn randomize(
    text: &str,
    layouts: &BTreeMap<String, Layout>,
    art: &RoomArt,
    rng: &mut Rng,
) -> Result<Option<Edit>, String> {
    let mut patches: Vec<Patch> = Vec::new();
    let mut changes = Vec::new();

    for blueprint in blueprints(text)? {
        let Some(layout) = layouts.get(&blueprint.layout) else {
            continue;
        };
        let mut change = Change {
            blueprint: blueprint.name.clone(),
            layout: blueprint.layout.clone(),
            systems: Vec::new(),
            warnings: Vec::new(),
        };
        assign(text, &blueprint, layout, art, rng, &mut patches, &mut change)?;
        changes.push(change);
    }

    if patches.is_empty() {
        return Ok(None);
    }
    Ok(Some(Edit {
        bytes: rewrite(text, &patches),
        changes,
    }))
}

/// The systems of one blueprint, handed the rooms they got.
fn assign(
    text: &str,
    blueprint: &Blueprint,
    layout: &Layout,
    art: &RoomArt,
    rng: &mut Rng,
    patches: &mut Vec<Patch>,
    change: &mut Change,
) -> Result<(), String> {
    // The element ranges come back in file coordinates, so every patch can be
    // written straight into the file.
    let base = blueprint.span.start;
    let systems: Vec<Element> = system_tags(&text[blueprint.span.clone()], base)?;
    if systems.is_empty() {
        change
            .warnings
            .push(format!("{} has no system with a room", blueprint.name));
        return Ok(());
    }

    // One room per group of systems, in the order the systems appear.
    let groups = groups(&systems);

    // Every group gets a room of its own that its kind fits in. The caller is meant
    // to have drawn a layout that can house the ship - see [`houses`] - so a layout
    // that cannot is made the best of here and said out loud.
    let room_of = match match_rooms(&groups, layout, rng) {
        Some(rooms) => rooms,
        None => {
            change.warnings.push(format!(
                "{} cannot be housed in the {} rooms of {}: some systems stand in a room of a shape they were not drawn in",
                blueprint.name,
                layout.rooms.len(),
                blueprint.layout
            ));
            force_rooms(&groups, layout, rng)
        }
    };

    // Then each group has its room and its configuration written. One configuration
    // per group, so that a medbay and a clone bay - one machine, then the other, in
    // the same room - file one tile rather than two.
    for (group, (kind, members)) in groups.iter().enumerate() {
        let room = room_of[group];
        let picked = pick_plan(
            text,
            kind,
            members,
            &layout.rooms[room],
            &layout.doors,
            rng,
            &mut change.warnings,
        );
        let picture = picture(
            text,
            kind,
            members,
            (layout.rooms[room].w, layout.rooms[room].h),
            art,
            rng,
            &mut change.warnings,
        );
        for system in members {
            let raw = &text[system.tag.clone()];
            let (value, _) = attribute(raw, "room").expect("systems were picked for their room");
            patches.push((
                system.tag.start + value.start..system.tag.start + value.end,
                room.to_string(),
            ));
            change.systems.push((system.name.clone(), room));
            write_picture(text, system, &picture, patches);
            write_slot(
                text,
                system,
                picked,
                &layout.rooms[room],
                &mut change.warnings,
                patches,
            );
        }
    }
    Ok(())
}

/// The groups of a blueprint: one per room it needs, named by the kind of room, with
/// the systems that share it. The medbay and the clone bay are one group.
fn groups(systems: &[Element]) -> Vec<(&str, Vec<&Element>)> {
    let mut groups: Vec<(&str, Vec<&Element>)> = Vec::new();
    for system in systems.iter() {
        let key = share(&system.name);
        match groups.iter_mut().find(|(name, _)| *name == key) {
            Some((_, members)) => members.push(system),
            None => groups.push((key, vec![system])),
        }
    }
    groups
}

/// The rooms a kind of room can stand in, as well as `worst` at least: of a shape the
/// kind was drawn in, and - when `worst` is [`Fit::Clear`] - with a wall the door is
/// off.
fn choices(kind: &str, layout: &Layout, worst: Fit) -> Vec<usize> {
    (0..layout.rooms.len())
        .filter(|&room| fits(kind, &layout.rooms[room], &layout.doors) <= worst)
        .collect()
}

/// Every group handed a room of its own that its kind fits in, first with a wall the
/// door is off and, failing that, in any room of a shape the kind was drawn in.
/// `None` when the rooms do not cover the systems.
fn match_rooms(groups: &[(&str, Vec<&Element>)], layout: &Layout, rng: &mut Rng) -> Option<Vec<usize>> {
    let mut order: Vec<usize> = (0..groups.len()).collect();
    rng.shuffle(&mut order);
    for worst in [Fit::Clear, Fit::Door] {
        // The groups that can be housed in the fewest rooms go first, so a piloting
        // station is served while thin rooms are still going and a system with no
        // station, which fits anywhere, takes what is left.
        order.sort_by_key(|&group| choices(groups[group].0, layout, worst).len());
        if let Some(rooms) = match_on(groups, layout, worst, &order) {
            return Some(rooms);
        }
    }
    None
}

/// true when a layout houses every system of a blueprint, each group in a room of a
/// shape it was drawn in and, for the systems the crew stand at, with a wall the door
/// is off. `rooms::redraw` asks this before it keeps a layout it drew.
pub fn houses(text: &str, blueprint: &Blueprint, layout: &Layout) -> bool {
    let Ok(systems) = system_tags(&text[blueprint.span.clone()], blueprint.span.start) else {
        // A blueprint this module cannot read is left alone by `randomize`, so a
        // layout is not redrawn over it.
        return true;
    };
    let groups = groups(&systems);
    let mut order: Vec<usize> = (0..groups.len()).collect();
    order.sort_by_key(|&group| choices(groups[group].0, layout, Fit::Clear).len());
    match_on(&groups, layout, Fit::Clear, &order).is_some()
}

/// The matching behind [`match_rooms`]: each group in turn claims a room, and a group
/// that has taken one is moved on when a later group has nowhere else. Without that
/// moving, a group that was served first can take the last room a later group can
/// stand in and leave it with none, where a swap would have housed them both.
/// `order` is the order the groups are served in.
fn match_on(
    groups: &[(&str, Vec<&Element>)],
    layout: &Layout,
    worst: Fit,
    order: &[usize],
) -> Option<Vec<usize>> {
    let allowed: Vec<Vec<usize>> = groups
        .iter()
        .map(|(kind, _)| choices(kind, layout, worst))
        .collect();
    let mut held: Vec<Option<usize>> = vec![None; layout.rooms.len()];
    for &group in order {
        let mut tried = vec![false; layout.rooms.len()];
        if !augment(group, &allowed, &mut held, &mut tried) {
            return None;
        }
    }
    let mut room_of = vec![0usize; groups.len()];
    for (room, owner) in held.iter().enumerate() {
        if let Some(group) = owner {
            room_of[*group] = room;
        }
    }
    Some(room_of)
}

/// One group claims a room: an empty one it can take, or one held by a group that can
/// be moved elsewhere. `tried` keeps the search off the rooms it has looked at, which
/// is also what stops it going round in circles.
fn augment(
    group: usize,
    allowed: &[Vec<usize>],
    held: &mut Vec<Option<usize>>,
    tried: &mut [bool],
) -> bool {
    allowed[group].iter().copied().any(|room| {
        if tried[room] {
            return false;
        }
        tried[room] = true;
        let won = held[room].is_none_or(|other| augment(other, allowed, held, tried));
        if won {
            held[room] = Some(group);
        }
        won
    })
}

/// Rooms for every group when no matching exists: the best room still going, and a
/// room handed out twice rather than a system left without one. Only the layout of a
/// ship too big for its own rooms, or one kept after every redraw failed, comes here.
fn force_rooms(groups: &[(&str, Vec<&Element>)], layout: &Layout, rng: &mut Rng) -> Vec<usize> {
    let mut free: Vec<usize> = (0..layout.rooms.len()).collect();
    rng.shuffle(&mut free);
    let mut room_of: Vec<usize> = Vec::with_capacity(groups.len());
    for (kind, _) in groups.iter() {
        let best = match free.is_empty() {
            false => free.clone(),
            true => (0..layout.rooms.len()).collect(),
        };
        let mut room = best[0];
        let mut fit = fits(kind, &layout.rooms[room], &layout.doors);
        for other in best {
            let better = fits(kind, &layout.rooms[other], &layout.doors);
            if better < fit {
                room = other;
                fit = better;
            }
        }
        free.retain(|used| *used != room);
        room_of.push(room);
    }
    room_of
}

/// What to do with the room art of a group of systems once its room is drawn.
#[derive(Debug)]
enum Picture {
    /// The file the blueprint names was cut for the room that was drawn.
    Keep,
    /// Name another file of the same system, cut for that room.
    Replace(String),
    /// The system has no art for that shape, so the room is drawn plain.
    Plain,
}

/// The room art the game ships, gathered from the blueprints before any of them is
/// changed.
///
/// The art of a room belongs to the system, not to the ship: the 28 hulls share the
/// one set of files, and every file is cut for one shape of room - 35 pixels to a
/// tile, so `room_pilot.png` at 35x70 is art for a 1x2 room. A system moved into a
/// room of another shape is therefore drawn with the art of another ship of the same
/// system, which is what the files are for.
#[derive(Default)]
pub struct RoomArt {
    /// The shape each file was cut for, `None` for the few the shipped blueprints use
    /// in two shapes - the art of `room_engines_2` is 70x70, yet one hull draws it in
    /// a 1x2 room, which cannot be honoured for both.
    cut: BTreeMap<String, Option<(i32, i32)>>,
    /// The files one kind of room has for one shape, in the order they were read.
    files: BTreeMap<(String, (i32, i32)), Vec<String>>,
}

impl RoomArt {
    /// Note one system of one shipped ship: the kind of room it is, the shape of the
    /// room it sat in, and the art its blueprint named.
    pub fn file(&mut self, system: &str, shape: (i32, i32), img: &str) {
        let cut = match self.cut.get(img) {
            Some(seen) if *seen != Some(shape) => None,
            Some(seen) => *seen,
            None => Some(shape),
        };
        self.cut.insert(img.to_string(), cut);
        let files = self
            .files
            .entry((share(system).to_string(), shape))
            .or_default();
        if !files.iter().any(|held| held == img) {
            files.push(img.to_string());
        }
    }

    /// The shape a file was cut for, `None` when nothing was read of it or when the
    /// shipped ships draw it in more than one shape.
    fn cut_for(&self, img: &str) -> Option<(i32, i32)> {
        self.cut.get(img).copied().flatten()
    }

    /// A file of a kind of room cut for a shape, `None` when the kind has none.
    ///
    /// A file the shipped ships draw in two shapes is a last choice: it was cut for
    /// one of them, and there is no telling which from the files alone, so one that
    /// was only ever drawn for the shape at hand comes first.
    fn pick(&self, kind: &str, shape: (i32, i32), rng: &mut Rng) -> Option<&str> {
        let files = self.files.get(&(kind.to_string(), shape))?;
        let cut: Vec<usize> = files
            .iter()
            .enumerate()
            .filter(|(_, file)| self.cut_for(file) == Some(shape))
            .map(|(at, _)| at)
            .collect();
        let chosen = if cut.is_empty() {
            rng.index(files.len())
        } else {
            cut[rng.index(cut.len())]
        };
        Some(&files[chosen])
    }
}

/// The art a group of systems should name once its room is drawn. One choice per
/// group, because the medbay and the clone bay draw one room.
fn picture(
    text: &str,
    kind: &str,
    members: &[&Element],
    shape: (i32, i32),
    art: &RoomArt,
    rng: &mut Rng,
    warnings: &mut Vec<String>,
) -> Picture {
    let Some(named) = members.iter().find_map(|system| image_of(text, system)) else {
        // The ship drew this room plain, so it stays plain.
        return Picture::Keep;
    };
    if art.cut_for(&named) == Some(shape) {
        return Picture::Keep;
    }
    match art.pick(kind, shape, rng) {
        Some(file) => Picture::Replace(file.to_string()),
        None => {
            warnings.push(format!(
                "a {kind} has no art cut for a {}x{} room, so it is drawn plain",
                shape.0, shape.1
            ));
            Picture::Plain
        }
    }
}

/// The `img` a system names, `None` when it names none.
fn image_of(text: &str, system: &Element) -> Option<String> {
    let (_, value) = attribute(&text[system.tag.clone()], "img")?;
    Some(value.to_string())
}

/// Write the room art of a system: another file, or no file at all, which leaves the
/// room as a plain one. A system that named none keeps its plain room.
fn write_picture(text: &str, system: &Element, picture: &Picture, patches: &mut Vec<Patch>) {
    let Some((value, _)) = attribute(&text[system.tag.clone()], "img") else {
        return;
    };
    let value = system.tag.start + value.start..system.tag.start + value.end;
    match picture {
        Picture::Keep => {}
        Picture::Replace(file) => patches.push((value, file.clone())),
        Picture::Plain => {
            // The attribute goes, with the space in front of it, so the tag reads as
            // though it never named art.
            if let Some(span) = attribute_span(text, &value) {
                patches.push((span, String::new()));
            }
        }
    }
}

/// The whole ` key="value"` around a value this module has located - the closing
/// quote, the name, the `=` and the space the attribute sits behind - so that an
/// attribute can be taken out of a tag whole. `None` when the bytes around the value
/// are not the shape this module files attributes in.
fn attribute_span(text: &str, value: &Range<usize>) -> Option<Range<usize>> {
    let bytes = text.as_bytes();
    // The value sits between quotes.
    let quote = *bytes.get(value.start.checked_sub(1)?)?;
    if bytes.get(value.end) != Some(&b'"') || quote != b'"' {
        return None;
    }
    // Back over the opening quote, then over the `=` and the name, which the game
    // tolerates spaces around, and finally over the space the attribute hangs
    // behind.
    let mut back = value.start - 1;
    for keep in [
        |byte: &u8| byte.is_ascii_whitespace(),
        |byte: &u8| *byte == b'=',
        |byte: &u8| byte.is_ascii_whitespace(),
        |byte: &u8| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'),
        |byte: &u8| byte.is_ascii_whitespace(),
    ] {
        while back > 0 && keep(&bytes[back - 1]) {
            back -= 1;
        }
    }
    Some(back..value.end + 1)
}

/// The configuration a group of systems gets in the room it was handed: one of those
/// the game files for the kind, with the door off the wall it faces where the room
/// allows that, and the wall and the tile the blueprint filed where a configuration
/// has them. `None` when the kind has neither a station nor a machine.
fn pick_plan(
    text: &str,
    kind: &str,
    members: &[&Element],
    room: &Room,
    doors: &[Door],
    rng: &mut Rng,
    warnings: &mut Vec<String>,
) -> Option<Plan> {
    let listed = configurations(kind);
    if listed.is_empty() {
        return None;
    }
    let clear: Vec<Plan> = listed.iter().filter(|plan| stands(plan, room, doors)).copied().collect();
    let shape: Vec<Plan> = listed.iter().filter(|plan| plan.holds(room)).copied().collect();
    let pool = if !clear.is_empty() {
        clear
    } else if !shape.is_empty() {
        warnings.push(format!(
            "{kind} has no wall here with the door off it, so its station faces a door"
        ));
        shape
    } else {
        return least_worst(kind, room, doors, warnings, rng);
    };

    // The wall and the tile the blueprint filed, or the ones the game would have
    // drawn in their place, so a ship keeps the look it came with as far as the room
    // allows.
    let (wall, tile) = wanted(text, kind, members);
    let pool = prefer(pool, |plan| wall.is_none_or(|side| plan.side == Some(side)));
    let pool = prefer(pool, |plan| tile.is_none_or(|number| plan.number == number));
    Some(pool[rng.index(pool.len())])
}

/// The entries of a pool a rule picks, or the whole pool when it picks none.
fn prefer(pool: Vec<Plan>, pick: impl Fn(&Plan) -> bool) -> Vec<Plan> {
    let kept: Vec<Plan> = pool.iter().filter(|plan| pick(plan)).copied().collect();
    match kept.is_empty() {
        true => pool,
        false => kept,
    }
}

/// The wall and the tile to aim for: the ones the blueprint filed for the group, or
/// the ones the game would have drawn in their place.
fn wanted(text: &str, kind: &str, members: &[&Element]) -> (Option<Side>, Option<i32>) {
    let mut wall = None;
    let mut tile = None;
    for system in members {
        let inside = elements(&text[system.span.clone()], system.span.start).unwrap_or_default();
        let slots: Vec<&Element> = inside
            .iter()
            .filter(|element| element.name == "slot")
            .collect();
        wall = wall.or_else(|| slots.iter().find_map(|slot| slot_side(text, slot)));
        tile = tile.or_else(|| slots.iter().find_map(|slot| slot_number(text, slot)));
    }
    let default = DEFAULTS.iter().find(|(name, _, _)| *name == kind);
    (
        wall.or(default.and_then(|(_, side, _)| *side)),
        tile.or(default.and_then(|(_, _, number)| *number)),
    )
}

/// The best that is left when a system was handed a room of a shape it was never
/// drawn in: the wall rule alone, which at least keeps a console against a wall and
/// out of the middle of the floor, and no tile blocked at all for a machine.
fn least_worst(
    kind: &str,
    room: &Room,
    doors: &[Door],
    warnings: &mut Vec<String>,
    rng: &mut Rng,
) -> Option<Plan> {
    warnings.push(format!(
        "a {kind} stands in a {}x{} room, a shape no shipped ship draws it in",
        room.w, room.h
    ));
    if !configurations(kind).iter().any(|plan| plan.side.is_some()) {
        return Some(Plan { w: room.w, h: room.h, number: -2, side: None });
    }
    let mut spots = layout::stations(room, doors);
    if spots.is_empty() {
        spots = layout::station_tiles(room);
    }
    let spot = spots[rng.index(spots.len())];
    Some(Plan { w: room.w, h: room.h, number: spot.number, side: Some(spot.side) })
}

/// Write the `<slot>` of a configuration into a system, or leave the numbers alone
/// in a system that has neither a station nor a machine.
///
/// A number names a tile of the room, so a number the new room does not have would
/// put a machine out of reach: it blocks nothing instead.
fn write_slot(
    text: &str,
    system: &Element,
    plan: Option<Plan>,
    room: &Room,
    warnings: &mut Vec<String>,
    patches: &mut Vec<Patch>,
) {
    let inside = match elements(&text[system.span.clone()], system.span.start) {
        Ok(inside) => inside,
        Err(error) => {
            warnings.push(format!("<{}>: {error}", system.name));
            return;
        }
    };
    let slots: Vec<&Element> = inside
        .iter()
        .filter(|element| element.name == "slot")
        .collect();

    let Some(picked) = plan else {
        let tiles = room.w * room.h;
        for element in &inside {
            if element.name != "number" {
                continue;
            }
            let value = text[number_range(element)].trim();
            if let Ok(number) = value.parse::<i32>()
                && number >= 0
                && number >= tiles
            {
                warnings.push(format!(
                    "<{}> numbered tile {number} of a room of {tiles}, which blocks nothing now",
                    system.name
                ));
                patches.push((number_range(element), "-2".to_string()));
            }
        }
        return;
    };
    let block = slot_block(text, system, picked.side, picked.number);
    write_block(text, system, &slots, &block, warnings, patches);
}

/// The wall a `<slot>` names, `None` when it names none or a word the blueprints
/// do not use.
fn slot_side(text: &str, slot: &Element) -> Option<Side> {
    Side::parse(&inside(text, slot, "direction")?)
}

/// The tile a `<slot>` names, `None` when it names none or is not a number.
fn slot_number(text: &str, slot: &Element) -> Option<i32> {
    inside(text, slot, "number")?.parse().ok()
}

/// One `<slot>` block, indented like the system it belongs to: the tile it names,
/// and the wall the crew face when someone stands there.
fn slot_block(text: &str, system: &Element, side: Option<Side>, number: i32) -> String {
    let newline = match text.contains("\r\n") {
        true => "\r\n",
        false => "\n",
    };
    let edge = indent_before(text, system.span.start);
    let one = format!("{edge}\t");
    let two = format!("{one}\t");
    let wall = match side {
        Some(side) => format!("{newline}{two}<direction>{}</direction>", side.name()),
        None => String::new(),
    };
    format!(
        "{newline}{one}<slot>{wall}{newline}{two}<number>{number}</number>{newline}{one}</slot>{newline}{edge}"
    )
}

/// Put a `<slot>` block where the slots of a system were, or in front of its
/// closing tag when it had none.
fn write_block(
    text: &str,
    system: &Element,
    slots: &[&Element],
    block: &str,
    warnings: &mut Vec<String>,
    patches: &mut Vec<Patch>,
) {
    let close = format!("</{}>", system.name);
    let Some(closing) = system
        .span
        .end
        .checked_sub(close.len())
        .filter(|at| *at >= system.tag.end && text[*at..system.span.end] == close)
    else {
        // Not closed, so self closing: the `/>` becomes an open tag, the block and
        // a closing tag.
        if !text[system.tag.clone()].ends_with("/>") {
            warnings.push(format!("<{}> is not closed", system.name));
            return;
        }
        patches.push((
            system.tag.end - 2..system.tag.end,
            format!(">{block}</{}>", system.name),
        ));
        return;
    };

    let children = system.tag.end..closing;
    let mut rest = text[children.clone()].to_string();
    for slot in slots.iter().rev() {
        rest.replace_range(slot.span.start - children.start..slot.span.end - children.start, "");
    }
    if carries_tags(&rest) {
        warnings.push(format!(
            "<{}> holds something that is not a slot, its station stayed where it was",
            system.name
        ));
        return;
    }
    // The new block takes the place of the old slots, and of the whitespace that
    // leaned on them, so no blank line is left behind and the comments the
    // blueprint filed beside them stay where they were. Without a slot the block
    // goes in front of the closing tag.
    let region = match slots.first() {
        Some(first) => first.span.start..slots.last().expect("first came from slots").span.end,
        None => closing..closing,
    };
    let bytes = text.as_bytes();
    let mut from = region.start;
    while from > children.start && bytes[from - 1].is_ascii_whitespace() {
        from -= 1;
    }
    let mut to = region.end;
    while to < closing && bytes[to].is_ascii_whitespace() {
        to += 1;
    }
    patches.push((from..to, block.to_string()));
}

/// true when a text holds a tag that is not a comment. The blueprint files carry
/// notes as comments between the systems and their slots, and those may stay.
fn carries_tags(text: &str) -> bool {
    let mut rest = text;
    let mut outside = String::new();
    while let Some(from) = rest.find("<!--") {
        outside.push_str(&rest[..from]);
        match rest[from..].find("-->") {
            Some(ends) => rest = &rest[from + ends + 3..],
            None => {
                rest = "";
                break;
            }
        }
    }
    outside.push_str(rest);
    outside.chars().any(|ch| ch == '<')
}

/// The text of a `<child>` of an element, trimmed.
fn inside(text: &str, element: &Element, child: &str) -> Option<String> {
    let at = text[element.span.clone()].find(&format!("<{child}>"))? + element.span.start;
    let open = at + child.len() + 2;
    let end = open + text[open..].find(&format!("</{child}>"))?;
    Some(text[open..end].trim().to_string())
}

/// The bytes between a `<number>` tag and its closing tag.
fn number_range(element: &Element) -> Range<usize> {
    element.tag.end..element.span.end - "</number>".len()
}

/// The whitespace that opens the line `at` sits on.
fn indent_before(text: &str, at: usize) -> String {
    let line = text[..at].rsplit('\n').next().unwrap_or("");
    line.chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}

/// An element of a file, with its byte ranges.
#[derive(Debug, Clone)]
pub struct Element {
    pub name: String,
    /// The `<name ...>` bytes, `/` included when the element is self closing.
    pub tag: Range<usize>,
    /// The whole element, children included.
    pub span: Range<usize>,
    /// How deep it sits, the first element of the walk being at depth 0.
    pub depth: usize,
}

/// Every element of `text`, in document order. `base` is where `text` starts in
/// the file the ranges are meant for, which lets a slice of a file hand back file
/// coordinates.
///
/// quick-xml reports the byte just past each event, so a tag starts where its own
/// text says it does. That is checked against the source: a range that does not
/// spell the tag it claims is an error, not a guess.
fn elements(text: &str, base: usize) -> Result<Vec<Element>, String> {
    let mut reader = Reader::from_str(text);
    let mut stack: Vec<(String, Range<usize>)> = Vec::new();
    let mut out = Vec::new();
    loop {
        let end = match reader.read_event() {
            Ok(Event::Eof) => break,
            Ok(Event::Start(tag)) => {
                let end = reader.buffer_position() as usize;
                let name = name(&tag);
                let start = tag_start(text, end, tag.as_ref(), ">")?;
                stack.push((name, start + base..end + base));
                continue;
            }
            Ok(Event::Empty(tag)) => {
                let end = reader.buffer_position() as usize;
                let name = name(&tag);
                let start = tag_start(text, end, tag.as_ref(), "/>")?;
                out.push(Element {
                    name,
                    tag: start + base..end + base,
                    span: start + base..end + base,
                    depth: stack.len(),
                });
                continue;
            }
            Ok(Event::End(tag)) => {
                let end = reader.buffer_position() as usize;
                let name = String::from_utf8_lossy(tag.local_name().as_ref()).into_owned();
                let Some((open, tag)) = stack.pop() else {
                    return Err(format!("</{name}> at {end} closes nothing"));
                };
                if open != name {
                    return Err(format!("</{name}> at {end} closes <{open}>"));
                }
                (name, tag, end + base)
            }
            Ok(_) => continue,
            Err(error) => return Err(error.to_string()),
        };
        out.push(Element {
            name: end.0,
            tag: end.1.clone(),
            span: end.1.start..end.2,
            depth: stack.len(),
        });
    }
    if let Some((open, _)) = stack.last() {
        return Err(format!("<{open}> is never closed"));
    }
    Ok(out)
}

fn name(tag: &quick_xml::events::BytesStart<'_>) -> String {
    String::from_utf8_lossy(tag.local_name().as_ref()).into_owned()
}

/// Where a tag starts, given where it ended and what quick-xml read out of it.
fn tag_start(text: &str, end: usize, inner: &[u8], tail: &str) -> Result<usize, String> {
    let need = 1 + inner.len() + tail.len();
    if end < need {
        return Err(format!("a tag ends at {end}, which is before the file starts"));
    }
    let start = end - need;
    let mut want = Vec::with_capacity(need);
    want.push(b'<');
    want.extend_from_slice(inner);
    want.extend_from_slice(tail.as_bytes());
    if &text.as_bytes()[start..end] != want.as_slice() {
        return Err(format!(
            "the tag at {start}..{end} is {:?}, not {:?}",
            &text[start..end],
            String::from_utf8_lossy(&want)
        ));
    }
    Ok(start)
}

/// The value of an attribute, with the byte range of the value itself.
///
/// The key has to be a name of its own and be followed by `=`. That is what keeps
/// a value that happens to hold it - `img="room_doors_5"` - from being mistaken
/// for `room="5"`, and it takes the one blueprint where two attributes were typed
/// without a space between them: `max="3"room="7"`.
pub fn attribute<'a>(tag: &'a str, key: &str) -> Option<(Range<usize>, &'a str)> {
    let bytes = tag.as_bytes();
    let mut from = 0;
    while let Some(found) = tag[from..].find(key) {
        let at = from + found;
        from = at + key.len();
        let before = if at == 0 { b'<' } else { bytes[at - 1] };
        if before.is_ascii_alphanumeric()
            || matches!(before, b'-' | b'_' | b'.' | b':')
        {
            continue;
        }
        let mut after = at + key.len();
        while bytes.get(after).is_some_and(u8::is_ascii_whitespace) {
            after += 1;
        }
        if bytes.get(after) != Some(&b'=') {
            continue;
        }
        after += 1;
        while bytes.get(after).is_some_and(u8::is_ascii_whitespace) {
            after += 1;
        }
        let quote = *bytes.get(after)?;
        if quote != b'"' && quote != b'\'' {
            continue;
        }
        let value = after + 1;
        let close = value + tag[value..].find(quote as char)?;
        return Some((value..close, &tag[value..close]));
    }
    None
}

/// Replace each range of `text`, highest offset first, so the ranges below stay
/// where they were found.
fn rewrite(text: &str, patches: &[Patch]) -> Vec<u8> {
    let mut order: Vec<&Patch> = patches.iter().collect();
    order.sort_by_key(|patch| std::cmp::Reverse(patch.0.start));
    let mut out = text.as_bytes().to_vec();
    for (range, with) in order {
        out.splice(range.start..range.end, with.as_bytes().iter().copied());
    }
    out
}