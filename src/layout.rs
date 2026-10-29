//! Random ship layouts: room rectangles, the doors between the rooms, and
//! airlocks.
//!
//! Nothing in here checks or repairs what it made, because the steps cannot get
//! it wrong: a room is only ever placed on free tiles the hull allows, every new
//! room is pushed against a tile that is already used so the room graph is
//! connected from the second room onwards, the linking step keeps adding doors
//! until one walk reaches every room, and an open side two tiles wide only has
//! "no airlock" and "both airlocks" to draw from. The one thing a single growth
//! pass can miss is the room count, so `grow` repeats the pass until it lands.
//!
//! The other half of this module moves the rooms of an enemy layout instead of
//! growing new ones, because an enemy ship has to keep its rooms: [`rearrange`]
//! walks the rooms onto new tiles and [`cluster_doors`] doors them, and both only
//! ever leave a layout that is as legal as the one it started from.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use image::RgbaImage;

use crate::rng::Rng;

/// One tile of the ship grid. Coordinates can be negative: the grid is not the
/// image it is drawn on.
pub type Tile = (i32, i32);

/// One room tile is 35 by 35 pixels of hull art.
pub const TILE: i32 = 35;

/// The room count range a ship can get.
pub const MIN_ROOMS: usize = 15;
pub const MAX_ROOMS: usize = 25;
/// Growth gives up after this many tries in a row that find no room, which is
/// how a full hull ends a pass.
const FULL_HULL_TRIES: u32 = 1500;
/// Placements compared before each room goes down. The one whose middle sits
/// furthest from the middle of the rooms already down wins, which is what
/// spreads a layout over the ship instead of piling it next to the seed room.
const CANDIDATES: usize = 12;
/// Randomness added to that choice, so ships do not all grow the same way.
const JITTER: u64 = 14;
/// Tiles nearest the middle of the hull that may hold the seed room.
const SEED_TRIES: usize = 12;
/// Growth passes a hull is given to reach its room count.
const GROWTH_PASSES: usize = 400;
/// Sweeps of rooms an enemy layout is given when it is shuffled. Each sweep picks
/// every room up and puts it down again somewhere that keeps the layout legal, so
/// the walk cannot wander into a broken layout and needs no repair; the sweeps are
/// what carries it away from the layout it started from.
const SHUFFLE_SWEEPS: usize = 30;
/// How often one layout may be slid towards its anchor before the pass is given
/// up on. Each slide moves a room or the whole layout, so a handful is plenty.
const SETTLE_TRIES: usize = 8;
/// How far a room may slide along the wall it is lifted onto the anchor with.
const CROSS_REACH: i32 = 3;

/// The room shapes, in the order a placement tries them from a random start.
/// Five of the eight starts are a 2x2 block, so a block is tried first two times
/// out of three and the thin 1x2 rooms take the parts of the hull no block fits.
const SHAPES: [(i32, i32); 8] = [(2, 2), (1, 2), (2, 1), (2, 2), (2, 2), (1, 2), (2, 1), (2, 2)];

/// Doors in the wall between two rooms, by the length of that wall in tiles: the
/// entry is the number of doors. Measured on the 28 player layouts by
/// `src/bin/probe_door_stats.rs`. A wall is never longer than two tiles, because
/// no room is wider than two.
const WALL_DOOR_ODDS: &[&[u32]] = &[&[120, 880], &[220, 520, 270]];

/// Airlocks on an open run of a room side: run length in tiles, then the kind of
/// open space the run faces - `d` is inner hull deck no room took, `s` is outside
/// the ship, `m` is both - then the patterns to draw from, one letter per tile of
/// the run, `A` for an airlock and `.` for none. Same source as above.
///
/// A run two tiles wide takes both airlocks or neither. The shipped layouts break
/// that about once in twenty runs, which reads as sloppiness rather than a shape,
/// so the pair is a rule here and the odd singles are folded into the pair.
const AIRLOCK_ODDS: &[(usize, char, &[&str], &[u32])] = &[
    (1, 'd', &[".", "A"], &[966, 34]),
    (1, 's', &[".", "A"], &[940, 60]),
    (2, 'd', &["..", "AA"], &[860, 140]),
    (2, 's', &["..", "AA"], &[780, 220]),
    (2, 'm', &["..", "AA"], &[770, 230]),
];

/// A room, as a rectangle of tiles. Its id is its place in the room list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Room {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// A door, as the game `DOOR` record stores it. A vertical door sits on the left
/// wall of tile `(x, y)` and divides it from `(x - 1, y)`; a horizontal door sits
/// on the top wall of that tile and divides it from `(x, y - 1)`. A room id of
/// `-1` means the other side is open space, which makes the door an airlock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Door {
    pub x: i32,
    pub y: i32,
    pub left_top: i32,
    pub right_bottom: i32,
    /// true for a wall that runs up and down, so it divides two rooms left and right.
    pub vertical: bool,
}

/// A whole layout: the rooms, and every door of the ship. A door names a room by
/// its place in `rooms`.
pub struct Layout {
    pub rooms: Vec<Room>,
    pub doors: Vec<Door>,
}

