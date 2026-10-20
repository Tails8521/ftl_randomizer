//! The `<layout>.txt` file: the room grid, the doors and the hull shape of a
//! ship.
//!
//! The format is a flat list of records, one token per line: a key then a fixed
//! number of numbers. The keys before the first `ROOM` describe the hull and are
//! carried over untouched, `ROOM` and `DOOR` describe the layout and are what the
//! randomizer replaces. Line endings are CRLF: the game reads XML with either and
//! crashes on a bare `LF` in a layout file.
//!
//! ```
//! use ftl_randomizer::ship_txt::ShipTxt;
//!
//! let txt = ShipTxt::parse(b"X_OFFSET\r\n0\r\nROOM\r\n0\r\n4\r\n2\r\n2\r\n2\r\n".to_vec()).unwrap();
//! assert_eq!(txt.rooms.len(), 1);
//! ```

use crate::layout::{Door, Room};

/// How many numbers follow each key.
const KEYS: [(&str, usize); 7] = [
    ("X_OFFSET", 1),
    ("Y_OFFSET", 1),
    ("HORIZONTAL", 1),
    ("VERTICAL", 1),
    ("ELLIPSE", 4),
    ("ROOM", 5),
    ("DOOR", 5),
];

/// A parsed layout file.
#[derive(Debug)]
pub struct ShipTxt {
    /// Everything before the first `ROOM` or `DOOR`, as it was read.
    head: Vec<u8>,
    /// The hull keys of the head, `ELLIPSE` carrying four numbers.
    pub keys: Vec<(String, Vec<i32>)>,
    /// The rooms, in file order. A room id is its place in this list.
    pub rooms: Vec<Room>,
    /// Every door of the ship, airlocks included.
    pub doors: Vec<Door>,
}

impl ShipTxt {
    /// Read a layout file. A key this module does not know, or a record missing
    /// its numbers, is an error: a layout the randomizer cannot read fully is a
    /// layout it must not rewrite.
    pub fn parse(src: Vec<u8>) -> Result<Self, String> {
        let tokens = tokens(&src);
        let mut keys = Vec::new();
        let mut rooms = Vec::new();
        let mut doors = Vec::new();
        let mut head: Option<usize> = None;

        let mut at = 0;
        while at < tokens.len() {
            let name = String::from_utf8_lossy(&src[tokens[at].0..tokens[at].1]).into_owned();
            let count = KEYS
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, count)| *count)
                .ok_or_else(|| format!("unknown key {name:?} at byte {}", tokens[at].0))?;
            if head.is_none() && (name == "ROOM" || name == "DOOR") {
                head = Some(tokens[at].0);
            }
            if at + count >= tokens.len() {
                return Err(format!("{name} at token {at} is missing numbers"));
            }
            let mut numbers = Vec::with_capacity(count);
            for token in &tokens[at + 1..at + 1 + count] {
                let text = String::from_utf8_lossy(&src[token.0..token.1]).into_owned();
                numbers.push(text.parse::<i32>().map_err(|_| {
                    format!("{name} at token {at} has {text:?} where a number belongs")
                })?);
            }
            at += count + 1;

            match name.as_str() {
                "ROOM" => rooms.push(Room {
                    x: numbers[1],
                    y: numbers[2],
                    w: numbers[3],
                    h: numbers[4],
                }),
                "DOOR" => doors.push(Door {
                    x: numbers[0],
                    y: numbers[1],
                    left_top: numbers[2],
                    right_bottom: numbers[3],
                    vertical: numbers[4] == 1,
                }),
                _ => keys.push((name, numbers)),
            }
        }
        if rooms.is_empty() {
            return Err("the layout has no rooms".to_string());
        }
        let mut txt = ShipTxt {
            head: src[..head.unwrap_or(src.len())].to_vec(),
            keys,
            rooms,
            doors,
        };
        txt.normalize_head();
        Ok(txt)
    }

    /// The single number of a head key, `HORIZONTAL` and `VERTICAL` for example.
    pub fn number(&self, key: &str) -> Option<i32> {
        self.keys
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, numbers)| numbers[0])
    }

    /// The box the rooms of the file cover, `(low x, low y, high x, high y)`,
    /// corners included, in tiles. `None` when the file holds no room.
    ///
    /// Every layout the game ships keeps its rooms at tile 0 or above and inside
    /// its own box, so a layout that leaves it steps outside anything the game is
    /// known to build.
    pub fn bounds(&self) -> Option<(i32, i32, i32, i32)> {
        let low = |pick: fn(&Room) -> i32| self.rooms.iter().map(pick).min();
        let high = |pick: fn(&Room) -> i32| self.rooms.iter().map(pick).max();
        Some((
            low(|r| r.x)?,
            low(|r| r.y)?,
            high(|r| r.x + r.w - 1)?,
            high(|r| r.y + r.h - 1)?,
        ))
    }

    /// Put a new layout in place of the rooms and doors that were read. The head
    /// stays, because it describes the hull art, not the rooms.
    pub fn set_layout(&mut self, rooms: Vec<Room>, doors: Vec<Door>) {
        self.rooms = rooms;
        self.doors = doors;
    }

    /// Write the file back out, with CRLF endings and one token per line.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = self.head.clone();
        for (index, room) in self.rooms.iter().enumerate() {
            record(
                &mut out,
                "ROOM",
                &[
                    index as i32,
                    room.x,
                    room.y,
                    room.w,
                    room.h,
                ],
            );
        }
        for door in &self.doors {
            record(
                &mut out,
                "DOOR",
                &[
                    door.x,
                    door.y,
                    door.left_top,
                    door.right_bottom,
                    i32::from(door.vertical),
                ],
            );
        }
        out
    }

    /// Rewrite the head with CRLF endings, so a file read with bare `LF` still
    /// comes back the way the layout reader wants it.
    fn normalize_head(&mut self) {
        let text = String::from_utf8_lossy(&self.head).into_owned();
        let mut out = Vec::new();
        for line in text.lines() {
            out.extend_from_slice(line.trim_end().as_bytes());
            out.extend_from_slice(b"\r\n");
        }
        self.head = out;
    }
}

/// One record: the key then its numbers, each on its own line.
fn record(out: &mut Vec<u8>, key: &str, numbers: &[i32]) {
    out.extend_from_slice(key.as_bytes());
    out.extend_from_slice(b"\r\n");
    for number in numbers {
        out.extend_from_slice(number.to_string().as_bytes());
        out.extend_from_slice(b"\r\n");
    }
}

/// Byte ranges of the whitespace separated tokens of `src`.
fn tokens(src: &[u8]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < src.len() {
        while at < src.len() && src[at].is_ascii_whitespace() {
            at += 1;
        }
        let start = at;
        while at < src.len() && !src[at].is_ascii_whitespace() {
            at += 1;
        }
        if start < at {
            out.push((start, at));
        }
    }
    out
}