//! Random room layouts for the enemy ships of an archive.
//!
//! An enemy ship is a layout file, a pair of hull images and a `<shipBlueprint>`,
//! and it differs from a player ship in two ways that decide how far it can be
//! redrawn:
//!
//! * the game paints its rooms itself over `img/ships_noglow/<img>_base.png`, so
//!   there is no hull art cut up around the rooms and no deck drawn under them;
//! * a `<shipBlueprint>` names the room of every system by number, and the game
//!   draws the look of a system for the shape of the room it stands in.
//!
//! So the rooms of an enemy ship are moved instead of drawn again. The layout keeps
//! its room count, the shape of every room and what stands in it, and only the
//! tiles the rooms stand on and the walls they share change - which is also why
//! nothing here touches the blueprints: a room that keeps its number and its shape
//! needs nothing moved in the lists that name it.
//!
//! The clusters the shipped layout joins with doors are kept as clusters. Some
//! enemy ships are not one walkable ship: `jelly_button` and `auto_assault` seal two
//! rooms off from everything else, and the bosses park a gun pod or two outside the
//! hull they have a door into. Rooms of one cluster have to be
//! walkable from each other in the new layout too, and rooms of two clusters never
//! get a door between them - so a sealed room stays sealed, somewhere else.
//!
//! The tiles a room may stand on are the tiles of the hull art that every art of the
//! ship paints, plus the tiles the shipped rooms already stand on. Nothing here touches the
//! filesystem, and only the layout file of a ship is written.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::blueprints;
use crate::layout::{self, Door, Room, Tile};
use crate::rng::Rng;
use crate::ship_txt::ShipTxt;
use crate::{rooms, FtlArchive};

/// What went wrong with an archive.
#[derive(Debug)]
pub enum Error {
    /// A file an enemy ship needs is not in the archive.
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

/// One enemy layout whose rooms were moved.
#[derive(Debug)]
pub struct Ship {
    pub layout: String,
    /// The hull art the rooms were measured against.
    pub art: Vec<String>,
    /// Tiles of the hull a room may stand on.
    pub hull: usize,
    pub rooms: usize,
    /// Clusters of rooms the shipped layout joined with doors.
    pub clusters: usize,
    pub doors: usize,
    /// Rooms that ended up on other tiles than they started on. A small ship whose
    /// hull has no spare tile leaves this at nought.
    pub moved: usize,
    /// The blueprints that fly this layout.
    pub blueprints: Vec<String>,
}

/// What a run changed.
pub struct Report {
    pub ships: Vec<Ship>,
    /// Archive paths that were written.
    pub changed: Vec<String>,
    /// Anything that could not be done cleanly.
    pub warnings: Vec<String>,
}

/// Give every enemy ship of `archive` a new layout: its rooms, standing on new
/// tiles of its own hull, connected the same way they were.
pub fn randomize(archive: &mut FtlArchive, rng: &mut Rng) -> Result<Report> {
    let mut warnings = Vec::new();
    let wanted = requests(archive, &mut warnings)?;

    let mut ships: Vec<Ship> = Vec::new();
    let mut changed: Vec<String> = Vec::new();
    for (layout, request) in &wanted {
        let Some(ship) = redraw(archive, rng, layout, request, &mut warnings)? else {
            continue;
        };
        changed.push(layout_txt(layout));
        ships.push(ship);
    }

    Ok(Report {
        ships,
        changed,
        warnings,
    })
}

/// What a layout has to be told before its rooms are moved.
#[derive(Default)]
struct Request {
    /// The art names of the blueprints that fly the layout. One layout can be flown
    /// by several ships, and every hull they are drawn with has to keep its rooms
    /// inside it.
    art: BTreeSet<String>,
    /// The blueprints that fly the layout.
    blueprints: Vec<String>,
    /// The room of each gun pod of a boss, by its place in the boss's gun list. The
    /// pods have to keep their left to right order and stand clear of one another.
    pods: BTreeMap<usize, usize>,
    /// Set when two blueprints of this layout put the same pod in two rooms, which
    /// leaves no order to keep.
    pods_lost: bool,
}

/// Every layout the enemy ships fly, and what each of them needs.
fn requests(archive: &FtlArchive, warnings: &mut Vec<String>) -> Result<BTreeMap<String, Request>> {
    let mut wanted: BTreeMap<String, Request> = BTreeMap::new();
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
        for element in blueprints::elements(&text, 0)
            .map_err(|why| Error::Unreadable { path: path.clone(), why })?
        {
            if element.name != "shipBlueprint" {
                continue;
            }
            let tag = &text[element.tag.clone()];
            let Some((_, name)) = blueprints::attribute(tag, "name") else { continue };
            let Some((_, layout)) = blueprints::attribute(tag, "layout") else { continue };
            // A player hull files art of its own in `img/ship`, and `rooms.rs` has
            // drawn its rooms already: this pass is for the ships drawn from
            // `img/ships_noglow`, which have none.
            if archive.get_file(&base_image(layout)).is_some() {
                continue;
            }
            let entry = wanted.entry(layout.to_string()).or_default();
            entry.blueprints.push(name.to_string());
            if let Some((_, img)) = blueprints::attribute(tag, "img") {
                entry.art.insert(img.to_string());
            }
            let inside = blueprints::system_tags(&text[element.span.clone()], element.span.start)
                .map_err(|why| Error::Unreadable { path: path.clone(), why })?;
            for system in inside {
                let Some((rank, room)) = pod(&text[system.tag.clone()]) else { continue };
                match entry.pods.get(&rank) {
                    Some(&other) if other != room => {
                        warnings.push(format!(
                            "{layout}: {name} files gun pod {rank} in room {room}, another \
                             blueprint of the same layout files it in room {other}, so the \
                             pods keep no order"
                        ));
                        entry.pods_lost = true;
                    }
                    _ => {
                        entry.pods.insert(rank, room);
                    }
                }
            }
        }
    }
    Ok(wanted)
}