/// A random layout for one hull. `mask` is the tiles a room may cover, from
/// [`hull_mask`]. `min_rooms` is the room count the ship has to reach, which is
/// how many systems it has to house; a hull too small for it gets the most rooms
/// it can hold.
///
/// `anchor` is the low corner the rooms have to start from, the low x and low y of
/// the layout being replaced. The game hangs the hull art off that corner, so a
/// layout that starts further right or lower than the shipped one slides the ship
/// art sideways. The far edges are free to end wherever the hull lets them.
pub fn generate(mask: &BTreeSet<Tile>, rng: &mut Rng, min_rooms: usize, anchor: Tile) -> Layout {
    let target = room_target(mask.len(), rng, min_rooms);
    let rooms = grow(mask, rng, target, anchor);
    let mut doors = wall_doors(&rooms, None, rng);
    link_rooms(&rooms, None, &mut doors, rng);
    doors.append(&mut airlocks(&rooms, mask, rng));
    Layout { rooms, doors }
}

/// Tiles a room may cover in a base image: every `TILE * TILE` pixel of the tile
/// has alpha 255. Strict on purpose, it keeps rooms clear of the deck art, the
/// crystal spikes and the hull edge, and it costs only a few tiles.
///
/// `origin` is the pixel of tile `(0, 0)` inside the image, which is
/// `x = -img_x + HORIZONTAL`, `y = -img_y + VERTICAL`: the corner of the art from
/// `<img>` in `<ship>.xml`, moved by the two keys of `<ship>.txt`.
pub fn hull_mask(base: &RgbaImage, origin: (i32, i32)) -> BTreeSet<Tile> {
    hull_paint(base, origin, (TILE * TILE) as usize)
}

/// The tiles of a base image with `painted` of their `TILE * TILE` pixels at full
/// alpha. [`hull_mask`] is the case that asks for every pixel of the tile.
///
/// A player hull paints its rooms, so every tile of its inside is painted whole and
/// nothing is given away by asking for all of them. The hull of an enemy ship is a
/// silhouette cut on the tile grid, and the layouts the game ships with it stand on
/// tiles that miss that silhouette by a bevel: asked for every pixel, 10 of the 41
/// enemy layouts are left with no spare tile at all, and a room of such a layout has
/// nowhere to be moved to. `src/bin/probe_enemy_rooms.rs` walks the layouts.
pub fn hull_paint(base: &RgbaImage, origin: (i32, i32), painted: usize) -> BTreeSet<Tile> {
    let (width, height) = base.dimensions();
    let mut mask = BTreeSet::new();
    for ty in -2..=(height as i32 / TILE + 2) {
        for tx in -2..=(width as i32 / TILE + 2) {
            if painted_over(base, origin, tx, ty) >= painted {
                mask.insert((tx, ty));
            }
        }
    }
    mask
}

/// How many pixels of one tile of a base image are painted at full alpha. `TILE *
/// TILE` when the tile stands whole inside the paint.
pub fn painted_over(base: &RgbaImage, origin: (i32, i32), tx: i32, ty: i32) -> usize {
    let (width, height) = base.dimensions();
    let (x0, y0) = (origin.0 + tx * TILE, origin.1 + ty * TILE);
    if x0 < 0 || y0 < 0 || x0 + TILE > width as i32 || y0 + TILE > height as i32 {
        return 0;
    }
    (y0..y0 + TILE)
        .flat_map(|y| (x0..x0 + TILE).map(move |x| (x, y)))
        .filter(|&(x, y)| base.get_pixel(x as u32, y as u32)[3] == 255)
        .count()
}

/// Where a hull art file stands on the room grid: the pixel of the file that sits
/// on grid pixel `(0, 0)`.
///
/// `<img x y>` names the corner of the box the layout file declares for the hull
/// and `<img w h>` the size of that box. The art of a player ship fills its box,
/// so the corner of the file is the corner of the box and this is [`hull_mask`]'s
/// origin unchanged. The art of an enemy ship is cropped to its own paint, and the
/// game centres the file in the declared box; the crop has to be put back before
/// anyone can tell which tiles the hull paints. Measured over the 41 enemy layouts:
/// with the centre kept, every tile under their rooms has paint on it and 0.8% of
/// the pixels of their rooms fall off the hull, with the corner kept instead it is
/// 11%.
pub fn art_origin(img: (i32, i32), declared: Option<(i32, i32)>, file: (u32, u32)) -> (i32, i32) {
    let crop = |pair: (i32, i32)| {
        (
            (pair.0 - file.0 as i32) / 2,
            (pair.1 - file.1 as i32) / 2,
        )
    };
    let (left, top) = declared.map(crop).unwrap_or((0, 0));
    (-(img.0 + left), -(img.1 + top))
}

// ------------------------------------------------------------------- rooms --

impl Room {
    /// The tiles the room covers, row by row from the top left, which is the
    /// order the blueprint numbers the slots of a room in.
    pub fn tiles(&self) -> impl Iterator<Item = Tile> {
        (self.y..self.y + self.h).flat_map(move |y| (self.x..self.x + self.w).map(move |x| (x, y)))
    }
}

