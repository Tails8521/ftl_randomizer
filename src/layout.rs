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

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use image::RgbaImage;

use crate::rng::Rng;

/// One tile of the ship grid. Coordinates can be negative: the grid is not the
/// image it is drawn on.
pub type Tile = (i32, i32);

/// One room tile is 35 by 35 pixels of hull art.
pub const TILE: i32 = 35;

/// The room count range a ship can get.
const MIN_ROOMS: i64 = 15;
const MAX_ROOMS: i64 = 25;
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
/// [`hull_mask`].
pub fn generate(mask: &BTreeSet<Tile>, rng: &mut Rng) -> Layout {
    let target = room_target(mask.len(), rng);
    let rooms = grow(mask, rng, target);
    let mut doors = wall_doors(&rooms, rng);
    link_rooms(&rooms, &mut doors, rng);
    doors.append(&mut airlocks(&rooms, mask, rng));
    Layout { rooms, doors }
}

/// Tiles a room may cover in a base image: every `TILE * TILE` pixel of the tile
/// has alpha 255. Strict on purpose, it keeps rooms clear of the deck art and of
/// the crystal spikes, and it costs only a few tiles.
///
/// `origin` is the pixel of tile `(0, 0)` inside the image, which is
/// `x = -img_x + HORIZONTAL`, `y = -img_y + VERTICAL`: the corner of the art from
/// `<img>` in `<ship>.xml`, moved by the two keys of `<ship>.txt`.
pub fn hull_mask(base: &RgbaImage, origin: (i32, i32)) -> BTreeSet<Tile> {
    let (width, height) = base.dimensions();
    let mut mask = BTreeSet::new();
    for ty in -2..=(height as i32 / TILE + 2) {
        for tx in -2..=(width as i32 / TILE + 2) {
            if opaque(base, origin, tx, ty) {
                mask.insert((tx, ty));
            }
        }
    }
    mask
}

fn opaque(base: &RgbaImage, origin: (i32, i32), tx: i32, ty: i32) -> bool {
    let (width, height) = base.dimensions();
    let (x0, y0) = (origin.0 + tx * TILE, origin.1 + ty * TILE);
    if x0 < 0 || y0 < 0 || x0 + TILE > width as i32 || y0 + TILE > height as i32 {
        return false;
    }
    (y0..y0 + TILE)
        .flat_map(|y| (x0..x0 + TILE).map(move |x| (x, y)))
        .all(|(x, y)| base.get_pixel(x as u32, y as u32)[3] == 255)
}

// ------------------------------------------------------------------- rooms --

impl Room {
    /// The tiles the room covers.
    pub fn tiles(&self) -> impl Iterator<Item = Tile> {
        (self.y..self.y + self.h).flat_map(move |y| (self.x..self.x + self.w).map(move |x| (x, y)))
    }
}

/// How many rooms a hull of this size should get: a little under one room per
/// three tiles, plus or minus two, kept inside [`MIN_ROOMS`] to [`MAX_ROOMS`].
fn room_target(hull_tiles: usize, rng: &mut Rng) -> usize {
    (hull_tiles as i64 * 3 / 10 + rng.below(5) as i64 - 2).clamp(MIN_ROOMS, MAX_ROOMS) as usize
}

/// Grow a layout, repeat the growth pass until it reaches the target room count.
/// Every new room is pushed against a tile that is already used, so it shares a
/// wall with a room that is already down and nothing ever has to be checked
/// afterwards. What a pass can get wrong is the count: a pass that fills the hull
/// in a clumsy order has nowhere left for the last room, which happens on the
/// small hulls where three tenths of the tiles rounds up to the minimum of 15.
/// Roughly one ship in ten on the 28 player hulls. Each pass draws a fresh
/// layout, so the shortest one is only kept when none of them reaches the count.
fn grow(mask: &BTreeSet<Tile>, rng: &mut Rng, target: usize) -> Vec<Room> {
    let mut best: Vec<Room> = Vec::new();
    for _ in 0..GROWTH_PASSES {
        let rooms = grow_once(mask, rng, target);
        if rooms.len() > best.len() {
            best = rooms;
        }
        if best.len() >= target {
            break;
        }
    }
    best
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

#[derive(Clone, Copy)]
enum Side {
    Up,
    Down,
    Left,
    Right,
}

const SIDES: [Side; 4] = [Side::Up, Side::Down, Side::Left, Side::Right];

/// The door record of the wall between two touching tiles, with the room id of
/// each side. Position and ids come from the two tiles, so they cannot disagree
/// with the geometry: `left_top` is the room that owns the left or top tile.
fn door_record(one: Tile, id_one: i32, two: Tile, id_two: i32) -> Door {
    if one.0 == two.0 {
        // One tile above the other, so the door lies across them.
        let above_first = one.1 < two.1;
        Door {
            x: one.0,
            y: if above_first { two.1 } else { one.1 },
            left_top: if above_first { id_one } else { id_two },
            right_bottom: if above_first { id_two } else { id_one },
            vertical: false,
        }
    } else {
        let left_first = one.0 < two.0;
        Door {
            x: if left_first { two.0 } else { one.0 },
            y: one.1,
            left_top: if left_first { id_one } else { id_two },
            right_bottom: if left_first { id_two } else { id_one },
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
fn wall_doors(rooms: &[Room], rng: &mut Rng) -> Vec<Door> {
    let mut doors = Vec::new();
    for ((left, right), wall) in walls(rooms) {
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
/// were grown.
fn link_rooms(rooms: &[Room], doors: &mut Vec<Door>, rng: &mut Rng) {
    let walls = walls(rooms);
    loop {
        let group = groups(rooms.len(), doors);
        // The first wall that joins two groups, in room id order.
        let Some((pair, wall)) = walls.iter().find(|((a, b), _)| group[*a] != group[*b]) else {
            return;
        };
        // A wall between two groups cannot be full of doors, a door would have
        // joined them already.
        let free: Vec<Pair> = wall
            .iter()
            .copied()
            .filter(|spot| !doors.iter().any(|door| separates(door, *spot)))
            .collect();
        let spot = free[rng.index(free.len())];
        doors.push(door_record(spot.0, pair.0 as i32, spot.1, pair.1 as i32));
    }
}

/// true when the door sits exactly on this pair of tiles.
fn separates(door: &Door, spot: Pair) -> bool {
    let (one, two) = door.tiles();
    (one == spot.0 && two == spot.1) || (one == spot.1 && two == spot.0)
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