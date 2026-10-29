//! The room assignments of the ship blueprints.
//!
//! A `<shipBlueprint>` names the layout it drives (`layout="kestral"`) and lists
//! its systems in a `<systemList>`, each one naming the room it sits in
//! (`room="4"`). Give a layout a new shape and those numbers have to follow it,
//! which is all this module does: read a blueprints file, move every system into a
//! room of its own, write the file back. It also points the `<floorImage>` of the
//! eight C hulls at the deck drawn for their own layout, since they are shipped to
//! borrow the floor of the A ship.
//!
//! Nothing is re-serialized. Every change is a byte range of the input, so the
//! comments, the spacing and the order of the blueprints come back exactly as they
//! went in, and a file that holds no player ship is not written at all.
//!
//! Two rules of the game decide what a room has to hold:
//! * every system takes a room of a shape it was drawn in, its crew stand at the tile
//!   and the wall the ships that came with the game file for that room, and the art it
//!   names is the art a shipped ship named with that station. [`Stock`] holds those
//!   configurations, read out of the archive before a thing is changed: the art of a
//!   room is painted around one console at one wall, so a piloting strip manned from
//!   the far tile shows a crew member at a blank wall. A station also never looks
//!   through a door: see [`crate::layout::stations`].
//! * the medbay and the clone bay are one room with one machine at a time - a
//!   player buys the second and it takes the place of the first - so they get one
//!   room and one `<slot>` between them, and every other system gets a room to
//!   itself. The `<slot>` names the tile the machine stands on, which the crew
//!   cannot use, so a room too small to spare a tile blocks nothing.
//!
//! The art of a room belongs to its system and to no hull: the 28 ships draw from one
//! set of files, so a system moved into a room of another shape names another file of
//! its own system, one a shipped ship named in that shape too. A handful of shipped
//! rooms name art cut for another shape; those are left as they are, since the game
//! draws them that way.
//!
//! Artillery takes a room like any other system, but no station: the gun fires on
//! its own.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use quick_xml::events::Event;
use quick_xml::reader::Reader;

use crate::layout::{self, Door, Layout, Room, Side, Station};
use crate::rng::Rng;

/// A byte range of the file, and the text that replaces it.
pub(crate) type Patch = (Range<usize>, String);

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

/// The tile of a system that has neither a station nor a machine: such a system is
/// not filed a `<slot>` at all, and only the shape of its room and its art matter.
pub const NO_SLOT: i32 = i32::MIN;

/// One room one system was drawn in: the shape of the room, the tile of the station
/// or of the machine, the wall a station faces, and the art the blueprint of that
/// ship named - `None` when the tag named none, which is how the game is told to
/// draw the art of the system's own name. `Some("-")` is the opposite: a blueprint
/// that says outright that the room is drawn plain.
///
/// The art of a room paints its console at one tile against one wall, so the art of a
/// system and the `<slot>` of its crew are one decision and not two. Every piloting
/// strip is painted with the crew at the front of the ship looking out, and a station
/// filed anywhere else in it shows a crew member at a blank wall.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Plan {
    pub w: i32,
    pub h: i32,
    pub number: i32,
    pub side: Option<Side>,
    pub img: Option<String>,
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

    /// The tile to file in a `<slot>`, `None` when the system is left the numbers it
    /// came with because it has neither a station nor a machine.
    fn tile(&self) -> Option<i32> {
        match self.number {
            NO_SLOT => None,
            number => Some(number),
        }
    }

    /// How a configuration reads in a table.
    pub fn label(&self) -> String {
        let tile = match self.tile() {
            None => "no tile".to_string(),
            Some(number) => number.to_string(),
        };
        match self.side {
            Some(side) => format!("{}x{} {tile} {}", self.w, self.h, side.name()),
            None => format!("{}x{} {tile}", self.w, self.h),
        }
    }
}

/// What the ships the mod is written for draw: per kind of room, and per shape of
/// room, the configurations of the station or machine and the art filed with them.
///
/// This is the whole rule of where a system may go. A system is not moved into a room
/// of a shape no shipped ship draws it in, and none is handed a station and an art the
/// game never paired together, because the art is painted around that station. The
/// table is read out of the archive before any blueprint is changed, so the shipped
/// ships are the rule and not a guess about them - including the handful of rooms
/// whose art was cut for another shape of room, which the game draws anyway.
///
/// A kind of room no shipped ship has - the teleporter, which has no art of its own at
/// all - is not in the table and takes a room of any shape.
#[derive(Default)]
pub struct Stock {
    /// Per kind of room and per shape, the configurations filed, in the order they
    /// were read.
    plans: BTreeMap<(String, (i32, i32)), Vec<Plan>>,
    /// The kinds of room the shipped ships file at all.
    kinds: BTreeSet<String>,
}