/// How many rooms a hull of this size should get: a little under one room per
/// three tiles, plus or minus two, kept inside [`MIN_ROOMS`] to [`MAX_ROOMS`] and
/// never under what the ship's systems need.
fn room_target(hull_tiles: usize, rng: &mut Rng, min_rooms: usize) -> usize {
    let floor = min_rooms.clamp(MIN_ROOMS, MAX_ROOMS) as i64;
    let asked = hull_tiles as i64 * 3 / 10 + rng.below(5) as i64 - 2;
    asked.clamp(floor, MAX_ROOMS as i64) as usize
}

/// Grow a layout, repeat the growth pass until it reaches the target room count
/// and stands on both anchor lines. Every new room is pushed against a tile that
/// is already used, so it shares a wall with a room that is already down and
/// nothing ever has to be checked afterwards. What a pass can get wrong is the
/// count: a pass that fills the hull in a clumsy order has nowhere left for the
/// last room, which happens on the small hulls where three tenths of the tiles
/// rounds up to the minimum of 15. Roughly one ship in ten on the 28 player hulls.
/// Each pass draws a fresh layout, so the shortest one is only kept when none of
/// them reaches the count.
fn grow(mask: &BTreeSet<Tile>, rng: &mut Rng, target: usize, anchor: Tile) -> Vec<Room> {
    let mut best: Vec<Room> = Vec::new();
    let mut best_placed = false;
    for _ in 0..GROWTH_PASSES {
        let mut rooms = grow_once(mask, rng, target);
        settle(&mut rooms, mask, anchor, rng);
        let placed = reaches(&rooms, anchor) && rooms.len() >= target;
        if (placed, rooms.len()) > (best_placed, best.len()) {
            best = rooms;
            best_placed = placed;
        }
        if best_placed {
            break;
        }
    }
    best
}

/// true when some room stands on each anchor line: one on the low column, one on
/// the low row.
fn reaches(rooms: &[Room], anchor: Tile) -> bool {
    !rooms.is_empty()
        && rooms.iter().any(|room| room.x <= anchor.0)
        && rooms.iter().any(|room| room.y <= anchor.1)
}

/// Bring a finished layout onto its anchor, which most passes already are. True
/// when both lines ended up covered.
fn settle(rooms: &mut Vec<Room>, mask: &BTreeSet<Tile>, anchor: Tile, rng: &mut Rng) -> bool {
    for _ in 0..rooms.len().min(SETTLE_TRIES) {
        if reaches(rooms, anchor) || !slide(rooms, mask, anchor, rng) {
            break;
        }
    }
    reaches(rooms, anchor)
}

/// One step towards the anchor: the whole layout slides first, which keeps the
/// ship as it was drawn, and failing that one room is lifted onto the line that is
/// short. False when nothing could move, which ends the settling.
fn slide(rooms: &mut Vec<Room>, mask: &BTreeSet<Tile>, anchor: Tile, rng: &mut Rng) -> bool {
    let low_x = rooms.iter().map(|room| room.x).min().unwrap_or(0);
    let low_y = rooms.iter().map(|room| room.y).min().unwrap_or(0);
    let (need_x, need_y) = (anchor.0 - low_x, anchor.1 - low_y);

    for (dx, dy) in [(need_x, need_y), (need_x, 0), (0, need_y)] {
        if (dx, dy) == (0, 0) {
            continue;
        }
        let shifted: Vec<Room> = rooms.iter().map(|room| shift(*room, dx, dy)).collect();
        if shifted
            .iter()
            .all(|room| room.tiles().all(|tile| mask.contains(&tile)))
        {
            *rooms = shifted;
            return true;
        }
    }

    for (axis, need) in [(0, need_x), (1, need_y)] {
        if need >= 0 {
            continue;
        }
        // The rooms nearest the short line are the ones with the least to move.
        // Shuffled first, because `sort_by_key` is stable and a key that asked the
        // generator for a tie-break would not be a total order.
        let mut order: Vec<usize> = (0..rooms.len()).collect();
        rng.shuffle(&mut order);
        order.sort_by_key(|&index| if axis == 0 { rooms[index].x } else { rooms[index].y });
        for index in order {
            let room = rooms[index];
            let used: BTreeSet<Tile> = rooms
                .iter()
                .enumerate()
                .filter(|(at, _)| *at != index)
                .flat_map(|(_, other)| other.tiles())
                .collect();
            let along = if axis == 0 { room.y } else { room.x };
            let steps: Vec<i32> = (0..=CROSS_REACH)
                .flat_map(|step| [along + step, along - step])
                .collect();
            for offset in steps {
                let moved = if axis == 0 {
                    Room {
                        x: anchor.0,
                        y: offset,
                        ..room
                    }
                } else {
                    Room {
                        x: offset,
                        y: anchor.1,
                        ..room
                    }
                };
                if !fits(&moved, mask, &used) {
                    continue;
                }
                let mut trial = rooms.clone();
                trial[index] = moved;
                if linked(&trial) {
                    *rooms = trial;
                    return true;
                }
            }
        }
    }
    false
}

fn shift(room: Room, dx: i32, dy: i32) -> Room {
    Room {
        x: room.x + dx,
        y: room.y + dy,
        ..room
    }
}

