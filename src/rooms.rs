//! Random room layouts for the player ships of an archive.
//!
//! A player ship lives in four places at once:
//! * `img/ship/<layout>_base.png`, the hull art, which only player ships have -
//!   that is how the 28 of them are picked out of the hundreds of blueprints;
//! * `data/<layout>.txt`, the rooms, the doors and the airlocks;
//! * `data/<layout>.xml`, where the hull art sits relative to those rooms;
//! * a `<shipBlueprint>` in one of the blueprint files, which names the room every
//!   system sits in.
//!
//! So one ship is redrawn in two files, and the second one depends on the first:
//! the systems can only be handed rooms once the rooms exist. Blueprints come
//! before drawing here for that reason, and so does the art: the tiles a room may
//! cover are the painted tiles of the hull, inside the box the shipped layout used.
//!
//! Nothing here touches the filesystem, and nothing is copied more than the payload
//! that changes: the archive is edited in place.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use quick_xml::events::Event;
use quick_xml::reader::Reader;

use crate::blueprints::{self, Blueprint};
use crate::layout::{self, Room};
use crate::rng::Rng;
use crate::ship_txt::ShipTxt;
use crate::FtlArchive;

/// Rooms left over for corridors, cargo and boarding parties, on top of the one
/// every system needs.
const SPARE_ROOMS: usize = 3;

