//! The deck art of the player ships: the grey that shows through the rooms.
//!
//! The game draws `img/ship/<layout>_floor.png` behind the rooms, at
//! `<offsets><floor x y>` inside the hull art. That offset and `<img x y>` are
//! both relative to the hull, so the deck stands at a fixed place in the room
//! grid: the top left corner of the canvas sits on grid pixel
//! `floor + img`, and a room of the grid lands on it 35 pixels to a tile.
//!
//! What the deck looks like was measured over the 20 floors the game ships, and
//! `src/bin/probe_floor.rs` scores the rule against every one of them: seven of
//! the twenty are redrawn to within two hundredths of their pixels. Every room
//! box grown by `PAD`, with each corner of a box cut by `CHAMFER`, and the strip
//! behind every airlock door left unpainted, is the floor of all of them. It is
//! painted in the two colours of the art the ship ships: a band `OUTLINE` pixels
//! deep along the edge of the shape, and the deck colour within it. Zoltan B - the
//! one layout that is regular all over - is off by 126 pixels of 58 490.
//! The two ships farthest off are hand finished in ways no rule of rooms can
//! tell: Rock A leaves three of the corridors between its rooms unpainted, and
//! Mantis A leaves an octagonal hole in the deck of one of them, both of them
//! leaving out deck the rule paints.
//!
//! The canvas is never moved or resized: the shipped art gives the size, and
//! `<offsets>` keeps its numbers, so the game places the new deck exactly as it
//! placed the old one. A layout whose ship ships no deck art of its own - the
//! eight `*_3` hulls, whose floors the game has to borrow from another ship -
//! gets a canvas as big as its hull, parked where its own `<offsets>` puts it.

use std::collections::BTreeMap;

use image::{ImageReader, Rgba, RgbaImage};

use crate::layout::{Door, Room, Side, TILE};
use crate::ship_art::encode;

/// How far the deck grows past a room box on a wall with no airlock in it.
const PAD: i32 = 6;

/// How far inside the room the deck stops at an airlock door. The door and its
/// frame are drawn on the room side of the wall, over the tile the door stands
/// in, and the shipped decks leave that strip unpainted. Measured on Zoltan B,
/// where it is four pixels on every one of its five airlocks.
const AIRLOCK_IN: i32 = 4;

/// How much of each corner of a room box is cut away. Measured on Zoltan B: the
/// top row of a box starts three pixels in, the next two rows one pixel less.
const CHAMFER: i32 = 3;

/// The grey of a deck with no shipped art to take the colour from. It is what
/// 19 of the 20 shipped floors are painted with, `stealth_2` being the one that
/// is a shade bluer.
const GREY: [u8; 3] = [100, 105, 111];

/// The black of the band along the edge of a deck, for a ship that ships no deck
/// art to take it from. 19 of the 20 shipped floors draw that band in it.
const BLACK: [u8; 3] = [0, 0, 0];

/// How deep that band is, measured over the 20 shipped floors: the outer two rows
/// of the shape are the dark colour and nothing dark lies deeper. Over the 1 281
/// 205 pixels they paint, the rule of a two pixel band and the art disagree about
/// 6 300, which are the odd pixel at the corners of a cut, where the band was
/// painted by hand along the curve of the corner.
const OUTLINE: i32 = 2;

/// Where the deck art goes and what it is painted with.
#[derive(Clone, Copy, Debug)]
pub struct Deck {
    /// Canvas size in pixels.
    pub width: u32,
    pub height: u32,
    /// The grid pixel the top left corner of the canvas stands on.
    pub x: i32,
    pub y: i32,
    pub colour: [u8; 3],
    /// The colour of the band along the edge of the shape.
    pub edge: [u8; 3],
}

