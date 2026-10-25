//! WebAssembly front end: the same run as the command line, without a filesystem.
//!
//! The page hands over the bytes of an `ftl.dat` and the seed, and gets back the
//! two files a person can save. Everything runs in memory, so one
//! [`Job`] holds the archive that was dropped until the caller is done with it.
//!
//! ```text
//! const job = ftl.open(new Uint8Array(buffer), seed);
//! const mod = job.ftl();      // the .ftl for a mod manager
//! const dat = job.dat();      // the whole patched ftl.dat
//! job.reseed(seed);           // the same archive, drawn from another seed
//! ```
//!
//! A `Vec<u8>` is taken by value rather than a `&[u8]`: the glue copies a slice
//! argument into the wasm heap and never frees it, which for a 270 MiB archive a
//! person drops twice is 270 MiB lost.

use wasm_bindgen::prelude::*;

use crate::pipeline::{self, Options};

/// An archive that was loaded, with the ships already drawn.
#[wasm_bindgen]
pub struct Job {
    /// The archive as it was dropped. Kept so the `.dat` can be written on
    /// request without asking for the file again.
    raw: Vec<u8>,
    seed: u64,
    /// The mod, which is built when the file is read.
    ftl: Vec<u8>,
    /// Files the run rewrote, for the page to report.
    changed: usize,
    warnings: Vec<String>,
}

#[wasm_bindgen]
impl Job {
    /// Bytes of the `.ftl` mod for a mod manager.
    pub fn ftl(&self) -> Vec<u8> {
        self.ftl.clone()
    }

    /// The whole patched `ftl.dat`.
    ///
    /// The ships are drawn again from the same seed, which draws the same ships,
    /// so the two files always agree. It is the slow call: it reads the archive
    /// a second time and repacks all of it.
    pub fn dat(&self) -> Result<Vec<u8>, String> {
        let (archive, _) = pipeline::process(&self.raw, self.seed, &Options::default())
            .map_err(message)?;
        archive.to_bytes().map_err(message)
    }

    /// The seed the ships were drawn from.
    pub fn seed(&self) -> f64 {
        self.seed as f64
    }

    /// Draw the ships again from another seed, with the archive still loaded, so
    /// a person who does not like the ships only has to type a number.
    pub fn reseed(&mut self, seed: f64) -> Result<(), String> {
        let seed = whole(seed)?;
        // Not bound, so the archive is dropped here and stops borrowing `self.raw`
        // before the results are written back.
        let (_, outcome) =
            pipeline::process(&self.raw, seed, &Options::default()).map_err(message)?;
        self.seed = seed;
        self.changed = outcome.changed.len();
        self.warnings = warnings(&outcome);
        self.ftl = outcome.ftl;
        Ok(())
    }

    /// How many files of the game changed.
    pub fn changed(&self) -> usize {
        self.changed
    }

    /// Anything the run thought was worth saying.
    pub fn warnings(&self) -> Vec<String> {
        self.warnings.clone()
    }
}

/// Read an `ftl.dat` and draw the 28 player ships: their rooms, their loadouts
/// and their colours. `seed` is a whole number; the same number draws the same
/// ships.
#[wasm_bindgen]
pub fn open(raw: Vec<u8>, seed: f64) -> Result<Job, String> {
    let seed = whole(seed)?;

    // The archive is not bound, so it is dropped here and stops borrowing `raw`
    // before the job takes it.
    let (_, outcome) = pipeline::process(&raw, seed, &Options::default()).map_err(message)?;
    let warnings = warnings(&outcome);
    Ok(Job {
        raw,
        seed,
        changed: outcome.changed.len(),
        ftl: outcome.ftl,
        warnings,
    })
}

/// The seed a page sent, which arrives as a number. A number that is not a whole
/// one is refused rather than rounded: the seed is worth writing down, and a
/// rounded one would not be the number in the box.
fn whole(seed: f64) -> Result<u64, String> {
    match seed.is_finite() && seed.fract() == 0.0 && seed >= 0.0 {
        // f64 holds whole numbers exactly up to 2^53, far past any seed worth
        // typing, and the cast saturates above that.
        true => Ok(seed.min(u64::MAX as f64) as u64),
        false => Err("the seed has to be a whole number of 0 or more".to_string()),
    }
}

/// The warnings of both passes, in the order the passes ran.
fn warnings(outcome: &pipeline::Outcome) -> Vec<String> {
    let mut warnings = outcome.rooms.warnings.clone();
    if let Some(gear) = &outcome.gear {
        warnings.extend(gear.warnings.iter().cloned());
    }
    warnings
}

/// An error a page can show.
fn message(error: impl std::fmt::Display) -> String {
    error.to_string()
}