/// The gun pod of a boss: the room of an `<artillery weapon="ARTILLERY_BOSS_n">`,
/// with the rank the pod's number gives it, first pod being rank 0.
fn pod(tag: &str) -> Option<(usize, usize)> {
    let (rank, room) = (
        blueprints::attribute(tag, "weapon")?
            .1
            .strip_prefix("ARTILLERY_BOSS_")?
            .parse::<usize>()
            .ok()?
            - 1,
        blueprints::attribute(tag, "room")?
            .1
            .parse::<usize>()
            .ok()?,
    );
    Some((rank, room))
}

/// Move the rooms of one layout and write its layout file back. `Ok(None)` when the
/// archive holds no hull art to measure the ship against.
fn redraw(
    archive: &mut FtlArchive,
    rng: &mut Rng,
    layout: &str,
    request: &Request,
    warnings: &mut Vec<String>,
) -> Result<Option<Ship>> {
    let txt_path = layout_txt(layout);
    let bytes = archive
        .get_file(&txt_path)
        .ok_or_else(|| Error::Missing { path: txt_path.clone() })?
        .to_vec();
    let mut txt = ShipTxt::parse(bytes).map_err(|why| Error::Unreadable {
        path: txt_path.clone(),
        why,
    })?;
    let xml = archive
        .get_file(&layout_xml(layout))
        .ok_or_else(|| Error::Missing { path: layout_xml(layout) })?;
    let Some(box_of_ship) = txt.bounds() else {
        warnings.push(format!("{layout} files no rooms, so nothing was moved"));
        return Ok(None);
    };
    let (low_x, low_y, _, _) = box_of_ship;
    let was = txt.rooms.clone();
    let Some((mask, art)) = hull_tiles(archive, layout, request, xml, &was, warnings)? else {
        return Ok(None);
    };

    // The rooms keep their number, so the anchor is where the shipped layout began:
    // the game hangs the hull art off the low corner of the room grid, and a layout
    // that starts further right or lower slides the ship on the screen.
    let anchor = (low_x, low_y);
    let cluster = clusters(&txt);
    let rank = ranks(&txt, request);
    let plan = layout::Fixed {
        rooms: &was,
        mask: &mask,
        cluster: &cluster,
        anchor,
        rank: &rank,
    };
    let moved = layout::rearrange(&plan, rng);
    let shifted = moved
        .iter()
        .zip(&was)
        .filter(|(now, then)| now.x != then.x || now.y != then.y)
        .count();
    // The airlocks go with the rooms they stand in rather than being drawn afresh:
    // of the 41 enemy layouts the game ships, 36 file none at all.
    let airlocks = carried(&was, &moved, &txt.doors);
    let mut doors = layout::cluster_doors(&moved, &cluster, rng);
    doors.extend(airlocks);
    let clusters = cluster.iter().copied().collect::<BTreeSet<_>>().len();
    let rooms = was.len();
    txt.set_layout(moved, doors);
    archive.set_file(&txt_path, txt.to_bytes());

    if shifted == 0 {
        warnings.push(format!(
            "{layout}: its hull has no tile spare for any of its {rooms} rooms, so the \
             layout was left as it ships"
        ));
    }
    Ok(Some(Ship {
        layout: layout.to_string(),
        art,
        hull: mask.len(),
        rooms,
        clusters,
        doors: txt.doors.len(),
        moved: shifted,
        blueprints: request.blueprints.clone(),
    }))
}