impl Deck {
    /// The canvas of the deck art the game ships for a layout: its size, its two
    /// colours and `at`, the grid pixel its corner stands on.
    pub fn shipped(art: &[u8], at: (i32, i32)) -> Result<Self, String> {
        let image = ImageReader::new(std::io::Cursor::new(art))
            .with_guessed_format()
            .map_err(|error| error.to_string())?
            .decode()
            .map_err(|error| error.to_string())?
            .to_rgba8();
        let mut counts: BTreeMap<[u8; 3], u32> = BTreeMap::new();
        for pixel in image.pixels() {
            if pixel[3] == 255 {
                *counts.entry([pixel[0], pixel[1], pixel[2]]).or_default() += 1;
            }
        }
        // The deck colour is the one the art is mostly painted with, and the edge
        // colour the band along its shape: black for 19 of the 20 floors, and the
        // near black `stealth_2` paints itself with.
        let mut order: Vec<([u8; 3], u32)> = counts.into_iter().collect();
        order.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        let colour = order.first().map(|(rgb, _)| *rgb).unwrap_or(GREY);
        let edge = order.get(1).map(|(rgb, _)| *rgb).unwrap_or(BLACK);
        let (width, height) = image.dimensions();
        Ok(Deck {
            width,
            height,
            x: at.0,
            y: at.1,
            colour,
            edge,
        })
    }

    /// A canvas for a ship that ships no deck art: as big as its hull, parked
    /// where its own `<offsets>` puts it, which is enough room for any layout of
    /// that hull.
    pub fn hull(hull: (u32, u32), at: (i32, i32)) -> Self {
        Deck {
            width: hull.0,
            height: hull.1,
            x: at.0,
            y: at.1,
            colour: GREY,
            edge: BLACK,
        }
    }
}

/// The deck art that was drawn.
#[derive(Debug)]
pub struct Art {
    /// The PNG bytes.
    pub png: Vec<u8>,
    /// How many painted pixels fell off the canvas. Zero, since a layout never
    /// leaves the box its ship shipped in and the shipped canvas covers that box
    /// grown by `PAD` on every side.
    pub clipped: u64,
    /// How many pixels of deck were painted.
    pub pixels: u64,
}

/// The deck of one layout: every room of `rooms` grown by `PAD` and unioned, with
/// the strip behind every airlock door of `doors` taken back out again, painted
/// with the dark band of `OUTLINE` along its edge.
pub fn draw(rooms: &[Room], doors: &[Door], deck: &Deck) -> Result<Art, String> {
    let strips = airlock_strips(rooms, doors);
    let (wide, high) = (deck.width as usize, deck.height as usize);
    let mut mask = vec![false; wide * high];
    let mut clipped = 0;

    for (index, room) in rooms.iter().enumerate() {
        // The grown box in grid pixels, far edges exclusive, so it is
        // `w * TILE + 2 * PAD` across.
        let box_ = (
            room.x * TILE - PAD,
            room.y * TILE - PAD,
            (room.x + room.w) * TILE + PAD,
            (room.y + room.h) * TILE + PAD,
        );
        let holes = strips.get(&index).cloned().unwrap_or_default();
        for gy in box_.1..box_.3 {
            for gx in box_.0..box_.2 {
                if cut(gx - box_.0, gy - box_.1, box_.2 - box_.0, box_.3 - box_.1) {
                    continue;
                }
                if holes
                    .iter()
                    .any(|hole| hole.0 <= gx && gx < hole.2 && hole.1 <= gy && gy < hole.3)
                {
                    continue;
                }
                let (px, py) = (gx - deck.x, gy - deck.y);
                if px < 0 || py < 0 || px >= deck.width as i32 || py >= deck.height as i32 {
                    clipped += 1;
                    continue;
                }
                mask[py as usize * wide + px as usize] = true;
            }
        }
    }

    // The shape is painted with the two colours in a second pass, because the band
    // needs to know where the shape ends.
    let mut image = RgbaImage::from_pixel(deck.width, deck.height, Rgba([0, 0, 0, 0]));
    let deck_paint = Rgba([deck.colour[0], deck.colour[1], deck.colour[2], 255]);
    let edge_paint = Rgba([deck.edge[0], deck.edge[1], deck.edge[2], 255]);
    let mut pixels = 0;
    for y in 0..high {
        for x in 0..wide {
            if !mask[y * wide + x] {
                continue;
            }
            pixels += 1;
            let paint = if deep(&mask, wide, high, x, y) {
                deck_paint
            } else {
                edge_paint
            };
            image.put_pixel(x as u32, y as u32, paint);
        }
    }

    Ok(Art {
        png: encode(&image)?,
        clipped,
        pixels,
    })
}