/// true when every room shares a wall with the rest of the ship, walking room to
/// room from the first one.
fn linked(rooms: &[Room]) -> bool {
    if rooms.len() < 2 {
        return true;
    }
    let owner = owner_of(rooms);
    let mut near: Vec<Vec<usize>> = vec![Vec::new(); rooms.len()];
    for (&tile, &index) in &owner {
        for delta in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            if let Some(&other) = owner.get(&(tile.0 + delta.0, tile.1 + delta.1))
                && other != index
            {
                near[index].push(other);
            }
        }
    }
    let mut seen = vec![false; rooms.len()];
    let mut queue = vec![0];
    seen[0] = true;
    let mut walked = 1;
    while let Some(index) = queue.pop() {
        for &other in &near[index] {
            if !seen[other] {
                seen[other] = true;
                walked += 1;
                queue.push(other);
            }
        }
    }
    walked == rooms.len()
}

/// One growth pass.
fn grow_once(mask: &BTreeSet<Tile>, rng: &mut Rng, target: usize) -> Vec<Room> {
    let mut used: BTreeSet<Tile> = BTreeSet::new();
    let mut taken: Vec<Tile> = Vec::new();
    let mut rooms: Vec<Room> = Vec::new();

    // The seed room goes near the middle of the hull, so the layout can grow out
    // in every direction instead of hugging one corner.
    let middle = centre(mask.iter().copied());
    let mut nearest: Vec<(i64, Tile)> = mask
        .iter()
        .map(|tile| (distance2(wide(*tile), middle), *tile))
        .collect();
    nearest.sort_by_key(|(away, tile)| (*away, *tile));
    for (_, tile) in nearest.iter().take(SEED_TRIES) {
        let start = rng.index(SHAPES.len());
        let room = (0..SHAPES.len())
            .map(|step| SHAPES[(start + step) % SHAPES.len()])
            .map(|(w, h)| Room { x: tile.0, y: tile.1, w, h })
            .find(|room| fits(room, mask, &used));
        if let Some(room) = room {
            take(&mut used, &mut taken, &mut rooms, room);
            break;
        }
    }

    let mut misses = 0;
    while !taken.is_empty() && rooms.len() < target && misses < FULL_HULL_TRIES {
        let filled = centre(taken.iter().copied());
        let mut best: Option<(i64, Room)> = None;
        for _ in 0..CANDIDATES {
            let Some(room) = candidate(&taken, mask, &used, rng) else {
                continue;
            };
            let score = distance2(room_centre(&room), filled) + rng.below(JITTER) as i64;
            if best.is_none_or(|(top, _)| score > top) {
                best = Some((score, room));
            }
        }
        match best {
            Some((_, room)) => {
                take(&mut used, &mut taken, &mut rooms, room);
                misses = 0;
            }
            None => misses += 1,
        }
    }
    rooms
}

/// The next room, pushed against a random side of a random tile that is used. The
/// shape is picked from a random start and gives way to the next one when it does
/// not fit, so a thin part of the hull still gets a room. The room also slides
/// along the wall it rests on, so it does not have to sit square against the tile
/// it is anchored to.
fn candidate(
    taken: &[Tile],
    mask: &BTreeSet<Tile>,
    used: &BTreeSet<Tile>,
    rng: &mut Rng,
) -> Option<Room> {
    let (ax, ay) = taken.get(rng.index(taken.len()))?;
    let side = rng.index(4);
    let start = rng.index(SHAPES.len());
    for step in 0..SHAPES.len() {
        let (w, h) = SHAPES[(start + step) % SHAPES.len()];
        // How far the new room may slide along the wall it rests on.
        let slide = if side < 2 { h } else { w };
        for shift in 0..slide {
            let room = match side {
                0 => Room { x: ax + 1, y: ay - shift, w, h },    // right of the tile
                1 => Room { x: ax - w, y: ay - shift, w, h },    // left of it
                2 => Room { x: ax - shift, y: ay + 1, w, h },    // below it
                _ => Room { x: ax - shift, y: ay - h, w, h },    // above it
            };
            if fits(&room, mask, used) {
                return Some(room);
            }
        }
    }
    None
}

/// True when every tile of the room is allowed by the hull and still free.
fn fits(room: &Room, mask: &BTreeSet<Tile>, used: &BTreeSet<Tile>) -> bool {
    room.tiles().all(|tile| mask.contains(&tile) && !used.contains(&tile))
}

fn take(used: &mut BTreeSet<Tile>, taken: &mut Vec<Tile>, rooms: &mut Vec<Room>, room: Room) {
    used.extend(room.tiles());
    taken.extend(room.tiles());
    rooms.push(room);
}

/// Middle of a group of tiles.
fn centre(tiles: impl Iterator<Item = Tile>) -> (i64, i64) {
    let tiles: Vec<Tile> = tiles.collect();
    if tiles.is_empty() {
        return (0, 0);
    }
    (
        tiles.iter().map(|tile| tile.0 as i64).sum::<i64>() / tiles.len() as i64,
        tiles.iter().map(|tile| tile.1 as i64).sum::<i64>() / tiles.len() as i64,
    )
}