impl Stock {
    /// Note one system of one shipped ship.
    pub fn file(&mut self, kind: &str, plan: Plan) {
        let shape = (plan.w, plan.h);
        self.kinds.insert(kind.to_string());
        let held = self.plans.entry((kind.to_string(), shape)).or_default();
        if !held
            .iter()
            .any(|other| other.number == plan.number && other.side == plan.side && other.img == plan.img)
        {
            held.push(plan);
        }
    }

    /// true when a kind of room was read of a shipped ship at all.
    pub fn knows(&self, kind: &str) -> bool {
        self.kinds.iter().any(|name| name == kind)
    }

    /// true when the shipped ships stand crew at a kind of room. A kind they do not -
    /// a battery, a cloak - is not filed a `<slot>` and keeps no station.
    pub fn stationed(&self, kind: &str) -> bool {
        self.plans.iter().any(|((name, _), held)| {
            name == kind && held.iter().any(|plan| plan.side.is_some())
        })
    }

    /// The configurations a kind of room was drawn in, in a shape of room, each with
    /// the art filed with it. Empty for a kind never drawn in that shape.
    pub fn plans(&self, kind: &str, shape: (i32, i32)) -> Vec<Plan> {
        let held: Vec<Plan> = match self.plans.get(&(kind.to_string(), shape)) {
            Some(held) => held.clone(),
            None => return Vec::new(),
        };
        let stood = self.stationed(kind);
        held
            .into_iter()
            .filter(|plan| plan.side.is_some() == stood)
            .collect()
    }

    /// The rows one system was drawn in, in a shape of room.
    ///
    /// The medbay of a thin bay blocks no tile, because the art of the room paints its
    /// machine and no tile of a two tile room is to be stood on (user rule, and six
    /// shipped blueprints), so the rows that file a tile are left out for it. The clone
    /// bay has no art of its own - there is no `room_clonebay` file - and draws its
    /// machine as a sprite on the tile it files, so it files a tile of its room even in
    /// a thin bay, which is how all 48 shipped clone bays do it and what `-2` there
    /// would break: the sprite lands outside the room, in the room above.
    pub fn plans_for(&self, system: &str, shape: (i32, i32)) -> Vec<Plan> {
        let mut rows = self.plans(system, shape);
        if system == "medbay" && shape != (2, 2) {
            rows.retain(|plan| plan.number < 0);
        }
        rows
    }

    /// Every configuration filed for a kind of room, in every shape, with nothing
    /// filtered out. What `src/bin/probe_stations.rs` audits a mod against.
    pub fn rows(&self, kind: &str) -> Vec<Plan> {
        self.plans
            .iter()
            .filter(|((name, _), _)| name == kind)
            .flat_map(|(_, held)| held.iter().cloned())
            .collect()
    }