/// true when the deck reaches `OUTLINE` pixels beyond `x,y` on every side, so the
/// pixel lies within the dark band that runs along the edge of the shape. The side
/// of the canvas counts as an edge of it.
fn deep(mask: &[bool], wide: usize, high: usize, x: usize, y: usize) -> bool {
    let (x, y) = (x as i32, y as i32);
    (-OUTLINE..=OUTLINE).all(|dy| {
        (-OUTLINE..=OUTLINE).all(|dx| {
            let (nx, ny) = (x + dx, y + dy);
            nx >= 0
                && ny >= 0
                && (nx as usize) < wide
                && (ny as usize) < high
                && mask[ny as usize * wide + nx as usize]
        })
    })
}

/// true when the pixel of a box falls in one of the four corner cuts.
fn cut(dx: i32, dy: i32, width: i32, height: i32) -> bool {
    dx + dy < CHAMFER
        || (width - 1 - dx) + dy < CHAMFER
        || dx + (height - 1 - dy) < CHAMFER
        || (width - 1 - dx) + (height - 1 - dy) < CHAMFER
}

/// The strips to leave unpainted behind the airlock doors of each room, as grid
/// pixel rectangles, one per door.
///
/// A strip covers the tile the door stands in, from `PAD` outside the wall to
/// `AIRLOCK_IN` inside it. It stops at the tile, because the rest of the wall
/// keeps its deck: 16 of the walls of the shipped layouts hold an airlock on one
/// tile and nothing on the other, and the deck over the plain tile is grown like
/// any other. Zoltan B gives no hint of it, since each of its three airlock walls
/// is airlocked over its whole length. Anaerobic B, Kestrel B, Fed B and Mantis B
/// do have walls like that, and taking the strip per door rather than per wall
/// lifts the first two of them from 0.9318 and 0.9359 to 0.9654 and 0.9562.
///
/// An airlock names its room first and `-1` second whichever side of the door
/// the room is on, so the wall comes from the two tiles the door divides: the
/// room owns one of them, and the wall that tile touches is the one the door
/// lies on.
fn airlock_strips(rooms: &[Room], doors: &[Door]) -> BTreeMap<usize, Vec<(i32, i32, i32, i32)>> {
    let mut out: BTreeMap<usize, Vec<(i32, i32, i32, i32)>> = BTreeMap::new();
    for door in doors.iter().filter(|door| door.airlock()) {
        let id = if door.left_top == -1 {
            door.right_bottom
        } else {
            door.left_top
        };
        // A room id is its place in the room list.
        let Ok(index) = usize::try_from(id) else {
            continue;
        };
        let Some(&room) = rooms.get(index) else {
            continue;
        };
        let (low, high) = door.tiles();
        let (side, tile) = if holds(room, high) {
            (if door.vertical { Side::Left } else { Side::Up }, high)
        } else if holds(room, low) {
            (
                if door.vertical { Side::Right } else { Side::Down },
                low,
            )
        } else {
            continue;
        };
        // The walls of the room in grid pixels, and the span of the tile along
        // the wall.
        let (wall, span) = match side {
            Side::Left | Side::Right => {
                let line = match side {
                    Side::Left => room.x * TILE,
                    _ => (room.x + room.w) * TILE,
                };
                (line, (tile.1 * TILE, (tile.1 + 1) * TILE))
            }
            Side::Up | Side::Down => {
                let line = match side {
                    Side::Up => room.y * TILE,
                    _ => (room.y + room.h) * TILE,
                };
                (line, (tile.0 * TILE, (tile.0 + 1) * TILE))
            }
        };
        // Outside the wall by `PAD`, inside it by `AIRLOCK_IN`.
        let (from, to) = match side {
            Side::Left | Side::Up => (wall - PAD, wall + AIRLOCK_IN),
            Side::Right | Side::Down => (wall - AIRLOCK_IN, wall + PAD),
        };
        let strip = match side {
            Side::Left | Side::Right => (from, span.0, to, span.1),
            Side::Up | Side::Down => (span.0, from, span.1, to),
        };
        out.entry(index).or_default().push(strip);
    }
    out
}

/// true when the tile is one the room covers.
fn holds(room: Room, tile: (i32, i32)) -> bool {
    tile.0 >= room.x
        && tile.0 < room.x + room.w
        && tile.1 >= room.y
        && tile.1 < room.y + room.h
}