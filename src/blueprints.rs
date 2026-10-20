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
//! * the crew stand at six systems only - pilot, weapons, shields, engines, doors
//!   and sensors - so only those need a `<slot>` naming a tile their room really
//!   has, and one is written when the blueprint has none;
//! * the medbay and the clone bay are one room with two machines, so they get the
//!   same room and every other system gets a room to itself. Artillery takes a
//!   room like any other system, but no station: the gun fires on its own.

use std::collections::BTreeMap;
use std::ops::Range;

use quick_xml::events::Event;
use quick_xml::reader::Reader;

use crate::layout::Room;
use crate::rng::Rng;

/// A byte range of the file, and the text that replaces it.
type Patch = (Range<usize>, String);

/// The systems with a station, and the direction the crew face at it when the
/// blueprint does not say.
const MANNED: [(&str, &str); 6] = [
    ("pilot", "right"),
    ("weapons", "up"),
    ("shields", "left"),
    ("engines", "down"),
    ("doors", "right"),
    ("sensors", "down"),
];

/// One `<shipBlueprint>` of a blueprints file.
#[derive(Debug)]
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

/// The room a system shares with another. The clone bay is a medbay with a
/// different machine, in the same room.
fn share(name: &str) -> &str {
    if name == "clonebay" { "medbay" } else { name }
}

/// The direction the station of a manned system faces, `None` when the system has
/// no station at all.
fn station_direction(name: &str) -> Option<&'static str> {
    MANNED
        .iter()
        .find(|(system, _)| *system == name)
        .map(|(_, direction)| *direction)
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
fn system_tags(text: &str, base: usize) -> Result<Vec<Element>, String> {
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

/// Move the systems of every blueprint whose layout is in `layouts` into a room of
/// its own. `Ok(None)` means the file holds none of them.
pub fn randomize(
    text: &str,
    layouts: &BTreeMap<String, Vec<Room>>,
    rng: &mut Rng,
) -> Result<Option<Edit>, String> {
    let mut patches: Vec<Patch> = Vec::new();
    let mut changes = Vec::new();

    for blueprint in blueprints(text)? {
        let Some(rooms) = layouts.get(&blueprint.layout) else {
            continue;
        };
        let mut change = Change {
            blueprint: blueprint.name.clone(),
            layout: blueprint.layout.clone(),
            systems: Vec::new(),
            warnings: Vec::new(),
        };
        assign(text, &blueprint, rooms, rng, &mut patches, &mut change)?;
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
    rooms: &[Room],
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
    let mut groups: Vec<(&str, Vec<&Element>)> = Vec::new();
    for system in systems.iter() {
        let key = share(&system.name);
        match groups.iter_mut().find(|(name, _)| *name == key) {
            Some((_, members)) => members.push(system),
            None => groups.push((key, vec![system])),
        }
    }

    let mut free: Vec<usize> = (0..rooms.len()).collect();
    rng.shuffle(&mut free);
    if groups.len() > free.len() {
        change.warnings.push(format!(
            "{} needs {} rooms, the layout of {} has {}, so systems share rooms",
            blueprint.name,
            groups.len(),
            blueprint.layout,
            rooms.len()
        ));
    }

    for (index, (_, members)) in groups.iter().enumerate() {
        let room = free[index % free.len()];
        let tiles = rooms[room].w * rooms[room].h;
        for system in members {
            let raw = &text[system.tag.clone()];
            let (value, _) = attribute(raw, "room").expect("systems were picked for their room");
            patches.push((
                system.tag.start + value.start..system.tag.start + value.end,
                room.to_string(),
            ));
            change.systems.push((system.name.clone(), room));
            station(text, system, tiles, rng, &mut change.warnings, patches);
        }
    }
    Ok(())
}

/// The station of a manned system, and the numbers of the systems that have one
/// without being manned.
///
/// A number names a tile of the room, so a number the new room does not have would
/// put the station out of reach. A manned system always gets a fresh number, and a
/// whole `<slot>` block when the blueprint never had one. The others keep the
/// numbers they shipped with unless the room is too small to hold them.
fn station(
    text: &str,
    system: &Element,
    tiles: i32,
    rng: &mut Rng,
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
    let default = match station_direction(&system.name) {
        Some(direction) => direction,
        None => {
            for element in &inside {
                if element.name != "number" {
                    continue;
                }
                let value = text[number_range(element)].trim();
                if let Ok(number) = value.parse::<i32>()
                    && number >= 0
                    && number >= tiles
                {
                    patches.push((number_range(element), rng.below(tiles as u64).to_string()));
                }
            }
            return;
        }
    };

    // Everything between the open tag and the closing tag is replaced by one slot
    // block. Whatever is in there has to be slots, or the system holds something
    // this module does not know and is left alone.
    let block = slot_block(text, system, slots.first().copied(), default, tiles, rng);
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
    patches.push((from..to, block));
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

/// The `<slot>` block of a station: the direction the old slot had, or the one the
/// system faces by default, and a tile of the room the system was put in.
fn slot_block(
    text: &str,
    system: &Element,
    slot: Option<&Element>,
    default: &str,
    tiles: i32,
    rng: &mut Rng,
) -> String {
    let direction = slot
        .and_then(|slot| inside(text, slot, "direction"))
        .unwrap_or_else(|| default.to_string());
    let number = rng.below(tiles as u64);
    let newline = match text.contains("\r\n") {
        true => "\r\n",
        false => "\n",
    };
    let edge = indent_before(text, system.span.start);
    let one = format!("{edge}\t");
    let two = format!("{one}\t");
    format!(
        "{newline}{one}<slot>{newline}{two}<direction>{direction}</direction>\
         {newline}{two}<number>{number}</number>{newline}{one}</slot>{newline}{edge}"
    )
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
struct Element {
    name: String,
    /// The `<name ...>` bytes, `/` included when the element is self closing.
    tag: Range<usize>,
    /// The whole element, children included.
    span: Range<usize>,
    /// How deep it sits, the first element of the walk being at depth 0.
    depth: usize,
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
fn attribute<'a>(tag: &'a str, key: &str) -> Option<(Range<usize>, &'a str)> {
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