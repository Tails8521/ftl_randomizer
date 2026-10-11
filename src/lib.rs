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

pub mod randomizer;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// The stock archive bytes. Gitignored, so tests that need it say so
    /// plainly. The caller has to hold on to them: an archive borrows them.
    const STOCK: &str = "data/ftl.dat";

    fn stock_bytes() -> Vec<u8> {
        fs::read(STOCK).expect("no data/ftl.dat to test with")
    }

    fn tiny_archive() -> FtlArchive<'static> {
        let mut archive = FtlArchive::new();
        archive.set_file("data/zeta.xml", b"z".to_vec());
        archive.set_file("ships/kestrel.xml", b"k".to_vec());
        archive.set_file("data/alpha.xml", b"a".to_vec());
        archive
    }

    /// The path stored at `offset` in a path pool.
    fn path_at(pool: &[u8], offset: usize) -> &str {
        let end = offset + pool[offset..].iter().position(|&byte| byte == 0).unwrap();
        std::str::from_utf8(&pool[offset..end]).unwrap()
    }

    #[test]
    fn empty_archive_is_a_header() {
        let bytes = FtlArchive::new().to_bytes().unwrap();
        assert_eq!(bytes.len(), HEADER_SIZE);
        assert_eq!(FtlArchive::from_bytes(&bytes).unwrap().len(), 0);
    }

    #[test]
    fn written_archive_follows_the_layout_rules() {
        let bytes = tiny_archive().to_bytes().unwrap();
        let count = 3;
        let pool_start = HEADER_SIZE + count * ENTRY_SIZE;
        let pool_len = u32::from_be_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let pool = &bytes[pool_start..pool_start + pool_len];

        // path pool: sorted paths, null terminated, no padding
        assert_eq!(
            pool, b"data/alpha.xml\0data/zeta.xml\0ships/kestrel.xml\0",
            "path pool must be sorted and unpadded"
        );

        // index: ascending checksum, one entry per file, sizes recorded twice
        #[derive(Debug)]
        struct Row {
            checksum: u32,
            path: String,
            offset: usize,
            length: u32,
        }
        let mut rows = Vec::new();
        for i in 0..count {
            let at = HEADER_SIZE + i * ENTRY_SIZE;
            let field = |column: usize| {
                u32::from_be_bytes(
                    bytes[at + column * 4..at + column * 4 + 4]
                        .try_into()
                        .unwrap(),
                )
            };
            rows.push(Row {
                checksum: field(0),
                path: path_at(pool, field(1) as usize).to_string(),
                offset: field(2) as usize,
                length: field(3),
            });
        }
        assert!(
            rows.windows(2)
                .all(|pair| pair[0].checksum < pair[1].checksum),
            "index must be sorted by ascending checksum: {rows:?}"
        );
        for row in &rows {
            assert_eq!(row.checksum, ftl_hash(&row.path));
            assert_eq!(row.length, 1, "every tiny payload is one byte");
            assert_eq!(row.offset % ALIGN, 0, "payloads must be aligned");
        }

        // payloads sit in path order, each on its own alignment, and the file
        // ends with the last one
        let data_start = align_up(pool_start + pool_len, ALIGN);
        rows.sort_by_key(|row| row.path.clone());
        let offsets: Vec<usize> = rows.iter().map(|row| row.offset).collect();
        assert_eq!(
            offsets,
            vec![data_start, data_start + ALIGN, data_start + 2 * ALIGN],
            "payloads must be written in path order, {ALIGN} bytes apart"
        );
        assert_eq!(
            bytes.len(),
            data_start + 2 * ALIGN + 1,
            "the archive ends with the last payload, no trailing padding"
        );
        for (index, row) in rows.iter().enumerate() {
            let end = if index + 1 < rows.len() {
                rows[index + 1].offset
            } else {
                bytes.len()
            };
            let slot = &bytes[row.offset..end];
            assert_eq!(slot[0], b"azk"[index], "payload of the {}th path", index);
            assert!(
                slot[1..].iter().all(|&byte| byte == 0),
                "gaps are zero filled"
            );
        }
    }

    #[test]
    fn round_trip_keeps_every_file() {
        let original = tiny_archive();
        let bytes = original.to_bytes().unwrap();
        let repacked = FtlArchive::from_bytes(&bytes).unwrap();
        assert!(original.diff(&repacked).is_empty());
        assert_eq!(repacked.get_file("data/alpha.xml"), Some(&b"a"[..]));
        assert_eq!(
            repacked.paths().collect::<Vec<_>>(),
            vec!["data/alpha.xml", "data/zeta.xml", "ships/kestrel.xml"]
        );
    }

    #[test]
    fn edits_and_removals_are_written() {
        let mut archive = tiny_archive();
        assert!(
            archive.set_file("data/alpha.xml", b"edited".to_vec()),
            "path existed"
        );
        assert!(
            !archive.set_file("data/new.xml", b"added".to_vec()),
            "path was new"
        );
        assert!(archive.remove_file("data/zeta.xml"));
        assert!(!archive.remove_file("data/zeta.xml"));

        let written = archive.to_bytes().unwrap();
        let repacked = FtlArchive::from_bytes(&written).unwrap();
        assert_eq!(repacked.get_file("data/alpha.xml"), Some(&b"edited"[..]));
        assert_eq!(repacked.get_file("data/new.xml"), Some(&b"added"[..]));
        assert_eq!(repacked.get_file("data/zeta.xml"), None);
        assert_eq!(repacked.len(), 3);
    }

    #[test]
    fn rejects_a_broken_archive() {
        let good = tiny_archive().to_bytes().unwrap();
        // bad magic
        assert!(FtlArchive::from_bytes(b"JUNK").is_err());
        // truncated
        assert!(FtlArchive::from_bytes(&good[..good.len() - 1]).is_err());
        // a checksum that does not match its path: corrupt the first index entry
        let mut tampered = good.clone();
        tampered[HEADER_SIZE] ^= 0xFF;
        assert!(FtlArchive::from_bytes(&tampered).is_err());
    }

    #[test]
    fn stock_checksums_are_path_hashes() {
        let bytes = stock_bytes();
        let archive = FtlArchive::from_bytes(&bytes).expect("data/ftl.dat did not parse");
        assert_eq!(archive.len(), 3219);
        assert_eq!(archive.trailing_bytes(), 0);
        for path in archive.paths() {
            let checksum = ftl_hash(path);
            assert_ne!(checksum, 0, "{path} hashed to nothing");
        }
        // from_bytes already refuses an archive whose stored checksums disagree
    }

    /// The claim of the whole exercise: an unmodified archive writes back out as
    /// the same bytes. Ignored by default, it moves ~600 MB through RAM.
    #[test]
    #[ignore = "reads and writes the 280 MB stock archive"]
    fn stock_round_trip_is_byte_identical() {
        let bytes = stock_bytes();
        let archive = FtlArchive::from_bytes(&bytes).unwrap();
        let out = archive.to_bytes().unwrap();
        assert_eq!(out.len(), bytes.len(), "length differs");
        // never assert_eq! two 280 MB buffers: the panic message would be
        // 280 MB of bytes
        if let Some(at) = out.iter().zip(bytes.iter()).position(|(a, b)| a != b) {
            panic!(
                "first difference at byte {at}: wrote {:#04x}, input had {:#04x}",
                out[at], bytes[at]
            );
        }

        let repacked = FtlArchive::from_bytes(&out).unwrap();
        assert!(archive.diff(&repacked).is_empty(), "payload diff");
    }

    #[test]
    #[ignore = "reads and writes the 280 MB stock archive"]
    fn stock_edit_changes_only_that_file() {
        let bytes = stock_bytes();
        let original = FtlArchive::from_bytes(&bytes).unwrap();
        let mut archive = FtlArchive::from_bytes(&bytes).unwrap();
        let path = archive
            .paths()
            .find(|path| path.ends_with(".xml"))
            .expect("no xml in the archive")
            .to_string();
        archive.set_file(&path, b"<xml>a much shorter payload</xml>".to_vec());

        // the edit touched one file, nothing else
        let problems = original.diff(&archive);
        assert_eq!(problems.len(), 1, "only {path} should differ: {problems:?}");
        assert!(problems[0].starts_with(&path));
        drop(original);

        // and the archive written from it holds what the edited archive says
        let written = archive.to_bytes().unwrap();
        let repacked = FtlArchive::from_bytes(&written).unwrap();
        assert!(
            archive.diff(&repacked).is_empty(),
            "the written archive must match the edited one"
        );
        assert_eq!(
            repacked.get_file(&path),
            Some(&b"<xml>a much shorter payload</xml>"[..])
        );
    }
}
