//! The guns of the enemy ships.
//!
//! An enemy ship does not file its weapons one by one, it points at a list of them:
//! `<weaponList missiles="10" load="WEAPONS_REBEL"/>`. The `load` names a
//! `<blueprintList>`, and the game draws the ship's guns out of that list when the
//! ship is spawned. The lists the game ships hold the guns of one faction each, so a
//! rebel ship only ever carries rebel guns.
//!
//! This writes one list of its own into `data/autoBlueprints.xml`, where the game
//! files the lists its enemies draw from, and it holds every gun of
//! [`crate::randomizer::ALLOWED_ENEMY_WEAPONS_NAMES`], in that order and all of them.
//! Every `<weaponList load="WEAPONS_...">` of the archive is then pointed at it, and
//! the game picks the guns itself, out of the whole pool. Nothing here chooses a
//! weapon, and no seed is drawn, so the run is the same on every seed.
//!
//! A `<weaponList>` that files its weapons one by one belongs to a boss, a set fight
//! or an event, and keeps them exactly as they were filed. `<droneList>` is left
//! alone the same way: only the guns are drawn.
//!
//! Nothing is re-serialized: every change is a byte range of the file, as in
//! [`crate::blueprints`], so the comments and the spacing of the blueprint files come
//! back as they went in.

use std::collections::BTreeSet;
use std::fmt;
use std::ops::Range;

use crate::blueprints::{self, Element};
use crate::randomizer::ALLOWED_ENEMY_WEAPONS_NAMES;
use crate::FtlArchive;

/// The list the ships are pointed at. The game files no list of this name and keeps
/// no rule about it: it is only the name a ship writes in its `load`.
pub const LIST: &str = "WEAPONS_RANDOM";

/// Where the list is filed, which is where the game files the lists its enemies draw
/// from. `data/dlcPirateBlueprints.xml` points at lists that `data/autoBlueprints.xml`
/// files, so a list filed there is read by every file that wants one.
const HOME: &str = "data/autoBlueprints.xml";