/// What went wrong with an archive.
#[derive(Debug)]
pub enum Error {
    /// A file a player ship needs is not in the archive.
    Missing { path: String },
    /// A file that could not be read. `why` says what the file is and what it was
    /// not, and the path says where to look.
    Unreadable { path: String, why: String },
    /// The archive itself.
    Archive(crate::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Missing { path } => write!(out, "the archive has no {path}"),
            Error::Unreadable { path, why } => write!(out, "{path}: {why}"),
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

/// One ship layout that was redrawn.
#[derive(Debug)]
pub struct Ship {
    pub layout: String,
    /// Tiles of the hull a room may cover.
    pub hull: usize,
    pub rooms: usize,
    pub doors: usize,
    pub airlocks: usize,
    /// The blueprints that drive this layout, and had their systems moved.
    pub blueprints: Vec<String>,
}

/// What a run changed.
pub struct Report {
    pub ships: Vec<Ship>,
    /// Archive paths that were written, art and untouched files excluded.
    pub changed: Vec<String>,
    /// Systems moved, per blueprint, for whoever wants to see the rooms they got.
    pub moves: Vec<blueprints::Change>,
    /// Anything that could not be done cleanly.
    pub warnings: Vec<String>,
}

/// Give every player ship of `archive` a new layout, and move its systems into the
/// rooms of that layout.
pub fn randomize(archive: &mut FtlArchive, rng: &mut Rng) -> Result<Report> {
    let mut warnings = Vec::new();

    // The files that hold blueprints, and the blueprints they hold.
    let mut texts: BTreeMap<String, String> = BTreeMap::new();
    let mut lists: BTreeMap<String, Vec<Blueprint>> = BTreeMap::new();
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
        lists.insert(path.clone(), blueprints::blueprints(&text).map_err(|why| Error::Unreadable { path: path.clone(), why })?);
        texts.insert(path, text);
    }

    // The layouts to draw. A layout belongs to a player ship when the game holds a
    // base image for it, which enemy ships do not have.
    let mut wanted: BTreeMap<String, LayoutRequest> = BTreeMap::new();
    for (path, text) in &texts {
        for blueprint in lists[path].iter() {
            if archive.get_file(&base_image(&blueprint.layout)).is_none() {
                continue;
            }
            let need = blueprints::rooms_needed(&text[blueprint.span.clone()])
                .map_err(|why| Error::Unreadable { path: path.clone(), why })?
                + SPARE_ROOMS;
            let entry = wanted
                .entry(blueprint.layout.clone())
                .or_insert_with(|| LayoutRequest { rooms: 0, blueprints: Vec::new() });
            entry.rooms = entry.rooms.max(need);
            entry.blueprints.push(blueprint.name.clone());
        }
    }

    // One layout per ship, and the rooms its systems will be handed out of.
    let mut rooms_of: BTreeMap<String, Vec<Room>> = BTreeMap::new();
    let mut ships: Vec<Ship> = Vec::new();
    let mut changed: Vec<String> = Vec::new();
    for (layout, request) in &wanted {
        let txt_path = layout_txt(layout);
        let ship = redraw(archive, rng, layout, request, &txt_path)?;
        warnings.extend(ship.note.clone());
        rooms_of.insert(layout.clone(), ship.rooms.clone());
        changed.push(txt_path);
        ships.push(ship.ship);
    }

    // Then the systems, into those rooms.
    let mut moves = Vec::new();
    for (path, text) in &texts {
        let edit = blueprints::randomize(text, &rooms_of, rng)
            .map_err(|why| Error::Unreadable { path: path.clone(), why })?;
        let Some(edit) = edit else { continue };
        for change in &edit.changes {
            warnings.extend(change.warnings.iter().cloned());
        }
        moves.extend(edit.changes);
        archive.set_file(path, edit.bytes);
        changed.push(path.clone());
    }

    for ship in &mut ships {
        ship.blueprints.sort();
        ship.blueprints.dedup();
        let request = &wanted[&ship.layout];
        if ship.rooms < request.rooms {
            warnings.push(format!(
                "{} holds {} rooms, {} were asked for: some systems share a room",
                ship.layout, ship.rooms, request.rooms
            ));
        }
    }

    Ok(Report {
        ships,
        changed,
        moves,
        warnings,
    })
}

/// A layout, and why it has to be at least this big.
struct LayoutRequest {
    rooms: usize,
    blueprints: Vec<String>,
}

/// What `redraw` made of a ship.
struct Drawn {
    ship: Ship,
    rooms: Vec<Room>,
    /// Set when the hull art does not reach the box the layout shipped in.
    note: Option<String>,
}

/// Draw one ship layout and write it over the one in the archive.
fn redraw(
    archive: &mut FtlArchive,
    rng: &mut Rng,
    layout: &str,
    request: &LayoutRequest,
    txt_path: &str,
) -> Result<Drawn> {
    let txt = archive
        .get_file(txt_path)
        .ok_or_else(|| Error::Missing { path: txt_path.to_string() })?
        .to_vec();
    let xml = archive
        .get_file(&layout_xml(layout))
        .ok_or_else(|| Error::Missing { path: layout_xml(layout) })?;
    let art = archive
        .get_file(&base_image(layout))
        .ok_or_else(|| Error::Missing { path: base_image(layout) })?
        .to_vec();

    let mut txt = ShipTxt::parse(txt).map_err(|why| Error::Unreadable {
        path: txt_path.to_string(),
        why,
    })?;
    let art = image::load_from_memory(&art)
        .map_err(|error| Error::Unreadable {
            path: base_image(layout),
            why: error.to_string(),
        })?
        .to_rgba8();
    let (img_x, img_y) = img_offset(xml).map_err(|why| Error::Unreadable {
        path: layout_xml(layout),
        why,
    })?;

    // Tile (0, 0) is the corner of the hull art, slid by the pixel offsets the
    // layout file asks for. Rooms are stored as tiles, so this is all the art and
    // the grid have to agree on.
    let origin = (
        -img_x + txt.number("HORIZONTAL").unwrap_or(0),
        -img_y + txt.number("VERTICAL").unwrap_or(0),
    );
    let mask = layout::hull_mask(&art, origin);
    // Rooms stay inside the box the shipped layout used and start from its low
    // corner: the game hangs the hull art off the low corner of the room grid, so
    // rooms that begin further right or lower slide the ship art sideways. No
    // layout in the game puts a room below tile 0 either.
    let box_of_ship = txt.bounds();
    let mask: BTreeSet<(i32, i32)> = match box_of_ship {
        Some((low_x, low_y, high_x, high_y)) => mask
            .into_iter()
            .filter(|(x, y)| *x >= low_x && *x <= high_x && *y >= low_y && *y <= high_y)
            .collect(),
        None => mask,
    };
    // Where the art stops short of the box, its own corner is the furthest a room
    // can go, and the art will sit that much off. Worth a warning either way.
    let anchor = match box_of_ship {
        Some((low_x, low_y, _, _)) => (
            mask.iter().map(|(x, _)| *x).min().unwrap_or(low_x).max(low_x),
            mask.iter().map(|(_, y)| *y).min().unwrap_or(low_y).max(low_y),
        ),
        None => (0, 0),
    };
    let note = match box_of_ship {
        Some((low_x, low_y, _, _)) if (low_x, low_y) != anchor => Some(format!(
            "{}: the hull art stops at tile {},{} instead of {},{}, so its rooms start inside the box it shipped in",
            layout, anchor.0, anchor.1, low_x, low_y
        )),
        _ => None,
    };
    let drawn = layout::generate(&mask, rng, request.rooms, anchor);
    let rooms = drawn.rooms.clone();
    let doors = drawn.doors.iter().filter(|door| !door.airlock()).count();
    let airlocks = drawn.doors.len() - doors;

    txt.set_layout(drawn.rooms, drawn.doors);
    archive.set_file(txt_path, txt.to_bytes());

    Ok(Drawn {
        ship: Ship {
            layout: layout.to_string(),
            hull: mask.len(),
            rooms: rooms.len(),
            doors,
            airlocks,
            blueprints: request.blueprints.clone(),
        },
        rooms,
        note,
    })
}

/// Where the hull art of a ship sits: the `<img x= y=>` of its `<layout>.xml`.
fn img_offset(xml: &[u8]) -> std::result::Result<(i32, i32), String> {
    let mut reader = Reader::from_reader(xml);
    let mut buffer = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(tag)) | Ok(Event::Empty(tag)) if tag.local_name().as_ref() == b"img" => {
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
                return match (x, y) {
                    (Some(x), Some(y)) => Ok((x, y)),
                    _ => Err("<img> has no x and y".to_string()),
                };
            }
            Ok(Event::Eof) => return Err("no <img> in the layout file".to_string()),
            Ok(_) => buffer.clear(),
            Err(error) => return Err(error.to_string()),
        }
    }
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

/// The rooms and doors of a layout.
fn layout_txt(layout: &str) -> String {
    format!("data/{layout}.txt")
}

/// The hull art placement of a layout.
fn layout_xml(layout: &str) -> String {
    format!("data/{layout}.xml")
}