/// Middle of one room.
fn room_centre(room: &Room) -> (i64, i64) {
    (
        room.x as i64 + (room.w as i64 - 1) / 2,
        room.y as i64 + (room.h as i64 - 1) / 2,
    )
}

/// Squared distance in tiles. Only the ranking is used, so no square root.
fn distance2(a: (i64, i64), b: (i64, i64)) -> i64 {
    (a.0 - b.0) * (a.0 - b.0) + (a.1 - b.1) * (a.1 - b.1)
}

fn wide(tile: Tile) -> (i64, i64) {
    (tile.0 as i64, tile.1 as i64)
}

/// Which room owns each tile, in tile order. Walking this order is what puts the
/// tiles of a wall in order, which is what makes a seed replay a layout.
fn owner_of(rooms: &[Room]) -> BTreeMap<Tile, usize> {
    let mut owner = BTreeMap::new();
    for (index, room) in rooms.iter().enumerate() {
        for tile in room.tiles() {
            owner.insert(tile, index);
        }
    }
    owner
}

// ------------------------------------------------------------------- doors --

/// A pair of tiles that share a wall: one tile of a room and the tile on the
/// other side.
type Pair = (Tile, Tile);

impl Room {
    /// The tiles of one side of the room and the tiles next to them, outside it.
    fn side(&self, side: Side) -> Vec<Pair> {
        match side {
            Side::Up => (0..self.w).map(|i| ((self.x + i, self.y), (self.x + i, self.y - 1))).collect(),
            Side::Down => (0..self.w)
                .map(|i| ((self.x + i, self.y + self.h - 1), (self.x + i, self.y + self.h)))
                .collect(),
            Side::Left => (0..self.h).map(|i| ((self.x, self.y + i), (self.x - 1, self.y + i))).collect(),
            Side::Right => (0..self.h)
                .map(|i| ((self.x + self.w - 1, self.y + i), (self.x + self.w, self.y + i)))
                .collect(),
        }
    }
}

/// One side of a room.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Side {
    Up,
    Down,
    Left,
    Right,
}

const SIDES: [Side; 4] = [Side::Up, Side::Down, Side::Left, Side::Right];

impl Side {
    /// The name the blueprint files give to a station that faces this way.
    pub fn name(&self) -> &'static str {
        match self {
            Side::Up => "up",
            Side::Down => "down",
            Side::Left => "left",
            Side::Right => "right",
        }
    }

    /// The side a blueprint names, `None` for a word it does not use.
    pub fn parse(name: &str) -> Option<Side> {
        SIDES.into_iter().find(|side| side.name() == name)
    }

    /// The step from a tile to the tile on the other side of this wall.
    fn step(&self) -> Tile {
        match self {
            Side::Up => (0, -1),
            Side::Down => (0, 1),
            Side::Left => (-1, 0),
            Side::Right => (1, 0),
        }
    }

    /// true when the tile sits on this side of the room, so it touches the wall.
    fn holds(&self, room: &Room, tile: Tile) -> bool {
        match self {
            Side::Up => tile.1 == room.y,
            Side::Down => tile.1 == room.y + room.h - 1,
            Side::Left => tile.0 == room.x,
            Side::Right => tile.0 == room.x + room.w - 1,
        }
    }
}

/// The door record of the wall between two touching tiles, with the room id of
/// each side. Position and ids come from the two tiles, so they cannot disagree
/// with the geometry: `left_top` is the room that owns the left or top tile, and
/// the record sits on the tile that is lower or to the right.
///
/// An airlock is the one place the two ids are not the two sides: every airlock
/// record the game ships names its room first and `-1` second, whichever side the
/// room is on, so that is what is written here too.
fn door_record(one: Tile, id_one: i32, two: Tile, id_two: i32) -> Door {
    let (airlock, low, high) = if id_one == -1 || id_two == -1 {
        let room = if id_one == -1 { id_two } else { id_one };
        (true, room, -1)
    } else {
        (false, id_two, id_one)
    };
    if one.0 == two.0 {
        // One tile above the other, so the door lies across them.
        let above_first = one.1 < two.1;
        Door {
            x: one.0,
            y: if above_first { two.1 } else { one.1 },
            left_top: if airlock { low } else if above_first { id_one } else { id_two },
            right_bottom: if airlock { high } else if above_first { id_two } else { id_one },
            vertical: false,
        }
    } else {
        let left_first = one.0 < two.0;
        Door {
            x: if left_first { two.0 } else { one.0 },
            y: one.1,
            left_top: if airlock { low } else if left_first { id_one } else { id_two },
            right_bottom: if airlock { high } else if left_first { id_two } else { id_one },
            vertical: true,
        }
    }
}

impl Door {
    /// true when one side is open space, which makes the door an airlock.
    pub fn airlock(&self) -> bool {
        self.left_top == -1 || self.right_bottom == -1
    }

    /// The two tiles the door divides, low tile first.
    pub fn tiles(&self) -> Pair {
        if self.vertical {
            ((self.x - 1, self.y), (self.x, self.y))
        } else {
            ((self.x, self.y - 1), (self.x, self.y))
        }
    }