/// What went wrong with an archive.
#[derive(Debug)]
pub enum Error {
    /// A file the pass needs is not in the archive.
    Missing { path: String },
    /// A file that could not be read, or a gun the archive does not define. `why`
    /// says what was not as expected and where to look.
    Unreadable { path: String, why: String },
    /// The archive itself.
    Archive(crate::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Missing { path } => write!(out, "the archive has no {path}"),
            Error::Unreadable { path, why } => {
                if path.is_empty() {
                    write!(out, "{why}")
                } else {
                    write!(out, "{path}: {why}")
                }
            }
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

/// What a run changed.
pub struct Report {
    /// Guns the list holds.
    pub entries: usize,
    /// `<weaponList>`s pointed at it.
    pub pointed: usize,
    /// The lists the game shipped in its place.
    pub replaced: Vec<String>,
    /// The guns of the list, for whoever wants to see them.
    pub guns: Vec<String>,
    /// Archive paths that were written.
    pub changed: Vec<String>,
    /// Anything that could not be done cleanly.
    pub warnings: Vec<String>,
}

/// Point every enemy ship that draws its guns from a list at one list of the whole
/// pool. A ship that files its guns one by one is not touched.
pub fn randomize(archive: &mut FtlArchive) -> Result<Report> {
    // A name the game does not define would put a gun the game cannot draw into the
    // list, so the pool is checked against the blueprints before a thing is written.
    let known = defined(archive)?;
    let missing: Vec<&str> = ALLOWED_ENEMY_WEAPONS_NAMES
        .iter()
        .copied()
        .filter(|name| !known.contains(*name))
        .collect();
    if !missing.is_empty() {
        return Err(Error::Unreadable {
            path: "the blueprint files of the archive".to_string(),
            why: format!("no <weaponBlueprint> named {}", missing.join(", ")),
        });
    }

    let mut changed: BTreeSet<String> = BTreeSet::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut replaced: BTreeSet<String> = BTreeSet::new();
    let mut pointed = 0usize;

    for path in archive.paths().map(str::to_string).collect::<Vec<_>>() {
        if !path.starts_with("data/") || !path.ends_with(".xml") {
            continue;
        }
        let bytes = archive.get_file(&path).expect("path came from the archive");
        // Only a file that files a `<weaponList>` can hold a gun list to point, and
        // the test is on the tag rather than on `load="WEAPONS_`, which the game
        // tolerates spaces inside of.
        if !holds(bytes, b"<weaponList") {
            continue;
        }
        let text = String::from_utf8(bytes.to_vec()).map_err(|error| Error::Unreadable {
            path: path.clone(),
            why: format!("not utf8: {error}"),
        })?;
        let inside = walk(&path, &text)?;
        let mut edits: Vec<(Range<usize>, String)> = Vec::new();
        for list in inside.iter().filter(|element| element.name == "weaponList") {
            // A `<weaponList>` that names no `load` files its weapons itself, and is
            // left to them. A commented out one is never read by the walk.
            let Some((value, held)) = blueprints::attribute(&text[list.tag.clone()], "load")
            else {
                continue;
            };
            if !held.starts_with("WEAPONS_") {
                warnings.push(format!(
                    "{path}: a <weaponList> draws from {held}, which is not a gun list, and kept its guns"
                ));
                continue;
            }
            if held == LIST {
                // An archive this program already wrote.
                continue;
            }
            edits.push((
                list.tag.start + value.start..list.tag.start + value.end,
                LIST.to_string(),
            ));
            pointed += 1;
            replaced.insert(held.to_string());
        }
        if edits.is_empty() {
            continue;
        }
        archive.set_file(&path, blueprints::rewrite(&text, &edits));
        changed.insert(path);
    }

    // A list no ship draws from is a list nobody reads, so it is only filed once a
    // ship was pointed at it.
    if pointed > 0 {
        define(archive, &mut changed)?;
    }

    Ok(Report {
        entries: ALLOWED_ENEMY_WEAPONS_NAMES.len(),
        pointed,
        replaced: replaced.into_iter().collect(),
        guns: ALLOWED_ENEMY_WEAPONS_NAMES.iter().map(|name| name.to_string()).collect(),
        changed: changed.into_iter().collect(),
        warnings,
    })
}

/// File the list under the `<FTL>` root of [`HOME`] and note the file as changed.
///
/// The root is where the game files its own `<blueprintList>`s, and a list has to be
/// filed in a file the game reads before the ships that point at it: the ships of
/// `dlcPirateBlueprints.xml` already point at lists filed here, so a list filed here
/// is going to be read by all of them.
fn define(archive: &mut FtlArchive, changed: &mut BTreeSet<String>) -> Result<()> {
    let bytes = archive
        .get_file(HOME)
        .ok_or_else(|| Error::Missing { path: HOME.to_string() })?;
    let text = String::from_utf8(bytes.to_vec()).map_err(|error| Error::Unreadable {
        path: HOME.to_string(),
        why: format!("not utf8: {error}"),
    })?;
    if text.contains(&format!("name=\"{LIST}\"")) {
        // The list of a previous run, which holds what this build of the program
        // puts in it: the same archive written again is written the same.
        return Ok(());
    }
    let inside = walk(HOME, &text)?;
    let root = inside.iter().find(|element| element.name == "FTL").ok_or_else(|| {
        Error::Unreadable {
            path: HOME.to_string(),
            why: "no <FTL> root to file the list under".to_string(),
        }
    })?;
    let nl = newline(&text);
    let entries: String = ALLOWED_ENEMY_WEAPONS_NAMES
        .iter()
        .map(|name| format!("{nl}\t<name>{name}</name>"))
        .collect();
    let block = format!("{nl}<blueprintList name=\"{LIST}\">{entries}{nl}</blueprintList>");
    archive.set_file(HOME, blueprints::rewrite(&text, &[(root.tag.end..root.tag.end, block)]));
    changed.insert(HOME.to_string());
    Ok(())
}

/// Every `<weaponBlueprint>` name of the archive. The files are walked in path order
/// and a later definition wins, which is what makes `dlcBlueprintsOverwrite.xml`
/// overwrite.
fn defined(archive: &FtlArchive) -> Result<BTreeSet<String>> {
    let mut names: BTreeSet<String> = BTreeSet::new();
    for path in archive.paths().map(str::to_string).collect::<Vec<_>>() {
        if !path.starts_with("data/") || !path.ends_with(".xml") {
            continue;
        }
        let bytes = archive.get_file(&path).expect("path came from the archive");
        if !holds(bytes, b"<weaponBlueprint") {
            continue;
        }
        let text = String::from_utf8(bytes.to_vec()).map_err(|error| Error::Unreadable {
            path: path.clone(),
            why: format!("not utf8: {error}"),
        })?;
        for element in walk(&path, &text)? {
            if element.name != "weaponBlueprint" {
                continue;
            }
            // A definition names itself. The `<weaponBlueprint>` of a
            // `<droneBlueprint>`, which points at one, holds no name attribute.
            if let Some((_, name)) = blueprints::attribute(&text[element.tag.clone()], "name") {
                names.insert(name.to_string());
            }
        }
    }
    Ok(names)
}

/// The elements of a file, with a message that says which file it was.
fn walk(path: &str, text: &str) -> Result<Vec<Element>> {
    blueprints::elements(text, 0).map_err(|why| Error::Unreadable {
        path: path.to_string(),
        why,
    })
}

/// The line ending the file uses.
fn newline(text: &str) -> &'static str {
    match text.contains("\r\n") {
        true => "\r\n",
        false => "\n",
    }
}

/// true when `hay` holds `needle`.
fn holds(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|window| window == needle)
}