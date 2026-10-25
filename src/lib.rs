//! An in memory model of an FTL `ftl.dat` archive.
//!
//! Nothing in this module touches the filesystem, and nothing is copied: an
//! archive *borrows* the bytes it was read from, so the caller keeps that buffer
//! alive next to it. Payloads are a `Cow` - borrowed from those bytes until
//! something replaces them, owned afterwards.
//!
//! ```no_run
//! use ftl_randomizer::FtlArchive;
//!
//! let bytes = std::fs::read("data/ftl.dat").expect("no archive");
//! let mut archive = FtlArchive::from_bytes(&bytes).expect("not an ftl.dat");
//! archive.set_file("data/kestrel.xml", b"<FTL/>".to_vec());
//! std::fs::write("out.dat", archive.to_bytes().expect("archive too large")).expect("write failed");
//! ```
//!
//! The cost of borrowing is that `fn load() -> FtlArchive` cannot be written:
//! the buffer has to outlive the archive, so it stays visible in the caller.
//!
//! [`FtlArchive::to_bytes`] rebuilds the whole container - path pool, index and
//! every offset - it never copies the layout of the input. Read back an
//! unmodified archive and write it again and you get the same bytes, which is
//! what makes the format understood rather than merely tolerated.

pub mod blueprints;
pub mod floor;
pub mod ftl_mod;
pub mod layout;
pub mod loadout;
pub mod pipeline;
pub mod randomizer;
pub mod rooms;
pub mod rng;
pub mod ship_art;
pub mod ship_txt;
#[cfg(feature = "web")]
pub mod web;

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;

/// Magic bytes at offset 0.
pub const MAGIC: &[u8; 4] = b"PKG\n";
/// Header size, and the only value [`FtlArchive::from_bytes`] accepts there.
pub const HEADER_SIZE: usize = 16;
/// Index entry size, and the only accepted value there.
pub const ENTRY_SIZE: usize = 20;
/// Every payload starts on a 64 byte boundary.
pub const ALIGN: usize = 64;

/// The one error type of this module. Filesystem errors never reach it: the
/// caller owns all file access.
#[derive(Debug)]
pub enum Error {
    /// The bytes are not a valid archive, or break a rule this module needs.
    Format(String),
    /// A payload is bigger than the 4 byte size fields can record.
    TooLarge { path: String, len: usize },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Format(message) => write!(f, "{message}"),
            Error::TooLarge { path, len } => write!(
                f,
                "{path} is {len} bytes, the format stores sizes in 4 bytes (max {})",
                u32::MAX
            ),
        }
    }
}

impl std::error::Error for Error {}

/// Result of an archive operation.
pub type Result<T> = std::result::Result<T, Error>;

/// The name hash the game uses to look files up, and the value stored in the
/// `checksum` column of the index: rotate right by 5, xor in the lowercased
/// byte, once per byte of the path.
///
/// It hashes the *path*, not the content, so editing a file never changes it.
pub fn ftl_hash(path: &str) -> u32 {
    let mut hash: u32 = 0;
    for &byte in path.as_bytes() {
        hash = hash.rotate_right(5) ^ u32::from(byte.to_ascii_lowercase());
    }
    hash
}

fn align_up(value: usize, align: usize) -> usize {
    value.div_ceil(align) * align
}

/// Where a payload goes in the archive being written.
struct Plan<'a> {
    checksum: u32,
    path_offset: u32,
    data_offset: u32,
    bytes: &'a [u8],
}

/// An archive, complete, in memory.
#[derive(Debug)]
pub struct FtlArchive<'a> {
    /// Path to payload. A `BTreeMap` keeps paths sorted, and sorted paths are
    /// the order the path pool and the data section are written in.
    files: BTreeMap<String, Cow<'a, [u8]>>,
    /// Bytes after the last payload of the bytes this was read from. Kept for
    /// reporting, dropped by [`FtlArchive::to_bytes`].
    trailing: usize,
}

impl<'a> FtlArchive<'a> {
    /// An empty archive, for building one from nothing.
    pub fn new() -> Self {
        Self {
            files: BTreeMap::new(),
            trailing: 0,
        }
    }

