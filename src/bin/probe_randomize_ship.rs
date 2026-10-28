//! Draws ships from the pools of the archive, and prints what they look like.
//!
//! The pools come from `loadout::pools`, which reads them out of the game's own
//! `<weaponBlueprint>`, `<droneBlueprint>` and `<augBlueprint>` definitions, so
//! what this prints is what a run draws and not a copy of the numbers.
//!
//! It is the quick look at the shape of a loadout: does the draw stop, how many
//! of each thing does a ship get, how often does one come out with nothing to
//! shoot with, and what does the reactor add up to against the levels of the
//! systems that were asked for.
//!
//! ```text
//! cargo run --release --bin probe_randomize_ship
//! cargo run --release --bin probe_randomize_ship -- 5000 --dat data/ftl.dat
//! ```

use std::collections::BTreeMap;
use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use ftl_randomizer::loadout;
use ftl_randomizer::rng::Rng;
use ftl_randomizer::FtlArchive;

fn main() -> ExitCode {
    let mut runs = 500usize;
    let mut dat = PathBuf::from("data/ftl.dat");
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut at = 0;
    while at < argv.len() {
        match argv[at].as_str() {
            "--dat" => {
                at += 1;
                match argv.get(at) {
                    Some(path) => dat = PathBuf::from(path),
                    None => {
                        eprintln!("error: --dat needs a path");
                        return ExitCode::FAILURE;
                    }
                }
            }
            other => {
                runs = match other.parse() {
                    Ok(runs) => runs,
                    Err(error) => {
                        eprintln!("error: {other} is not a number of runs ({error})");
                        return ExitCode::FAILURE;
                    }
                }
            }
        }
        at += 1;
    }
    match draw(runs, &dat) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn draw(runs: usize, dat: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    std::fs::File::open(dat)?.read_to_end(&mut bytes)?;
    let archive = FtlArchive::from_bytes(&bytes)?;
    let randomizer = loadout::pools(&archive)?;
    let weights: BTreeMap<_, _> = randomizer
        .starting_augments_pool
        .iter()
        .zip(&randomizer.augment_weights)
        .map(|(augment, weight)| (augment.name.clone(), *weight))
        .collect();
    println!(
        "pools   {} weapons, {} drones, {} augments",
        randomizer.starting_weapons_pool.len(),
        randomizer.starting_drones_pool.len(),
        randomizer.starting_augments_pool.len(),
    );
    println!(
        "augurs  {}",
        weights
            .iter()
            .map(|(name, weight)| format!("{name} {weight}"))
            .collect::<Vec<_>>()
            .join(", ")
    );

    let mut rng = Rng::new(1);
    let mut seen: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut biggest = 0usize;
    let mut short = 0usize;
    let mut boarders = 0usize;
    let mut crewless = 0usize;
    println!(
        "\n{:>5}{:>5}{:>5}{:>5}{:>6}{:>8}{:>6}  the first few, and every twentieth",
        "run", "crew", "wpns", "drns", "aug", "reactor", "levels"
    );
    for run in 0..runs {
        // The four Kestrel style hulls: four weapon slots, two drone slots, and a
        // gun now that a hull of four slots carries one.
        let ship = randomizer.randomize_ship(&mut rng, true, false);
        let levels: usize = ship.systems.iter().map(|(_, level)| level).sum();
        biggest = biggest.max(ship.reactor);
        if ship.reactor < levels {
            short += 1;
        }
        if ship.weapons.is_empty() {
            boarders += 1;
        }
        crewless += usize::from(ship.crew.is_empty());
        for (name, level) in &ship.systems {
            let row = seen.entry(name.to_string()).or_insert((0, 0));
            row.0 += 1;
            row.1 = row.1.max(*level);
        }
        if ship.weapons.len() > 4 || ship.drones.len() > 3 {
            return Err(format!("run {run} drew {} weapons and {} drones", ship.weapons.len(), ship.drones.len()).into());
        }
        for must in ["pilot", "oxygen", "engines", "weapons"] {
            if !ship.systems.iter().any(|(name, _)| *name == must) {
                return Err(format!("run {run} has no {must} system").into());
            }
        }
        if run < 10 || run % 20 == 0 {
            let systems: Vec<String> = ship
                .systems
                .iter()
                .map(|(name, level)| format!("{name}{level}"))
                .collect();
            println!(
                "{run:>5}{:>5}{:>5}{:>5}{:>6}{:>8}{:>6}  {}",
                ship.crew.len(),
                ship.weapons.len(),
                ship.drones.len(),
                ship.augments.len(),
                ship.reactor,
                levels,
                systems.join(" "),
            );
        }
    }
    println!(
        "\n{runs} ships: biggest reactor {biggest}, {short} below their own system levels, \
         {boarders} with nothing to shoot with, {crewless} with no crew"
    );
    println!(
        "systems: {}",
        seen.iter()
            .map(|(name, (drawn, top))| format!("{name} {drawn}x lvl<={top}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(())
}