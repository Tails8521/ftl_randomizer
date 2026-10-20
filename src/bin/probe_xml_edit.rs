//! Probe: how far quick-xml can be trusted to leave a game XML file alone.
//!
//! Two questions:
//! * can the whole file be copied event by event and come back byte for byte?
//! * can the reader hand back the byte range of a tag, so a room number can be
//!   rewritten in place and nothing else moves?
//!
//! Run `cargo run --release --bin probe_xml_edit [file ...]`.

use quick_xml::events::Event;
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;
use std::fs;
use std::io::Cursor;

fn main() {
    let files: Vec<String> = std::env::args().skip(1).collect();
    let files = if files.is_empty() {
        vec![
            "extracted/data/blueprints.xml".to_string(),
            "extracted/data/dlcBlueprints.xml".to_string(),
            "extracted/data/dlcBlueprintsOverwrite.xml".to_string(),
            "extracted/data/kestral.txt".to_string(),
        ]
    } else {
        files
    };
    for file in files {
        let raw = match fs::read(&file) {
            Ok(raw) => raw,
            Err(error) => {
                eprintln!("{file}: {error}");
                continue;
            }
        };
        round_trip(&file, &raw);
        spans(&file, &raw);
    }
}

/// Copy every event into a writer and compare.
fn round_trip(file: &str, raw: &[u8]) {
    let text = String::from_utf8_lossy(raw);
    let raw = text.as_bytes();
    let mut reader = Reader::from_str(&text);
    reader.config_mut().trim_text(false);
    let mut out = Vec::new();
    {
        let mut writer = Writer::new(Cursor::new(&mut out));
        let mut buffer = Vec::new();
        loop {
            match reader.read_event_into(&mut buffer) {
                Ok(Event::Eof) => break,
                Ok(event) => match writer.write_event(event) {
                    Ok(()) => {}
                    Err(error) => {
                        println!("{file}: round trip stopped: {error}");
                        return;
                    }
                },
                Err(error) => {
                    println!("{file}: round trip stopped: {error}");
                    return;
                }
            }
            buffer.clear();
        }
    }
    if out == raw {
        println!("{file}: round trip is byte for byte equal ({} bytes)", raw.len());
        return;
    }
    let lines = |bytes: &[u8]| bytes.split(|&b| b == b'\n').count();
    println!(
        "{file}: round trip differs, {} bytes out vs {} in, {} lines vs {}",
        out.len(),
        raw.len(),
        lines(&out),
        lines(raw)
    );
    let mut shown = 0;
    for (i, (a, b)) in raw.iter().zip(out.iter()).enumerate() {
        if a != b {
            let (from, to) = (i.saturating_sub(40), (i + 40).min(raw.len()).min(out.len()));
            println!("  at {i}: in  {:?}", String::from_utf8_lossy(&raw[from..to]));
            println!("  at {i}: out {:?}", String::from_utf8_lossy(&out[from..to]));
            shown += 1;
            if shown == 4 {
                break;
            }
        }
    }
}

/// Walk the tags of a file and check that the byte range worked out from
/// `buffer_position` and the raw tag bytes really is the tag.
fn spans(file: &str, raw: &[u8]) {
    let text = String::from_utf8_lossy(raw);
    let raw = text.as_bytes();
    let mut reader = Reader::from_str(&text);
    let mut buffer = Vec::new();
    let mut checked = 0;
    let mut wrong = 0;
    let mut sample = String::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Eof) => break,
            Ok(Event::Start(tag)) => {
                if !check(raw, reader.buffer_position() as usize, &tag, false, &mut sample) {
                    wrong += 1;
                }
                checked += 1;
            }
            Ok(Event::Empty(tag)) => {
                if !check(raw, reader.buffer_position() as usize, &tag, true, &mut sample) {
                    wrong += 1;
                }
                checked += 1;
            }
            Ok(_) => {}
            Err(error) => {
                println!("{file}: span walk stopped: {error}");
                return;
            }
        }
        buffer.clear();
    }
    println!("{file}: {checked} tags, {wrong} with a wrong byte range");
    if !sample.is_empty() {
        println!("  sample: {sample}");
    }
}

/// true when `<raw[position-len..position]>` is the tag as written.
fn check(raw: &[u8], end: usize, tag: &quick_xml::events::BytesStart<'_>, empty: bool, sample: &mut String) -> bool {
    let inner = tag.as_ref();
    let tail: &[u8] = if empty { b"/>" } else { b">" };
    let need = inner.len() + 1 + tail.len();
    if end < need {
        return false;
    }
    let start = end - need;
    let mut want = Vec::with_capacity(need);
    want.push(b'<');
    want.extend_from_slice(inner);
    want.extend_from_slice(tail);
    if sample.is_empty() {
        *sample = format!("{:?} vs {:?}", String::from_utf8_lossy(&raw[start..end]), String::from_utf8_lossy(&want));
    }
    raw[start..end] == want
}