/// The tiles a room of the layout may stand on: every tile all the hull art of the
/// ship paints, plus the tiles the shipped rooms stand on.
///
/// The second half of that is not a detail: the art of a small enemy ship stops
/// short of the rooms the game ships with it, and a room cannot be asked to keep
/// inside paint that was never laid down. `Ok(None)` is a layout with no art to
/// read at all.
fn hull_tiles(
    archive: &FtlArchive,
    layout: &str,
    request: &Request,
    xml: &[u8],
    shipped: &[Room],
    warnings: &mut Vec<String>,
) -> Result<Option<(BTreeSet<Tile>, Vec<String>)>> {
    let offsets =
        rooms::art_offsets(xml).map_err(|why| Error::Unreadable {
            path: layout_xml(layout),
            why,
        })?;
    let mut names: Vec<String> = request.art.iter().cloned().collect();
    if names.is_empty() {
        names.push(layout.to_string());
    }

    let mut painted: Option<BTreeSet<Tile>> = None;
    let mut read: Vec<String> = Vec::new();
    for name in names {
        let path = art_image(&name);
        let Some(bytes) = archive.get_file(&path) else {
            warnings.push(format!(
                "{layout}: the archive has no {path}, so its hull was not measured"
            ));
            continue;
        };
        let art = image::load_from_memory(bytes).map_err(|error| Error::Unreadable {
            path: path.clone(),
            why: error.to_string(),
        })?;
        let art = art.into_rgba8();
        // The art of an enemy ship is cropped to its own paint and is drawn in the
        // middle of the box `<img w h>` declares, so the crop has to be put back
        // before the tiles the hull paints can be read off it.
        let origin = layout::art_origin(offsets.img, offsets.size, art.dimensions());
        let mask = layout::hull_paint(&art, origin, PAINT);
        painted = Some(match painted {
            Some(had) => had.intersection(&mask).copied().collect(),
            None => mask,
        });
        read.push(name);
    }
    let Some(painted) = painted else {
        warnings.push(format!(
            "{layout}: no hull art of its own, so its rooms stayed where they are"
        ));
        return Ok(None);
    };

    let mut mask = painted;
    mask.extend(shipped.iter().flat_map(Room::tiles));
    Ok(Some((mask, read)))
}

/// How much of a tile the hull art has to paint for a room to be put on it, out of
/// the `TILE * TILE` pixels of the tile.
///
/// Three quarters is what the game itself does: asked for the pixels under the rooms
/// of the 41 enemy layouts, 11 of them have a room tile with less than nine tenths
/// of its paint on the hull, and the least painted of them - `rebel_long_pirate` -
/// stands a fifth of its rooms on tiles that are barely a quarter hull.
pub const PAINT: usize = (layout::TILE * layout::TILE) as usize * 3 / 4;