    /// true when the door sits on the wall two touching tiles share.
    pub fn separates(&self, one: Tile, two: Tile) -> bool {
        let (a, b) = self.tiles();
        (a == one && b == two) || (a == two && b == one)
    }
}

/// Every shared wall, keyed by the two rooms it divides, holding one pair per
/// tile of contact. Rooms that only touch at a corner share no wall, so no door.
/// In a pair the tile of the lower room id comes first, so the two ids a door
/// takes from a pair land on the right tiles.
fn walls(rooms: &[Room]) -> BTreeMap<(usize, usize), Vec<Pair>> {
    let owner = owner_of(rooms);
    let mut walls = BTreeMap::new();
    for (&tile, &room) in &owner {
        for delta in [(1, 0), (0, 1)] {
            let other = (tile.0 + delta.0, tile.1 + delta.1);
            if let Some(&neighbour) = owner.get(&other)
                && neighbour != room
            {
                let (pair, spot) = if room < neighbour {
                    ((room, neighbour), (tile, other))
                } else {
                    ((neighbour, room), (other, tile))
                };
                walls.entry(pair).or_insert_with(Vec::new).push(spot);
            }
        }
    }
    walls
}

/// The doors between rooms. Each wall draws its own count from
/// [`WALL_DOOR_ODDS`], and the spots the doors go on are drawn too.
///
/// `cluster` is `Some` for a layout whose rooms are clustered, like an enemy ship
/// whose oxygen room is sealed off from the rest: two rooms of two clusters share
/// no door, because a door would put the two clusters on one walkable ship.
fn wall_doors(rooms: &[Room], cluster: Option<&[usize]>, rng: &mut Rng) -> Vec<Door> {
    let mut doors = Vec::new();
    for ((left, right), wall) in walls(rooms) {
        if cluster.is_some_and(|group| group[left] != group[right]) {
            continue;
        }
        let count = rng.pick(WALL_DOOR_ODDS[wall.len() - 1]);
        for spot in spots(count, wall.len(), rng) {
            let (one, two) = wall[spot];
            doors.push(door_record(one, left as i32, two, right as i32));
        }
    }
    doors
}

/// Add doors until one walk over built doors reaches every room. The wall odds
/// above can leave a room with no door at all. This only ever adds, and it always
/// finishes, because the rooms are connected through shared walls by how they
/// were grown - and, with a `cluster`, because each cluster was walked into one
/// connected group of rooms, which is what [`rearrange`] guarantees.
fn link_rooms(rooms: &[Room], cluster: Option<&[usize]>, doors: &mut Vec<Door>, rng: &mut Rng) {
    let walls = walls(rooms);
    loop {
        let group = groups(rooms.len(), doors);
        // The first wall that joins two groups, in room id order.
        let Some((pair, wall)) = walls.iter().find(|((a, b), _)| {
            cluster.is_none_or(|at| at[*a] == at[*b]) && group[*a] != group[*b]
        }) else {
            return;
        };
        // A wall between two groups cannot be full of doors, a door would have
        // joined them already.
        let free: Vec<Pair> = wall
            .iter()
            .copied()
            .filter(|(one, two)| !doors.iter().any(|door| door.separates(*one, *two)))
            .collect();
        let spot = free[rng.index(free.len())];
        doors.push(door_record(spot.0, pair.0 as i32, spot.1, pair.1 as i32));
    }
}

/// Which group each room belongs to when you only walk through built doors.
/// Rooms with no door between them are in different groups.
fn groups(count: usize, doors: &[Door]) -> Vec<usize> {
    let mut group = vec![usize::MAX; count];
    let mut next = 0;
    for start in 0..count {
        if group[start] != usize::MAX {
            continue;
        }
        group[start] = next;
        let mut queue = VecDeque::from([start]);
        while let Some(current) = queue.pop_front() {
            for door in doors.iter().filter(|door| !door.airlock()) {
                let other = match (door.left_top as usize, door.right_bottom as usize) {
                    (left, right) if left == current => right,
                    (left, right) if right == current => left,
                    _ => continue,
                };
                if group[other] == usize::MAX {
                    group[other] = next;
                    queue.push_back(other);
                }
            }
        }
        next += 1;
    }
    group
}

/// The airlocks, worked out side by side rather than room by room, because a door
/// is only an airlock when what stands on the other side is open space.
fn airlocks(rooms: &[Room], mask: &BTreeSet<Tile>, rng: &mut Rng) -> Vec<Door> {
    let owner = owner_of(rooms);
    let mut doors = Vec::new();
    for (index, room) in rooms.iter().enumerate() {
        for side in SIDES {
            for run in open_runs(&room.side(side), &owner) {
                let deck = run.iter().filter(|(_, outside)| mask.contains(outside)).count();
                let kind = match deck {
                    0 => 's',
                    taken if taken == run.len() => 'd',
                    _ => 'm',
                };
                let (_, _, patterns, odds) = AIRLOCK_ODDS
                    .iter()
                    .find(|(length, key, _, _)| *length == run.len() && *key == kind)
                    .unwrap();
                for (spot, letter) in patterns[rng.pick(odds)].chars().enumerate() {
                    if letter == 'A' {
                        let (inside, outside) = run[spot];
                        doors.push(door_record(inside, index as i32, outside, -1));
                    }
                }
            }
        }
    }
    doors
}

