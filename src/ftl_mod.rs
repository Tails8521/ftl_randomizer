//! The `.ftl` mod: a zip of the files to drop onto the game.
//!
//! A mod is a zip archive whose root holds the folders of `ftl.dat` - `data/`,
//! `img/`, ... - and only the files it changes, so it stays small and does not
//! clobber more of the game than it has to. `mod-appendix/metadata.xml` is the
//! description a mod manager shows, and is optional.
//!
//! Entries are stored rather than deflated. A randomized layout is 28 layout files
//! of about a kilobyte each and the three blueprints files, so a compressor would
//! save little and have to be carried into the library to do it. A stored entry is
//! the one thing every zip reader has to handle.
//!
//! ```
//! use ftl_randomizer::ftl_mod::FtlMod;
//!
//! let mut mod_file = FtlMod::new();
//! mod_file.add("data/kestral.txt", b"X_OFFSET\r\n0\r\n");
//! let bytes = mod_file.to_bytes();
//! assert_eq!(&bytes[..4], b"PK\x03\x04");
//! ```

use std::collections::BTreeMap;

use std::sync::OnceLock;

/// Zip: a file follows.
const LOCAL: u32 = 0x0403_4b50;
/// Zip: the index entry of a file.
const CENTRAL: u32 = 0x0201_4b50;
/// Zip: the end of the index.
const END: u32 = 0x0605_4b50;
/// The zip reader version that reads what this writes, and wrote it.
const VERSION: u16 = 20;
/// 1980-01-01 00:00:00, the DOS stamp of a file with no date. A fixed stamp keeps
/// a mod built twice byte for byte the same.
const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = 0x0021;

/// A mod, complete, in memory.
#[derive(Debug, Default)]
pub struct FtlMod {
    /// Archive path to payload. Sorted paths mean a rebuilt mod is the same bytes.
    files: BTreeMap<String, Vec<u8>>,
}

impl FtlMod {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add one file of the game, by its path inside `ftl.dat`.
    pub fn add(&mut self, path: &str, bytes: &[u8]) {
        self.files.insert(path.to_string(), bytes.to_vec());
    }

    /// The description a mod manager reads. `mod-appendix/` is read by the manager
    /// and never handed to the game.
    pub fn describe(&mut self, name: &str, version: &str, author: &str, description: &str) {
        let text = format!(
            "<mod>\r\n\t<name>{}</name>\r\n\t<version>{}</version>\r\n\t<author>{}</author>\r\n\
             \t<description>{}</description>\r\n</mod>\r\n",
            escaped(name),
            escaped(version),
            escaped(author),
            escaped(description)
        );
        self.add("mod-appendix/metadata.xml", text.as_bytes());
    }

    /// Number of files.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// True when the mod holds nothing.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Every path the mod carries, sorted.
    pub fn paths(&self) -> impl Iterator<Item = &str> + '_ {
        self.files.keys().map(String::as_str)
    }

    /// Write the archive: a header and a payload per file, then the index that
    /// points back at them.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut index = Vec::new();
        let mut offset = 0usize;

        for (path, bytes) in &self.files {
            let name = path.as_bytes();
            let crc = crc32(bytes);

            let mut head = Vec::new();
            number32(&mut head, LOCAL);
            number16(&mut head, VERSION); // reader the file needs
            number16(&mut head, 0); // no flags
            number16(&mut head, 0); // stored
            number16(&mut head, DOS_TIME);
            number16(&mut head, DOS_DATE);
            number32(&mut head, crc);
            number32(&mut head, bytes.len() as u32); // compressed size
            number32(&mut head, bytes.len() as u32); // size on the other side
            number16(&mut head, name.len() as u16);
            number16(&mut head, 0); // no extra field
            let written = head.len() + name.len() + bytes.len();

            out.extend_from_slice(&head);
            out.extend_from_slice(name);
            out.extend_from_slice(bytes);

            number32(&mut index, CENTRAL);
            number16(&mut index, VERSION); // written by
            number16(&mut index, VERSION); // reader the file needs
            number16(&mut index, 0); // no flags
            number16(&mut index, 0); // stored
            number16(&mut index, DOS_TIME);
            number16(&mut index, DOS_DATE);
            number32(&mut index, crc);
            number32(&mut index, bytes.len() as u32);
            number32(&mut index, bytes.len() as u32);
            number16(&mut index, name.len() as u16);
            number16(&mut index, 0); // no extra field
            number16(&mut index, 0); // no comment
            number16(&mut index, 0); // one disk
            number16(&mut index, 0); // no file attributes
            number32(&mut index, 0); // no file attributes either
            number32(&mut index, offset as u32); // where the header sits
            index.extend_from_slice(name);

            offset += written;
        }

        let directory = out.len();
        out.extend_from_slice(&index);
        number32(&mut out, END);
        number16(&mut out, 0); // this disk
        number16(&mut out, 0); // the disk the index starts on
        number16(&mut out, self.files.len() as u16); // entries here
        number16(&mut out, self.files.len() as u16); // entries in total
        number32(&mut out, index.len() as u32);
        number32(&mut out, directory as u32);
        number16(&mut out, 0); // no comment
        out
    }
}

/// CRC-32, the checksum a zip entry carries its payload with.
fn crc32(bytes: &[u8]) -> u32 {
    static TABLE: OnceLock<[u32; 256]> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut table = [0u32; 256];
        for (index, entry) in table.iter_mut().enumerate() {
            let mut value = index as u32;
            for _ in 0..8 {
                value = match value & 1 {
                    1 => 0xedb8_8320 ^ (value >> 1),
                    _ => value >> 1,
                };
            }
            *entry = value;
        }
        table
    });
    let mut crc = !0u32;
    for &byte in bytes {
        crc = table[((crc ^ u32::from(byte)) & 0xff) as usize] ^ (crc >> 8);
    }
    !crc
}

/// XML text, so a description cannot write tags.
fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

fn number16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn number32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}