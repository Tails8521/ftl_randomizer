//! Turn the colour of the enemy ships.
//!
//! An enemy ship is drawn from one art name, which its `<shipBlueprint>` files in
//! `img=`: `img/ships_noglow/<art>_base.png` and its four gibs, and the same five
//! files in `img/ships_glow`, which glow behind them. One angle turns all ten files
//! of a ship, so a hull, the wreck it breaks into and the glow of it keep one colour.
//! It is the rule `ship_art.rs` follows for the player hulls, whose art sits in
//! `img/ship` under the layout name and is turned there.
//!
//! The ship, rather than the art, is what gets an angle, because the game draws more
//! than one ship from a single art and the ships are what is to be told apart. The
//! rebel flagship is drawn from three arts for its six layouts - `boss_1` stands for
//! `boss_1` and for `boss_1_easy` alike, the first flown on a hard run and the second
//! on an easy or a normal one - and every one of the six is a ship of its own with a
//! colour of its own. For a ship whose art serves another layout too, this pass files
//! a copy of the art under the layout's own name and points the `img=` of those
//! blueprints at the copy, which is how the game ships its own variants:
//! `rock_scout_pirate` is an art of its own, named by `img=`.
//!
//! One art of the archive cannot be split that way: `jelly_truffle_pirate` is drawn by
//! the layouts `jelly_truffle_pirate` and `jelly_truffle`, and a copy of it would have
//! to be filed under a name that is already an art. Those two ships share one colour,
//! and the pass says so.
//!
//! The cloak sprite `img/ship/<art>_cloak.png` goes with a copy unturned. It is the
//! blue ghost the game draws while a ship is cloaked, and it is left as it ships on a
//! player hull too.
//!
//! Nothing here touches the filesystem. Of the 45 enemy ships the archive files, 44
//! are turned - one has no art to turn - and 440 images are written, 30 of them under a
//! name the game ships no files for. The images of a ship, the copy of them when there
//! is one, and the blueprint files that name an art are all that is written.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use crate::blueprints::{self, Patch};
use crate::rng::Rng;
use crate::ship_art::{self, Turned};
use crate::FtlArchive;