    /// Every shape of room a kind of room was drawn in.
    pub fn shapes(&self, kind: &str) -> Vec<(i32, i32)> {
        self.plans
            .keys()
            .filter(|(name, _)| name == kind)
            .map(|(_, shape)| *shape)
            .collect()
    }
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
fn fits(kind: &str, room: &Room, doors: &[Door], stock: &Stock) -> Fit {
    if !stock.knows(kind) {
        return Fit::Clear;
    }
    let listed = stock.plans(kind, (room.w, room.h));
    if listed.is_empty() {
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

/// The blueprints the mod is not written for: the Kestrel of the tutorial and of the
/// easy start, which share the hull of Kestrel A and are not ships a player chooses.
/// Their systems still follow the hull when it is drawn, they just set no rule for
/// where a system may go.
pub const SKIP: [&str; 2] = ["PLAYER_SHIP_EASY", "PLAYER_SHIP_TUTORIAL"];

/// Note one shipped blueprint in the `stock`: every system of it, in the room the
/// blueprint filed it in, with the `<slot>` it filed - or the one the game would have
/// drawn in its place - and with the art it named. `rooms` calls this over the archive
/// before it changes a thing; `shapes` holds the shape of every room of the layout as
/// the shipped `.txt` has it.
pub fn gather(
    text: &str,
    blueprint: &Blueprint,
    shapes: &[(i32, i32)],
    stock: &mut Stock,
) -> Result<(), String> {
    if SKIP.contains(&blueprint.name.as_str()) {
        return Ok(());
    }
    for system in system_tags(&text[blueprint.span.clone()], blueprint.span.start)? {
        let tag = &text[system.tag.clone()];
        let shape = attribute(tag, "room")
            .and_then(|(_, value)| value.parse::<usize>().ok())
            .and_then(|room| shapes.get(room).copied());
        let Some((w, h)) = shape else { continue };
        let kind = share(&system.name);
        let default = DEFAULTS.iter().find(|(name, _, _)| *name == kind);
        // A system with no `<slot>` and no default of its name has neither a station
        // nor a machine, and the room it stands in still says what its art was cut for.
        let (number, side) = filed(text, &system)
            .or_else(|| default.and_then(|(_, wall, tile)| tile.map(|tile| (tile, *wall))))
            .unwrap_or((NO_SLOT, None));
        let img = attribute(tag, "img").map(|(_, value)| value.to_string());
        let plan = Plan { w, h, number, side, img };
        // A bay is one room, so the medbay and the clone bay say together what shapes
        // of room a bay takes. They are two machines though, and the shipped blueprints
        // file them apart in a thin bay, so each is read on its own as well.
        let kinds = match kind {
            BAY => [kind, system.name.as_str()].to_vec(),
            _ => vec![kind],
        };
        for kind in kinds {
            stock.file(kind, plan.clone());
        }
    }
    Ok(())
}

/// The tile and the wall a system files in its own `<slot>`, `None` when it files
/// none. A `<slot>` with a tile and no wall is a machine nobody stands at.
fn filed(text: &str, system: &Element) -> Option<(i32, Option<Side>)> {
    let inside = elements(&text[system.span.clone()], system.span.start).ok()?;
    inside
        .iter()
        .filter(|element| element.name == "slot")
        .find_map(|slot| slot_number(text, slot).map(|number| (number, slot_side(text, slot))))
}

/// Move the systems of every blueprint whose layout is in `layouts` into a room of
/// its own, and name the room art that fits the room it was handed. `Ok(None)` means
/// the file holds none of them.
pub fn randomize(
    text: &str,
    layouts: &BTreeMap<String, Layout>,
    stock: &Stock,
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
        assign(text, &blueprint, layout, stock, rng, &mut patches, &mut change)?;
        own_floor(text, &blueprint, &mut patches);
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

/// Point the deck of a ship at the art drawn for its own layout.
///
/// The eight `*_3` hulls file `<floorImage>kestral</floorImage>` and the like,
/// which is how the game is told to draw the floor of the A ship under a hull of
/// another shape. Those hulls ship no deck art of their own, and the floor they
/// borrow is cut for the rooms of the A layout, so it does not fit them. Every
/// layout is drawn a deck of its own now, so the name written here is the layout
/// of the blueprint - which is the name the game takes when a blueprint files no
/// `<floorImage>` at all, since the A and B hulls file none and are drawn from
/// `<layout>_floor.png`.
///
/// Only a blueprint that files the element is written. `<shieldImage>` and
/// `<cloakImage>` are left where they are: the shield ring and the cloak of a C
/// hull are the A ship's art and are not drawn from the rooms.
fn own_floor(text: &str, blueprint: &Blueprint, patches: &mut Vec<Patch>) {
    let Ok(inside) = elements(&text[blueprint.span.clone()], blueprint.span.start) else {
        return;
    };
    let Some(image) = inside.into_iter().find(|element| element.name == "floorImage") else {
        return;
    };
    // The name the game reads is the text between the two tags of the element.
    let close = match text[image.span.clone()].rfind("</") {
        Some(at) => image.span.start + at,
        None => return,
    };
    let value = image.tag.end..close;
    if &text[value.clone()] == blueprint.layout {
        return;
    }
    patches.push((value, blueprint.layout.clone()));
}

/// The systems of one blueprint, handed the rooms they got.
fn assign(
    text: &str,
    blueprint: &Blueprint,
    layout: &Layout,
    stock: &Stock,
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
    let room_of = match match_rooms(&groups, layout, stock, rng) {
        Some(rooms) => rooms,
        None => {
            change.warnings.push(format!(
                "{} cannot be housed in the {} rooms of {}: some systems stand in a room of a shape they were not drawn in",
                blueprint.name,
                layout.rooms.len(),
                blueprint.layout
            ));
            force_rooms(&groups, layout, stock, rng)
        }
    };

    // Then each system has its room, its station and its art written. One
    // configuration is picked per system rather than per room, because a bay is one
    // room with two machines: a 2x2 bay has them file one tile between them, and a thin
    // one files nothing on the medbay and a tile on the clone bay, which is how the
    // shipped blueprints file them.
    for (group, (kind, members)) in groups.iter().enumerate() {
        let room = room_of[group];
        let where_ = &layout.rooms[room];
        let mut shared: Option<i32> = None;
        for system in members {
            let key = match *kind {
                BAY => system.name.as_str(),
                other => other,
            };
            let picked = pick_plan(
                text,
                key,
                kind,
                system,
                where_,
                &layout.doors,
                shared,
                stock,
                rng,
                &mut change.warnings,
            );
            shared = shared.or_else(|| picked.as_ref().and_then(|plan| plan.tile()));
            let raw = &text[system.tag.clone()];
            let (value, _) = attribute(raw, "room").expect("systems were picked for their room");
            patches.push((
                system.tag.start + value.start..system.tag.start + value.end,
                room.to_string(),
            ));
            change.systems.push((system.name.clone(), room));
            if let Some(plan) = picked.as_ref() {
                write_img(text, system, plan.img.as_deref(), patches);
            }
            write_slot(
                text,
                system,
                picked.as_ref(),
                where_,
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
fn choices(kind: &str, layout: &Layout, worst: Fit, stock: &Stock) -> Vec<usize> {
    (0..layout.rooms.len())
        .filter(|&room| fits(kind, &layout.rooms[room], &layout.doors, stock) <= worst)
        .collect()
}

/// Every group handed a room of its own that its kind fits in, first with a wall the
/// door is off and, failing that, in any room of a shape the kind was drawn in.
/// `None` when the rooms do not cover the systems.
fn match_rooms(
    groups: &[(&str, Vec<&Element>)],
    layout: &Layout,
    stock: &Stock,
    rng: &mut Rng,
) -> Option<Vec<usize>> {
    let mut order: Vec<usize> = (0..groups.len()).collect();
    rng.shuffle(&mut order);
    for worst in [Fit::Clear, Fit::Door] {
        // The groups that can be housed in the fewest rooms go first, so a piloting
        // station is served while thin rooms are still going and a system with no
        // station, which fits anywhere, takes what is left.
        order.sort_by_key(|&group| choices(groups[group].0, layout, worst, stock).len());
        if let Some(rooms) = match_on(groups, layout, worst, &order, stock) {
            return Some(rooms);
        }
    }
    None
}

/// true when a layout houses every system of a blueprint, each group in a room of a
/// shape it was drawn in and, for the systems the crew stand at, with a wall the door
/// is off. `rooms::redraw` asks this before it keeps a layout it drew.
pub fn houses(text: &str, blueprint: &Blueprint, layout: &Layout, stock: &Stock) -> bool {
    let Ok(systems) = system_tags(&text[blueprint.span.clone()], blueprint.span.start) else {
        // A blueprint this module cannot read is left alone by `randomize`, so a
        // layout is not redrawn over it.
        return true;
    };
    let groups = groups(&systems);
    let mut order: Vec<usize> = (0..groups.len()).collect();
    order.sort_by_key(|&group| choices(groups[group].0, layout, Fit::Clear, stock).len());
    match_on(&groups, layout, Fit::Clear, &order, stock).is_some()
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
    stock: &Stock,
) -> Option<Vec<usize>> {
    let allowed: Vec<Vec<usize>> = groups
        .iter()
        .map(|(kind, _)| choices(kind, layout, worst, stock))
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
fn force_rooms(
    groups: &[(&str, Vec<&Element>)],
    layout: &Layout,
    stock: &Stock,
    rng: &mut Rng,
) -> Vec<usize> {
    let mut free: Vec<usize> = (0..layout.rooms.len()).collect();
    rng.shuffle(&mut free);
    let mut room_of: Vec<usize> = Vec::with_capacity(groups.len());
    for (kind, _) in groups.iter() {
        let best = match free.is_empty() {
            false => free.clone(),
            true => (0..layout.rooms.len()).collect(),
        };
        let mut room = best[0];
        let mut fit = fits(kind, &layout.rooms[room], &layout.doors, stock);
        for other in best {
            let better = fits(kind, &layout.rooms[other], &layout.doors, stock);
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

/// The `img` a system names, `None` when it names none.
fn image_of(text: &str, system: &Element) -> Option<String> {
    let (_, value) = attribute(&text[system.tag.clone()], "img")?;
    Some(value.to_string())
}

/// Write the art a plan names into a system: another file, no file at all when the
/// game is to draw the art of the system's own name, or the file it already named left
/// where it is.
fn write_img(text: &str, system: &Element, want: Option<&str>, patches: &mut Vec<Patch>) {
    let tag = &text[system.tag.clone()];
    match (attribute(tag, "img"), want) {
        (None, None) => {}
        (Some((_, held)), Some(want)) if held == want => {}
        // The file the blueprint named was cut for another shape of room.
        (Some((value, _)), Some(want)) => {
            let at = system.tag.start + value.start..system.tag.start + value.end;
            patches.push((at, want.to_string()));
        }
        // The room is to be drawn with the art of the system's own name, so the file
        // the blueprint named goes, attribute and all, so that the tag reads as though
        // it never named one.
        (Some((value, _)), None) => {
            let at = system.tag.start + value.start..system.tag.start + value.end;
            if let Some(span) = attribute_span(text, &at) {
                patches.push((span, String::new()));
            }
        }
        // This ship drew its room with the art of the system's own name, so it named no
        // file. The room it was handed is of a shape that art was not cut for, and a
        // file has to be named where none was.
        (None, Some(want)) => {
            // The file goes behind the room the tag already names, which is where the
            // blueprints keep it and where no other edit of this module writes: the end
            // of the tag is not such a spot, because a system that had no children is
            // opened up there for its `<slot>`, and that edit would swallow this one.
            if let Some((value, _)) = attribute(tag, "room") {
                let at = system.tag.start + value.end + 1;
                patches.push((at..at, format!(" img=\"{want}\"")));
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

/// The configuration and the art one system gets in the room it was handed, `None`
/// when nothing was read of the kind of room and the system is best left as it was
/// filed. `kind` is the row of the [`Stock`] to read, which is the system tag of a bay
/// machine and the kind of room of anything else; `group` is the kind of room, which is
/// what the defaults of the game are filed under.
///
/// The art and the station are picked together, out of one row of the [`Stock`],
/// because the art is painted around the station it shipped with: naming an art and a
/// wall of its own choosing is how a crew ends up manning a blank wall. The row that
/// keeps the art this ship filed comes first, then the row of the tile another machine
/// of the same room files, then the rows that file the wall and the tile this blueprint
/// filed, and one of the rest after that.
fn pick_plan(
    text: &str,
    kind: &str,
    group: &str,
    system: &Element,
    room: &Room,
    doors: &[Door],
    shared: Option<i32>,
    stock: &Stock,
    rng: &mut Rng,
    warnings: &mut Vec<String>,
) -> Option<Plan> {
    if !stock.knows(kind) {
        return None;
    }
    let shape = (room.w, room.h);
    let listed = stock.plans_for(kind, shape);
    let free: Vec<Plan> =
        listed.iter().filter(|plan| stands(plan, room, doors)).cloned().collect();
    let pool = if free.is_empty() {
        if listed.is_empty() {
            return least_worst(kind, group, room, doors, text, system, stock, warnings, rng);
        }
        warnings.push(format!(
            "{kind} has no wall here with the door off it, so its station faces a door"
        ));
        listed
    } else {
        free
    };

    // The art this ship filed, `None` when it filed none and the game was left to draw
    // the art of the system's own name. A row of the same art keeps a ship looking like
    // itself, and with it comes the station that art was painted around.
    let named = image_of(text, system);
    let pool = prefer(pool, |plan| plan.img == named);
    // The tile another machine of this room files. The medbay and the clone bay are one
    // machine and then the other, and 12 of the 13 shipped 2x2 bays that file one at all
    // file the same tile for both. A thin bay has no tile the two can share - the medbay
    // files none - and this rule then picks nothing.
    let pool = prefer(pool, |plan| shared.is_none_or(|number| plan.number == number));
    let (wall, tile) = wanted(text, group, system);
    let pool = prefer(pool, |plan| wall.is_none_or(|side| plan.side == Some(side)));
    let pool = prefer(pool, |plan| tile.is_none_or(|number| plan.number == number));
    Some(pool[rng.index(pool.len())].clone())
}

/// The entries of a pool a rule picks, or the whole pool when it picks none.
fn prefer(pool: Vec<Plan>, pick: impl Fn(&Plan) -> bool) -> Vec<Plan> {
    let kept: Vec<Plan> = pool.iter().filter(|plan| pick(plan)).cloned().collect();
    match kept.is_empty() {
        true => pool,
        false => kept,
    }
}

/// The wall and the tile to aim for: the ones the blueprint filed for the system, or
/// the ones the game would have drawn in their place.
fn wanted(text: &str, group: &str, system: &Element) -> (Option<Side>, Option<i32>) {
    let inside = elements(&text[system.span.clone()], system.span.start).unwrap_or_default();
    let slots: Vec<&Element> = inside.iter().filter(|element| element.name == "slot").collect();
    let wall = slots.iter().find_map(|slot| slot_side(text, slot));
    let tile = slots.iter().find_map(|slot| slot_number(text, slot));
    let default = DEFAULTS.iter().find(|(name, _, _)| *name == group);
    (
        wall.or(default.and_then(|(_, side, _)| *side)),
        tile.or(default.and_then(|(_, _, number)| *number)),
    )
}

/// The best that is left when a system was handed a room of a shape no shipped ship
/// draws it in: the wall rule alone, which at least keeps a console against a wall and
/// out of the middle of the floor, the machine of a thin bay filed off the floor, and
/// the art of the ship left where it was. Only a ship too big for its own rooms, or one
/// whose layout was kept after every redraw failed, comes here.
fn least_worst(
    kind: &str,
    group: &str,
    room: &Room,
    doors: &[Door],
    text: &str,
    system: &Element,
    stock: &Stock,
    warnings: &mut Vec<String>,
    rng: &mut Rng,
) -> Option<Plan> {
    warnings.push(format!(
        "a {kind} stands in a {}x{} room, a shape no shipped ship draws it in",
        room.w, room.h
    ));
    let img = image_of(text, system);
    if group == BAY {
        // The medbay blocks no tile; the clone bay has to file one, since its machine
        // is drawn on the tile it names and nowhere else.
        let number = match kind {
            "medbay" => -2,
            _ => 0,
        };
        return Some(Plan { w: room.w, h: room.h, number, side: None, img });
    }
    if !stock.stationed(kind) {
        return Some(Plan { w: room.w, h: room.h, number: NO_SLOT, side: None, img });
    }
    let mut spots = layout::stations(room, doors);
    if spots.is_empty() {
        spots = layout::station_tiles(room);
    }
    let spot = spots[rng.index(spots.len())];
    Some(Plan { w: room.w, h: room.h, number: spot.number, side: Some(spot.side), img })
}

/// Write the `<slot>` of a configuration into a system, or leave the numbers alone
/// in a system that has neither a station nor a machine.
///
/// A number names a tile of the room, so a number the new room does not have would
/// put a machine out of reach: it blocks nothing instead.
fn write_slot(
    text: &str,
    system: &Element,
    plan: Option<&Plan>,
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

    let Some(number) = plan.and_then(|plan| plan.tile()) else {
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
    let block = slot_block(text, system, plan.and_then(|plan| plan.side), number);
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
pub(crate) fn indent_before(text: &str, at: usize) -> String {
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
/// Public because `src/bin/probe_enemy_weapons.rs` reads a blueprint file the same
/// way the passes do.
///
/// quick-xml reports the byte just past each event, so a tag starts where its own
/// text says it does. That is checked against the source: a range that does not
/// spell the tag it claims is an error, not a guess.
pub fn elements(text: &str, base: usize) -> Result<Vec<Element>, String> {
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
pub(crate) fn rewrite(text: &str, patches: &[Patch]) -> Vec<u8> {
    let mut order: Vec<&Patch> = patches.iter().collect();
    order.sort_by_key(|patch| std::cmp::Reverse(patch.0.start));
    let mut out = text.as_bytes().to_vec();
    for (range, with) in order {
        out.splice(range.start..range.end, with.as_bytes().iter().copied());
    }
    out
}