/// Which cluster each room belongs to: the rooms a door of the shipped layout joins,
/// walked from room to room. A room no door touches is a cluster of its own, which
/// is how the small slug ends up with three.
fn clusters(txt: &ShipTxt) -> Vec<usize> {
    let count = txt.rooms.len();
    let mut near: Vec<Vec<usize>> = vec![Vec::new(); count];
    for door in &txt.doors {
        if door.airlock() {
            continue;
        }
        let (one, two) = (door.left_top as usize, door.right_bottom as usize);
        // A door joins two rooms of its own file. Anything else is left to its own
        // cluster rather than guessed at.
        if one >= count || two >= count {
            continue;
        }
        near[one].push(two);
        near[two].push(one);
    }
    let mut cluster = vec![usize::MAX; count];
    let mut next = 0;
    for start in 0..count {
        if cluster[start] != usize::MAX {
            continue;
        }
        cluster[start] = next;
        let mut queue = vec![start];
        while let Some(current) = queue.pop() {
            for &other in &near[current] {
                if cluster[other] == usize::MAX {
                    cluster[other] = next;
                    queue.push(other);
                }
            }
        }
        next += 1;
    }
    cluster
}

/// The rank every room has to keep in the left to right order of the gun pods,
/// `None` for a room that is no pod.
///
/// The rank comes from the column the shipped layout stood in, not from the number
/// the blueprint gives the pod, so the shuffle keeps the pods in the order the game
/// ships them in and clear of one another - which is also what makes the rule always
/// satisfiable, the shipped layout obeying it already. Pods that share a column share
/// a rank, since no order between them can be kept.
fn ranks(txt: &ShipTxt, request: &Request) -> Vec<Option<usize>> {
    if request.pods_lost {
        return vec![None; txt.rooms.len()];
    }
    let mut order: Vec<(i32, usize, usize)> = request
        .pods
        .iter()
        .filter_map(|(rank, room)| txt.rooms.get(*room).map(|was| (was.x, *rank, *room)))
        .collect();
    order.sort_unstable();
    let mut rank = vec![None; txt.rooms.len()];
    let mut column = 0;
    for (place, (x, _, room)) in order.iter().enumerate() {
        if place > 0 && *x != order[place - 1].0 {
            column += 1;
        }
        rank[*room] = Some(column);
    }
    rank
}

/// The airlocks of the shipped layout, carried along to where their room went. One
/// is dropped when a room now stands on the tile it faces: a door between two rooms
/// cannot name `-1`.
fn carried(was: &[Room], rooms: &[Room], doors: &[Door]) -> Vec<Door> {
    let used: BTreeSet<Tile> = rooms.iter().flat_map(Room::tiles).collect();
    doors
        .iter()
        .filter(|door| door.airlock())
        .filter_map(|door| {
            let room = usize::try_from(if door.left_top == -1 {
                door.right_bottom
            } else {
                door.left_top
            })
            .ok()?;
            let (before, now) = (was.get(room)?, rooms.get(room)?);
            let moved = Door {
                x: door.x + now.x - before.x,
                y: door.y + now.y - before.y,
                ..*door
            };
            // The door divides two tiles, of which the room owns one. What lies the
            // other side has to be open still.
            let (one, two) = moved.tiles();
            let owns = |room: &Room, tile: Tile| room.tiles().any(|held| held == tile);
            let outside = if owns(now, one) { two } else { one };
            (!used.contains(&outside)).then_some(moved)
        })
        .collect()
}

/// true when `hay` holds `needle`.
fn holds(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|window| window == needle)
}

/// The player hull art, which is what tells a player layout from an enemy one.
fn base_image(layout: &str) -> String {
    format!("img/ship/{layout}_base.png")
}

/// The hull art an enemy ship is drawn with. The game draws the rooms themselves
/// over it, and `img/ships_glow` holds the same picture to glow behind it.
fn art_image(art: &str) -> String {
    format!("img/ships_noglow/{art}_base.png")
}

/// The rooms and doors of a layout.
fn layout_txt(layout: &str) -> String {
    format!("data/{layout}.txt")
}

/// The hull art placement of a layout.
fn layout_xml(layout: &str) -> String {
    format!("data/{layout}.xml")
}