    /// Read an archive from the bytes of a `.dat` file. The archive borrows
    /// `raw`, so `raw` has to outlive it.
    pub fn from_bytes(raw: &'a [u8]) -> Result<Self> {
        if raw.len() < HEADER_SIZE {
            return Err(Error::Format(format!(
                "file is {} bytes, too small for a {} byte header",
                raw.len(),
                HEADER_SIZE
            )));
        }
        if &raw[..4] != MAGIC {
            return Err(Error::Format(format!("bad magic bytes {:?}", &raw[..4])));
        }
        let header_size = u16::from_be_bytes([raw[4], raw[5]]) as usize;
        let entry_size = u16::from_be_bytes([raw[6], raw[7]]) as usize;
        let count = u32::from_be_bytes(raw[8..12].try_into().unwrap()) as usize;
        let pool_size = u32::from_be_bytes(raw[12..16].try_into().unwrap()) as usize;

        if header_size != HEADER_SIZE || entry_size != ENTRY_SIZE {
            return Err(Error::Format(format!(
                "unsupported header size {header_size} / entry size {entry_size}"
            )));
        }

        let index = header_size..header_size + count * ENTRY_SIZE;
        let pool = index.end..index.end + pool_size;
        if raw.len() < pool.end {
            return Err(Error::Format(format!(
                "file is {} bytes, needs {} for {count} index entries and a {pool_size} byte path pool",
                raw.len(),
                pool.end
            )));
        }

        let mut files: BTreeMap<String, Cow<'a, [u8]>> = BTreeMap::new();
        let mut last_payload = pool.end;
        for i in 0..count {
            let at = index.start + i * ENTRY_SIZE;
            let fields: [u8; ENTRY_SIZE] = raw[at..at + ENTRY_SIZE].try_into().unwrap();
            let checksum = u32::from_be_bytes(fields[0..4].try_into().unwrap());
            let path_offset = u32::from_be_bytes(fields[4..8].try_into().unwrap());
            let data_offset = u32::from_be_bytes(fields[8..12].try_into().unwrap());
            let length = u32::from_be_bytes(fields[12..16].try_into().unwrap());
            let filesize = u32::from_be_bytes(fields[16..20].try_into().unwrap());

            let start = pool.start + path_offset as usize;
            if start >= pool.end {
                return Err(Error::Format(format!(
                    "entry {i}: path offset {path_offset} is past the end of the path pool"
                )));
            }
            let end = raw[start..pool.end]
                .iter()
                .position(|&byte| byte == 0)
                .map(|zero| start + zero)
                .ok_or_else(|| {
                    Error::Format(format!(
                        "entry {i}: path at offset {path_offset} is not null terminated"
                    ))
                })?;
            let path = String::from_utf8(raw[start..end].to_vec()).map_err(|err| {
                Error::Format(format!("entry {i}: path is not valid utf8: {err}"))
            })?;

            let data = data_offset as usize..data_offset as usize + length as usize;
            if data.end > raw.len() {
                return Err(Error::Format(format!(
                    "entry {i} ({path}): payload {data_offset}..{} is past the end of the file ({})",
                    data.end,
                    raw.len()
                )));
            }
            if checksum != ftl_hash(&path) {
                return Err(Error::Format(format!(
                    "entry {i} ({path}): stored checksum {checksum:#010x} does not match the path hash {:010x}",
                    ftl_hash(&path)
                )));
            }
            if length != filesize {
                return Err(Error::Format(format!(
                    "entry {i} ({path}): length {length} and filesize {filesize} differ"
                )));
            }
            if files.contains_key(&path) {
                return Err(Error::Format(format!("entry {i}: duplicate path {path}")));
            }

            last_payload = last_payload.max(data.end);
            files.insert(path, Cow::Borrowed(&raw[data]));
        }

        Ok(Self {
            files,
            trailing: raw.len().saturating_sub(last_payload),
        })
    }

    /// Write the archive out. The result is a complete `.dat` file: header,
    /// index, path pool, then the payloads.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let count = self.files.len();

        // Path pool, and the payload table, both in path order.
        let mut pool: Vec<u8> = Vec::new();
        let mut plan: Vec<Plan> = Vec::with_capacity(count);
        for (path, payload) in &self.files {
            let bytes = &payload[..];
            if bytes.len() > u32::MAX as usize {
                return Err(Error::TooLarge {
                    path: path.clone(),
                    len: bytes.len(),
                });
            }
            let path_offset = pool.len() as u32;
            pool.extend_from_slice(path.as_bytes());
            pool.push(0);
            plan.push(Plan {
                checksum: ftl_hash(path),
                path_offset,
                data_offset: 0,
                bytes,
            });
        }
        if pool.len() > u32::MAX as usize {
            return Err(Error::Format(format!(
                "path pool of {} bytes does not fit in 4 bytes",
                pool.len()
            )));
        }