/// The open runs of a side: stretches of room tiles whose outside neighbour is
/// not a room. A tile another room took breaks the run, because the door there is
/// a door between two rooms, not an airlock.
fn open_runs(cells: &[Pair], owner: &BTreeMap<Tile, usize>) -> Vec<Vec<Pair>> {
    let mut runs = Vec::new();
    let mut run: Vec<Pair> = Vec::new();
    for cell in cells {
        if owner.contains_key(&cell.1) {
            if !run.is_empty() {
                runs.push(std::mem::take(&mut run));
            }
        } else {
            run.push(*cell);
        }
    }
    if !run.is_empty() {
        runs.push(run);
    }
    runs
}

/// `count` different spots out of `total`, in random order.
fn spots(count: usize, total: usize, rng: &mut Rng) -> Vec<usize> {
    let mut pool: Vec<usize> = (0..total).collect();
    let mut chosen = Vec::with_capacity(count);
    for _ in 0..count {
        chosen.push(pool.swap_remove(rng.index(pool.len())));
    }
    chosen
}

// ------------------------------------------------------------- moved rooms --

/// A layout whose rooms are moved rather than grown: the same rooms, in the same
/// order and of the same shape, standing on new tiles of the same hull.
///
/// A room keeps its place in the list and its size because a blueprint names the
/// room every system of an enemy ship sits in, and the game draws the look of a
/// system for the shape of the room it stands in.
pub struct Fixed<'a> {
    /// The rooms of the layout as it ships, in id order.
    pub rooms: &'a [Room],
    /// The tiles a room may stand on.
    pub mask: &'a BTreeSet<Tile>,
    /// Which cluster each room belongs to, one entry per room. The rooms of one
    /// cluster have to be walkable from each other through the walls they share,
    /// and rooms of two clusters must never be joined by a door.
    pub cluster: &'a [usize],
    /// The low corner of the box the layout ships in. The game hangs the hull art
    /// off the low corner of the rooms, so the shuffled layout has to begin on both
    /// of its lines and step neither left of nor above them, or the whole ship
    /// slides sideways.
    pub anchor: Tile,
    /// `Some(rank)` for the rooms that have to keep their left to right order - the
    /// boss's gun pods - with the lowest rank furthest left, and `None` for every
    /// other room.
    pub rank: &'a [Option<usize>],
}

/// Move the rooms of a layout about inside the hull they already fit in.
///
/// Each sweep takes the rooms in a random order and stands each one on a place it
/// can go to without breaking the layout. The layout the walk starts from is
/// legal, so every layout it passes through is legal, and a hull with nowhere to
/// put a room hands back the one it was given.
pub fn rearrange(fixed: &Fixed, rng: &mut Rng) -> Vec<Room> {
    let mut rooms = fixed.rooms.to_vec();
    for _ in 0..SHUFFLE_SWEEPS {
        let mut order: Vec<usize> = (0..rooms.len()).collect();
        rng.shuffle(&mut order);
        for index in order {
            if let Some(moved) = park(fixed, &rooms, index, rng) {
                rooms[index] = moved;
            }
        }
    }
    rooms
}

/// Where room `index` can be put down that keeps the layout legal, every place but
/// the one it stands on, drawn at random. `None` when there is nowhere else for it.
fn park(fixed: &Fixed, rooms: &[Room], index: usize, rng: &mut Rng) -> Option<Room> {
    let room = rooms[index];
    let (low, high) = board(fixed.mask);
    let mut free: Vec<Room> = Vec::new();
    for y in low.1..=high.1 - room.h + 1 {
        for x in low.0..=high.0 - room.w + 1 {
            let trial = Room { x, y, ..room };
            if trial != room && holds(fixed, rooms, index, trial) {
                free.push(trial);
            }
        }
    }
    if free.is_empty() {
        return None;
    }
    let pick = rng.index(free.len());
    Some(free.swap_remove(pick))
}

/// true when the layout is legal with room `index` standing at `trial`: inside the
/// hull, with nothing on top of it, its cluster walkable, the ranked rooms in order
/// and the box of the layout still beginning on its two anchor lines.
fn holds(fixed: &Fixed, rooms: &[Room], index: usize, trial: Room) -> bool {
    let at = |pick: usize| -> Room {
        if pick == index { trial } else { rooms[pick] }
    };
    if !trial.tiles().all(|tile| fixed.mask.contains(&tile)) {
        return false;
    }
    if (0..rooms.len()).any(|other| other != index && overlaps(&trial, &rooms[other])) {
        return false;
    }
    // The hull art hangs off the low corner of the rooms, so the layout has to keep
    // beginning on both anchor lines: no room may step left of or above one, and one
    // room has to stay on each of them.
    let lines = (0..rooms.len()).fold((true, true, false, false), |state, other| {
        let room = at(other);
        (
            state.0 && room.x >= fixed.anchor.0,
            state.1 && room.y >= fixed.anchor.1,
            state.2 || room.x == fixed.anchor.0,
            state.3 || room.y == fixed.anchor.1,
        )
    });
    if !lines.0 || !lines.1 || !lines.2 || !lines.3 {
        return false;
    }
    walkable(fixed, rooms, index, trial) && ranked(rooms, index, trial, fixed.rank)
}