/// What went wrong with the archive. Its own type, like the other passes, so a
/// caller only ever shows the message.
#[derive(Debug)]
pub enum Error {
    /// A file that could not be read.
    Unreadable { path: String, why: String },
    /// The archive itself.
    Archive(crate::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unreadable { path, why } => write!(out, "{path}: {why}"),
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

/// One enemy ship whose colours were turned.
#[derive(Debug)]
pub struct Ship {
    pub layout: String,
    /// The art it was drawn from.
    pub art: String,
    /// The art it is drawn from now, which is another name when this ship had to be
    /// given a copy of an art the game shares out.
    pub into: String,
    pub degrees: i32,
    /// The images written for it. Nought when the ship is drawn from an art another
    /// ship was turned first, which is the colour it shares.
    pub files: usize,
    /// The blueprints pointed at `into`.
    pub blueprints: Vec<String>,
}

/// What the pass changed.
pub struct Report {
    pub ships: Vec<Ship>,
    /// Archive paths that were written.
    pub changed: Vec<String>,
    /// Anything that could not be done cleanly.
    pub warnings: Vec<String>,
}

/// Turn every enemy ship of `archive` by an angle of its own. A ship whose art the
/// game shares with another ship of another layout comes back with a copy of that art
/// under its own name, and its blueprints pointed at it.
pub fn recolor(archive: &mut FtlArchive, rng: &mut Rng) -> Result<Report> {
    let foes = fleet(archive)?;

    // The layouts each art is drawn by: an art of more than one is what has to be
    // copied to give every ship a colour of its own.
    let mut drawn: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for foe in &foes {
        drawn.entry(foe.art.as_str()).or_default().insert(foe.layout.as_str());
    }
    // Every art name of the archive, so a copy is never filed over the art of a ship
    // that already has one.
    let arts: BTreeSet<String> = archive
        .paths()
        .filter_map(|path| path.strip_prefix("img/ships_noglow/"))
        .filter_map(|name| name.strip_suffix("_base.png"))
        .map(str::to_string)
        .collect();

    // The ships, in the order of their names, so the seed is what draws the angles
    // and not the order the blueprint files were read in.
    let ships: BTreeSet<(&str, &str)> = foes
        .iter()
        .map(|foe| (foe.layout.as_str(), foe.art.as_str()))
        .collect();

    let mut report = Report {
        ships: Vec::new(),
        changed: Vec::new(),
        warnings: Vec::new(),
    };
    let mut edits: BTreeMap<String, Vec<Patch>> = BTreeMap::new();
    // The angle each art name was turned by, and the art that was turned into it. One
    // name holds one colour, since it is one set of files: two ships drawn from one
    // name that could not be split share what was drawn for the first of them.
    let mut turned: BTreeMap<String, (i32, String)> = BTreeMap::new();

    for (layout, art) in ships {
        let into = target(layout, art, &drawn, &arts, &mut report.warnings);
        let (angle, from, fresh) = match turned.entry(into.clone()) {
            Entry::Occupied(had) => (had.get().0, had.get().1.clone(), false),
            Entry::Vacant(want) => {
                let angle = ship_art::TURN_MIN
                    + rng.below((ship_art::TURN_MAX - ship_art::TURN_MIN + 1) as u64) as i32;
                want.insert((angle, art.to_string()));
                (angle, art.to_string(), true)
            }
        };

        let files = if fresh {
            match turn_ship(archive, art, &into, angle, &mut report.changed)? {
                Some(files) => files,
                None => {
                    report.warnings.push(format!(
                        "{layout}: nothing is drawn from art {art}, so its colours stayed as \
                         they are"
                    ));
                    continue;
                }
            }
        } else {
            if from != art {
                report.warnings.push(format!(
                    "{layout}: {into} is already drawn from the art of another ship, so a copy \
                     of {art} could not be filed there and this ship keeps that colour"
                ));
            }
            0
        };
        if fresh && into != art {
            copy_cloak(archive, art, &into, &mut report.changed);
        }

        // Every blueprint of this ship that names the old art has to be told the new
        // name. A blueprint that named no art is drawn from its layout's name
        // already, which is the name the copy would have had anyway.
        let mut names = Vec::new();
        for foe in foes.iter().filter(|foe| foe.layout == layout && foe.art == art) {
            let Some(at) = &foe.at else { continue };
            edits
                .entry(foe.file.clone())
                .or_default()
                .push((at.clone(), into.clone()));
            names.push(foe.name.clone());
        }
        report.ships.push(Ship {
            layout: layout.to_string(),
            art: art.to_string(),
            blueprints: names,
            files,
            degrees: angle,
            into,
        });
    }

    for (path, patches) in edits {
        let text = String::from_utf8(
            archive
                .get_file(&path)
                .ok_or_else(|| Error::Unreadable {
                    path: path.clone(),
                    why: "the archive lost a blueprint file".to_string(),
                })?
                .to_vec(),
        )
        .map_err(|error| Error::Unreadable {
            path: path.clone(),
            why: format!("not utf8: {error}"),
        })?;
        let bytes = blueprints::rewrite(&text, &patches);
        archive.set_file(&path, bytes);
        if !report.changed.contains(&path) {
            report.changed.push(path);
        }
    }
    report.changed.sort();

    Ok(report)
}

/// One `<shipBlueprint>` of an enemy ship, as far as its colours go.
struct Foe {
    /// The blueprint file that holds it.
    file: String,
    name: String,
    layout: String,
    /// The art it is drawn with: what `img=` names, or its own layout when it names
    /// no art, which is what the game falls back to.
    art: String,
    /// Where the `img="..."` value sits in `file`, when it names an art, so it can be
    /// pointed at a copy.
    at: Option<Range<usize>>,
}

/// Every enemy `<shipBlueprint>` of the archive. A blueprint of a player hull is left
/// to `ship_art.rs`, which turns the art of those under the layout name.
fn fleet(archive: &FtlArchive) -> Result<Vec<Foe>> {
    let mut foes = Vec::new();
    for path in archive.paths().map(str::to_string).collect::<Vec<_>>() {
        if !path.starts_with("data/") || !path.ends_with(".xml") {
            continue;
        }
        let bytes = archive.get_file(&path).expect("path came from the archive");
        if !holds(bytes, b"<shipBlueprint") {
            continue;
        }
        let text = String::from_utf8(bytes.to_vec()).map_err(|error| Error::Unreadable {
            path: path.clone(),
            why: format!("not utf8: {error}"),
        })?;
        for element in blueprints::elements(&text, 0).map_err(|why| Error::Unreadable {
            path: path.clone(),
            why,
        })? {
            if element.name != "shipBlueprint" {
                continue;
            }
            let tag = &text[element.tag.clone()];
            let Some((_, name)) = blueprints::attribute(tag, "name") else {
                continue;
            };
            let Some((_, layout)) = blueprints::attribute(tag, "layout") else {
                continue;
            };
            // A player hull files art of its own in `img/ship` and is turned there.
            if archive
                .get_file(&format!("img/ship/{layout}_base.png"))
                .is_some()
            {
                continue;
            }
            let (at, art) = match blueprints::attribute(&text[element.tag.clone()], "img") {
                Some((value, img)) => (Some(element.tag.start + value.start..element.tag.start + value.end), img.to_string()),
                // No art of its own: the game draws it from the layout, which is
                // what the art of the layouts of the enemy ships is named after
                // anyway.
                None => (None, layout.to_string()),
            };
            foes.push(Foe {
                file: path.clone(),
                name: name.to_string(),
                layout: layout.to_string(),
                art,
                at,
            });
        }
    }
    Ok(foes)
}

/// The art the ship will be drawn from: the one it ships with when nothing else is
/// drawn from it, or when the layout and the art already share their name, and the
/// layout's own name - a copy of the art this pass files - when the art serves
/// several layouts.
fn target(
    layout: &str,
    art: &str,
    drawn: &BTreeMap<&str, BTreeSet<&str>>,
    arts: &BTreeSet<String>,
    warnings: &mut Vec<String>,
) -> String {
    let shared = drawn.get(art).is_some_and(|layouts| layouts.len() > 1);
    if !shared || art == layout {
        return art.to_string();
    }
    if arts.contains(&layout.to_string()) {
        warnings.push(format!(
            "{layout}: art {art} is drawn by several layouts and {layout} is an art name of its \
             own, so these layouts keep sharing one art and one colour"
        ));
        return art.to_string();
    }
    layout.to_string()
}

/// The ten images of `art`, turned by `degrees` and written under the name `into`:
/// the count of the files written, or `None` when the archive holds no image of `art`
/// at all. A file of the art that the archive lost is skipped, and the copy of it is
/// never filed under the new name.
fn turn_ship(
    archive: &mut FtlArchive,
    art: &str,
    into: &str,
    degrees: i32,
    changed: &mut Vec<String>,
) -> Result<Option<usize>> {
    // One table of colors for all ten files: the gibs are cut out of the same art, so
    // a color seen in the hull is not converted again, and both halves of a ship turn
    // to the same colors.
    let mut hues: Turned = Turned::default();
    let mut written = 0;
    for (path, copy) in images(art).into_iter().zip(images(into)) {
        let Some(bytes) = archive.get_file(&path) else {
            continue;
        };
        let turned = ship_art::turn(bytes, degrees, &mut hues).map_err(|why| Error::Unreadable {
            path: path.clone(),
            why,
        })?;
        archive.set_file(&copy, turned);
        if !changed.contains(&copy) {
            changed.push(copy);
        }
        written += 1;
    }
    Ok((written > 0).then_some(written))
}

/// The cloak sprite of a copied art, filed under the new name as it ships. The game
/// looks it up by the art, and an art of the enemy ships that has no cloak sprite of
/// its own - the pirate variants among them - is a thing the game already does.
fn copy_cloak(archive: &mut FtlArchive, art: &str, into: &str, changed: &mut Vec<String>) {
    let Some(bytes) = archive.get_file(&cloak(art)).map(|bytes| bytes.to_vec()) else {
        return;
    };
    archive.set_file(&cloak(into), bytes);
    if !changed.contains(&cloak(into)) {
        changed.push(cloak(into));
    }
}

/// The ten images of an art, each with the file it was written to. For a ship that
/// kept its art, both halves of every pair are the same file.
pub fn pairs(art: &str, into: &str) -> Vec<(String, String)> {
    images(art).into_iter().zip(images(into)).collect()
}

/// The images an art is drawn from, in the order they are turned in: the hull and its
/// four gibs, and the same five files of the glow that sits behind them.
fn images(art: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for folder in ["ships_noglow", "ships_glow"] {
        for part in ["base", "gib1", "gib2", "gib3", "gib4"] {
            paths.push(format!("img/{folder}/{art}_{part}.png"));
        }
    }
    paths
}

/// The blue ghost the game draws while a ship of this art is cloaked.
fn cloak(art: &str) -> String {
    format!("img/ship/{art}_cloak.png")
}

/// true when `hay` holds `needle`.
fn holds(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|window| window == needle)
}