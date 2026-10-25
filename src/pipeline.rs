//! One run of the randomizer: the bytes of an `ftl.dat` in, the changed archive
//! and the `.ftl` mod out.
//!
//! Nothing here reads or writes files, so the command line and the web build run
//! the same code and a seed draws the same ships in both. The order of the three
//! passes is the order of the draws from one [`crate::rng::Rng`], and is what
//! makes a seed replay.

use std::collections::{BTreeMap, BTreeSet};

use crate::ftl_mod::{FtlMod, Meta};
use crate::rng::Rng;
use crate::{FtlArchive, loadout, rooms, ship_art};

/// What went wrong with a run. The passes each have their own error type, and a
/// caller only ever shows the message, so they are kept as they are.
pub type Error = Box<dyn std::error::Error>;
/// Result of a run.
pub type Result<T> = std::result::Result<T, Error>;

/// What the mod manager shows as the name.
pub const NAME: &str = "FTL Randomizer";
/// Read out of `Cargo.toml` at compile time, so a release cannot leave a mod
/// claiming the version of the one before it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const AUTHOR: &str = env!("CARGO_PKG_NAME");
/// The thread the manager links from the mod. Also read out of `Cargo.toml`.
pub const THREAD_URL: &str = env!("CARGO_PKG_HOMEPAGE");

/// What a run should draw, beyond the rooms.
#[derive(Debug)]
pub struct Options {
    /// Turn the colors of every hull.
    pub hue: bool,
    /// Draw a new loadout for every hull.
    pub loadout: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            hue: true,
            loadout: true,
        }
    }
}

/// Everything a run produced except the archive.
pub struct Outcome {
    /// The seed the run drew from.
    pub seed: u64,
    /// Files the archive held before anything was drawn. The run adds deck files
    /// for the hulls that ship none, so this is not `archive.len()` afterwards.
    pub read: usize,
    /// Rooms, doors and deck art.
    pub rooms: rooms::Report,
    /// The loadout, when one was drawn.
    pub gear: Option<loadout::Report>,
    /// Hue angle of each layout, for the report.
    pub hues: BTreeMap<String, i32>,
    /// Every file the run rewrote.
    pub changed: Vec<String>,
    /// The mod for a mod manager: the changed files and the metadata.
    pub ftl: Vec<u8>,
}

/// Read an archive, redraw the 28 player hulls, and pack the mod.
///
/// The patched archive comes back next to the reports so a caller can write the
/// whole `.dat` or unpack the result; it borrows `raw`, which has to outlive it.
pub fn process<'a>(
    raw: &'a [u8],
    seed: u64,
    options: &Options,
) -> Result<(FtlArchive<'a>, Outcome)> {
    let mut archive = FtlArchive::from_bytes(raw)?;
    let read = archive.len();
    let mut rng = Rng::new(seed);

    let rooms = rooms::randomize(&mut archive, &mut rng)?;

    // Then what the ships start with, into the rooms they now have.
    let gear = match options.loadout {
        true => Some(loadout::randomize(&mut archive, &mut rng)?),
        false => None,
    };

    // Then the colors. Every layout turns by its own angle, its gibs with it.
    let mut changed: BTreeSet<String> = rooms.changed.iter().cloned().collect();
    let mut hues = BTreeMap::new();
    if options.hue {
        let layouts: Vec<String> = rooms.ships.iter().map(|ship| ship.layout.clone()).collect();
        let art = ship_art::recolor(&mut archive, &mut rng, &layouts)?;
        hues = art.degrees;
        changed.extend(art.changed);
    }
    if let Some(gear) = &gear {
        changed.extend(gear.changed.iter().cloned());
    }

    let mut module = FtlMod::new();
    for path in &changed {
        let payload = archive
            .get_file(path)
            .ok_or_else(|| crate::Error::Format(format!("the archive lost {path}")))?;
        module.add(path, payload);
    }
    let about = description(seed);
    module.describe(&Meta {
        title: NAME,
        thread_url: THREAD_URL,
        author: AUTHOR,
        version: VERSION,
        description: &about,
    });

    Ok((
        archive,
        Outcome {
            seed,
            read,
            rooms,
            gear,
            hues,
            changed: changed.into_iter().collect(),
            ftl: module.to_bytes(),
        },
    ))
}

/// What the mod manager shows as the description. The seed belongs here: it is the
/// only way back to the ships of a `.ftl` once the run that drew them is gone, so
/// a mod found in a folder months later can still be drawn again.
pub fn description(seed: u64) -> String {
    format!("Made with {AUTHOR} {VERSION} from seed {seed}.")
}