/// true when the cluster room `index` belongs to stays walkable with that room at
/// `trial`. The other clusters did not move and were walkable already.
fn walkable(fixed: &Fixed, rooms: &[Room], index: usize, trial: Room) -> bool {
    let group = fixed.cluster[index];
    let plan: Vec<Room> = (0..rooms.len())
        .filter(|&at| fixed.cluster[at] == group)
        .map(|at| if at == index { trial } else { rooms[at] })
        .collect();
    let mut seen = vec![false; plan.len()];
    let mut queue = vec![0];
    seen[0] = true;
    let mut walked = 1;
    while let Some(at) = queue.pop() {
        for other in 0..plan.len() {
            if seen[other] || !touching(&plan[at], &plan[other]) {
                continue;
            }
            seen[other] = true;
            walked += 1;
            queue.push(other);
        }
    }
    walked == plan.len()
}

/// true when the ranked rooms stand furthest-left-first, each one further right
/// than the room its rank follows. Rooms of no rank take no part.
fn ranked(rooms: &[Room], index: usize, trial: Room, rank: &[Option<usize>]) -> bool {
    let mut order: Vec<(usize, i32)> = rooms
        .iter()
        .enumerate()
        .filter_map(|(at, room)| {
            let stand = if at == index { trial } else { *room };
            rank[at].map(|value| (value, stand.x))
        })
        .collect();
    order.sort_unstable();
    order.windows(2).all(|pair| pair[0].1 < pair[1].1)
}

/// The corners of the box a set of tiles fills, `(low, high)` both inclusive.
fn board(tiles: &BTreeSet<Tile>) -> (Tile, Tile) {
    let start = tiles.iter().next().copied().unwrap_or((0, 0));
    tiles.iter().fold((start, start), |(low, high), &tile| {
        (
            (low.0.min(tile.0), low.1.min(tile.1)),
            (high.0.max(tile.0), high.1.max(tile.1)),
        )
    })
}

/// true when two rooms share a wall. Touching at a corner alone is not enough:
/// there is no door to draw in a corner.
fn touching(one: &Room, two: &Room) -> bool {
    let side = (one.x + one.w == two.x || two.x + two.w == one.x)
        && meets(one.y, one.h, two.y, two.h);
    let across = (one.y + one.h == two.y || two.y + two.h == one.y)
        && meets(one.x, one.w, two.x, two.w);
    side || across
}

/// true when two rooms stand on one tile.
fn overlaps(one: &Room, two: &Room) -> bool {
    meets(one.x, one.w, two.x, two.w) && meets(one.y, one.h, two.y, two.h)
}

/// true when the run `a .. a + length` reaches into the run `b .. b + length`.
fn meets(a: i32, length_a: i32, b: i32, length_b: i32) -> bool {
    a < b + length_b && b < a + length_a
}

/// The doors of a layout whose rooms are clustered: a door only in a wall two
/// rooms of one cluster share, and enough of them for each cluster to be walkable
/// on its own. No airlock: of the 41 enemy layouts the game ships, 36 file none,
/// and the rooms of a shuffled layout stand on new tiles, so the few the others
/// file are moved along with their room instead of drawn afresh.
pub fn cluster_doors(rooms: &[Room], cluster: &[usize], rng: &mut Rng) -> Vec<Door> {
    let group = Some(cluster);
    let mut doors = wall_doors(rooms, group, rng);
    link_rooms(rooms, group, &mut doors, rng);
    doors
}

// ------------------------------------------------------------------ stations --

/// Where a station may stand: one tile of a room, and the wall of it the crew
/// face.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Station {
    /// The tile as the blueprint numbers them.
    pub number: i32,
    pub side: Side,
}

/// The stations the shape of a room allows, whatever stands outside it.
///
/// A station is drawn on the wall the crew face, so it has to face a wall of its
/// own room and never the room itself: only a tile of the top row may face up,
/// only a tile of the left column may face left, and so on. A tile of a corner
/// allows both its walls, which is what the four corners of a 2x2 room give eight
/// stations, and a tile of a 1x2 room, being the corner twice over, three.
pub fn station_tiles(room: &Room) -> Vec<Station> {
    room_stations(room, None)
}

/// The stations a room allows with its doors: [`station_tiles`] without the ones
/// that face a door, airlock included. A crew member stood in a doorway is in
/// everyone's way, and the console hides the door.
pub fn stations(room: &Room, doors: &[Door]) -> Vec<Station> {
    room_stations(room, Some(doors))
}

fn room_stations(room: &Room, doors: Option<&[Door]>) -> Vec<Station> {
    let mut spots = Vec::new();
    for (number, tile) in room.tiles().enumerate() {
        for side in SIDES {
            if !side.holds(room, tile) {
                continue;
            }
            if let Some(doors) = doors {
                let step = side.step();
                let other = (tile.0 + step.0, tile.1 + step.1);
                if doors.iter().any(|door| door.separates(tile, other)) {
                    continue;
                }
            }
            spots.push(Station { number: number as i32, side });
        }
    }
    spots
}