        // Payload offsets: path order, each one on an ALIGN boundary. The last
        // payload is not padded, the archive ends with it.
        let mut cursor = align_up(HEADER_SIZE + count * ENTRY_SIZE + pool.len(), ALIGN);
        for item in &mut plan {
            item.data_offset = cursor as u32;
            cursor = align_up(cursor + item.bytes.len(), ALIGN);
        }

        // Index order: ascending checksum, which is what the game binary
        // searches. Checksums are unique in practice; break ties on the path
        // offset so the output never depends on the sort's stability.
        let mut by_checksum: Vec<usize> = (0..count).collect();
        by_checksum.sort_by_key(|&i| (plan[i].checksum, plan[i].path_offset));

        let mut out = Vec::with_capacity(cursor);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(HEADER_SIZE as u16).to_be_bytes());
        out.extend_from_slice(&(ENTRY_SIZE as u16).to_be_bytes());
        out.extend_from_slice(&(count as u32).to_be_bytes());
        out.extend_from_slice(&(pool.len() as u32).to_be_bytes());

        for &i in &by_checksum {
            let item = &plan[i];
            out.extend_from_slice(&item.checksum.to_be_bytes());
            out.extend_from_slice(&item.path_offset.to_be_bytes());
            out.extend_from_slice(&item.data_offset.to_be_bytes());
            let size = item.bytes.len() as u32;
            out.extend_from_slice(&size.to_be_bytes());
            out.extend_from_slice(&size.to_be_bytes());
        }
        out.extend_from_slice(&pool);

        for item in &plan {
            out.resize(item.data_offset as usize, 0); // zero filled gap
            out.extend_from_slice(item.bytes);
        }
        Ok(out)
    }

    /// Bytes after the last payload of the bytes this archive was read from.
    /// `to_bytes` drops them; they are reported so a surprising archive is
    /// visible.
    pub fn trailing_bytes(&self) -> usize {
        self.trailing
    }

    /// Number of files.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// True when the archive holds no files.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// All paths, sorted.
    pub fn paths(&self) -> impl Iterator<Item = &str> + '_ {
        self.files.keys().map(String::as_str)
    }

    /// Every path and payload, sorted by path.
    pub fn files(&self) -> impl Iterator<Item = (&str, &[u8])> + '_ {
        self.files
            .iter()
            .map(|(path, payload)| (path.as_str(), &payload[..]))
    }

    /// Payload of one file, or `None` if the archive has no such path.
    pub fn get_file(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(|payload| &payload[..])
    }

    /// Replace the payload of `path`, or add it when it is not there yet.
    /// Returns true if the archive already had that path.
    pub fn set_file(&mut self, path: &str, bytes: Vec<u8>) -> bool {
        self.files
            .insert(path.to_string(), Cow::Owned(bytes))
            .is_some()
    }

    /// Drop a file, path and payload. Returns true if it was there.
    pub fn remove_file(&mut self, path: &str) -> bool {
        self.files.remove(path).is_some()
    }

    /// Compare the payloads of two archives, file by file. An empty vector
    /// means both hold the same bytes under the same paths.
    ///
    /// This compares the files, not the container: offsets, padding and index
    /// order are not looked at. Compare the bytes that were read with
    /// [`FtlArchive::to_bytes`] for that.
    pub fn diff(&self, other: &FtlArchive) -> Vec<String> {
        let mut problems = Vec::new();
        for (path, bytes) in self.files() {
            match other.get_file(path) {
                None => problems.push(format!("{path}: missing from the other archive")),
                Some(other_bytes) if other_bytes != bytes => problems.push(format!(
                    "{path}: payload differs ({} bytes vs {} bytes)",
                    bytes.len(),
                    other_bytes.len()
                )),
                Some(_) => {}
            }
        }
        for path in other.paths() {
            if self.get_file(path).is_none() {
                problems.push(format!("{path}: only in the other archive"));
            }
        }
        problems
    }
}

impl Default for FtlArchive<'_> {
    fn default() -> Self {
        Self::new()
    }
}