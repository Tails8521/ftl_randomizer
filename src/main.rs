//! Give every player ship a new room layout and a new loadout.
//!
//! All filesystem access lives here. The library only ever sees bytes: the archive
//! is read into a buffer, edited in memory, and written back out as a `.dat` for a
//! plain game install and a `.ftl` for a mod manager.
//!
//! The seed is printed first, and the same seed draws the same ships.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use ftl_randomizer::ftl_mod::FtlMod;
use ftl_randomizer::rng::Rng;
use ftl_randomizer::{loadout, rooms, ship_art, FtlArchive};
use std::collections::BTreeMap;

const NAME: &str = "Random Rooms";
const VERSION: &str = "0.1";
const AUTHOR: &str = "ftl_randomizer";
const DESCRIPTION: &str = "Every player ship gets a new room layout, a new loadout \
                           of weapons, drones, crew, augments and systems, the deck \
                           under it redrawn to fit, and every ship a new colour. \
                           Built from one seed: the same seed builds the same ships.";

const USAGE: &str = "\
randomize the room layout and the loadout of the 28 player ships of an ftl.dat

USAGE:
  ftl_randomizer [SEED] [OPTIONS]

  SEED   a number. The same seed draws the same ships. Without one the clock
         supplies it, and prints it, so the run can be drawn again.

OPTIONS:
  --in PATH      archive to read                 (default data/ftl.dat)
  --dat PATH     write the patched archive here   (default out/ftl.dat)
  --ftl PATH     write the mod for a mod manager  (default out/random_rooms.ftl)
  --no-dat       do not write the archive
  --no-ftl       do not write the mod
  --no-hue       leave the ship art its own colours
  --no-loadout   leave the weapons, drones, crew, augments and systems as they are
  --extract DIR  also unpack every file of the result under DIR
  --systems      print the room each system was moved into
  --loadout      print what each ship starts with
  --help         this text

Each ship is redrawn as it flies: the hull art is cut up and moved with the
rooms, and the deck under them is drawn again to fit, in the grey and the black
edge the ship's own deck was painted with. The eight C hulls borrow the floor of
the A ship as the game ships them, and are pointed at the deck drawn for them.

Each ship is then equipped anew from the game's own weapon, drone and augment
definitions: which systems it starts with and at what level, its reactor power,
its weapons and their missiles, its drones and their parts, its crew - of any
race - and its augments, drawn so that the ship can break a shield or put crew
aboard.

The .dat is a whole game archive: drop it over the one the game reads. The .ftl
holds only the files that changed, which is what a mod manager wants.";

/// What the command line asked for.
struct Args {
    seed: u64,
    input: PathBuf,
    dat: Option<PathBuf>,
    ftl: Option<PathBuf>,
    extract: Option<PathBuf>,
    systems: bool,
    /// Draw a loadout at all, and print the one that was drawn.
    loadout_pass: bool,
    loadout: bool,
    hue: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;

    let bytes = fs::read(&args.input)
        .map_err(|error| format!("cannot read {}: {error}", args.input.display()))?;
    let mut archive = FtlArchive::from_bytes(&bytes)?;
    println!(
        "read {} files from {} ({})",
        archive.len(),
        args.input.display(),
        size(bytes.len())
    );

    let mut rng = Rng::new(args.seed);
    let report = rooms::randomize(&mut archive, &mut rng)?;

    // Then what the ships start with, into the rooms they now have.
    let gear = match args.loadout_pass {
        true => Some(loadout::randomize(&mut archive, &mut rng)?),
        false => None,
    };

    // Then the colours. Every layout turns by its own angle, its gibs with it.
    let mut changed = report.changed.clone();
    let mut hues: BTreeMap<String, i32> = BTreeMap::new();
    if args.hue {
        let layouts: Vec<String> = report.ships.iter().map(|ship| ship.layout.clone()).collect();
        let art = ship_art::recolour(&mut archive, &mut rng, &layouts)?;
        hues = art.degrees;
        changed.extend(art.changed);
    }
    if let Some(gear) = &gear {
        changed.extend(gear.changed.iter().cloned());
    }

    println!("seed {}", args.seed);
    println!(
        "{:<22}{:>6}{:>7}{:>7}{:>9}{:>6}{:>9}  blueprints",
        "layout", "hull", "rooms", "doors", "airlocks", "hue", "deck"
    );
    for ship in &report.ships {
        println!(
            "{:<22}{:>6}{:>7}{:>7}{:>9}{:>6}{:>9}  {}",
            ship.layout,
            ship.hull,
            ship.rooms,
            ship.doors,
            ship.airlocks,
            match hues.get(&ship.layout) {
                Some(angle) => format!("{angle}°"),
                None => "-".to_string(),
            },
            // A `+` marks a hull that ships no deck art of its own, so the file
            // of this run is the first one the game has ever had for it.
            format!("{}{}", ship.deck, if ship.shipped { "" } else { "+" }),
            ship.blueprints.join(", ")
        );
    }

    if args.systems {
        for change in &report.moves {
            println!("\n{} on the {}", change.blueprint, change.layout);
            for (system, room) in &change.systems {
                println!("  {system:<12} room {room}");
            }
        }
    }
    if let Some(gear) = &gear {
        println!(
            "\n{:<24}{:<20}{:>5}{:>5}{:>5}{:>5}{:>7}{:>7}{:>7}  crew",
            "blueprint", "layout", "crew", "wpns", "drns", "augs", "power", "msils", "parts"
        );
        for ship in &gear.ships {
            println!(
                "{:<24}{:<20}{:>5}{:>5}{:>5}{:>5}{:>7}{:>7}{:>7}  {}",
                ship.name,
                ship.layout,
                ship.crew.iter().map(|(_, number)| number).sum::<usize>(),
                ship.weapons.len(),
                ship.drones.len(),
                ship.augments.len(),
                ship.reactor,
                ship.missiles,
                ship.drone_parts,
                ship.crew_text(),
            );
        }
    }
    if args.loadout {
        for ship in gear.iter().flat_map(|gear| gear.ships.iter()) {
            println!("\n{} on the {}", ship.name, ship.layout);
            let list = |names: &[String]| match names.is_empty() {
                true => "-".to_string(),
                false => names.join(", "),
            };
            let row = |what: &str, with: String| println!("  {what:<12}{with}");
            row("power", ship.reactor.to_string());
            row("missiles", ship.missiles.to_string());
            row("drone parts", ship.drone_parts.to_string());
            row("crew", ship.crew_text());
            row("weapons", list(&ship.weapons));
            row("drones", list(&ship.drones));
            row("augments", list(&ship.augments));
            if !ship.artillery.is_empty() {
                row("gun", ship.artillery.clone());
            }
            let systems: Vec<String> = ship
                .systems
                .iter()
                .map(|(name, level)| format!("{name} {level}"))
                .collect();
            row("systems", systems.join(", "));
        }
    }
    for warning in &report.warnings {
        println!("warning: {warning}");
    }
    if let Some(gear) = &gear {
        for warning in &gear.warnings {
            println!("warning: {warning}");
        }
    }

    if let Some(path) = &args.dat {
        let packed = archive.to_bytes()?;
        write(path, &packed)?;
        println!(
            "\nwrote the patched archive to {} ({}): put it over the game's ftl.dat",
            path.display(),
            size(packed.len())
        );
    }
    if let Some(path) = &args.ftl {
        let mut module = FtlMod::new();
        for changed in &changed {
            let payload = archive
                .get_file(changed)
                .ok_or_else(|| io::Error::other(format!("the archive lost {changed}")))?;
            module.add(changed, payload);
        }
        module.describe(NAME, VERSION, AUTHOR, DESCRIPTION);
        let packed = module.to_bytes();
        write(path, &packed)?;
        println!(
            "wrote the mod to {} ({}): {} changed files, install it with a mod manager",
            path.display(),
            size(packed.len()),
            module.len()
        );
    }

    if let Some(dir) = &args.extract {
        for (path, payload) in archive.files() {
            write(&dir.join(path), payload)?;
        }
        println!(
            "unpacked the result under {}, {} files changed",
            dir.display(),
            changed.len()
        );
    }
    Ok(())
}

/// Read the command line.
fn parse_args() -> Result<Args, Box<dyn std::error::Error>> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut seed: Option<u64> = None;
    let mut input = PathBuf::from("data/ftl.dat");
    let mut dat = Some(PathBuf::from("out/ftl.dat"));
    let mut ftl = Some(PathBuf::from("out/random_rooms.ftl"));
    let mut extract = None;
    let mut systems = false;
    let mut loadout_pass = true;
    let mut loadout = false;
    let mut hue = true;

    let mut at = 0;
    while at < argv.len() {
        match argv[at].as_str() {
            "--help" | "-h" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            "--in" => input = take_path(&argv, &mut at, "--in")?,
            "--dat" => dat = Some(take_path(&argv, &mut at, "--dat")?),
            "--ftl" => ftl = Some(take_path(&argv, &mut at, "--ftl")?),
            "--extract" => extract = Some(take_path(&argv, &mut at, "--extract")?),
            "--no-dat" => dat = None,
            "--no-ftl" => ftl = None,
            "--systems" => systems = true,
            "--loadout" => loadout = true,
            "--no-loadout" => loadout_pass = false,
            "--no-hue" => hue = false,
            other => {
                seed = Some(other.parse().map_err(|_| {
                    format!("{other:?} is neither a seed nor a flag, see --help")
                })?)
            }
        }
        at += 1;
    }

    let seed = match seed {
        Some(seed) => seed,
        None => SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos() as u64,
    };
    Ok(Args {
        seed,
        input,
        dat,
        ftl,
        extract,
        systems,
        loadout_pass,
        loadout,
        hue,
    })
}

/// The path a flag was given, which is the word after it.
fn take_path(argv: &[String], at: &mut usize, flag: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    *at += 1;
    argv.get(*at)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{flag} needs a path").into())
}

/// Write one file, making the directories above it.
fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::File::create(path)?;
    file.write_all(bytes)?;
    file.flush()
}

/// A byte count a person can read.
fn size(bytes: usize) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    match unit {
        0 => format!("{bytes} {}", UNITS[0]),
        _ => format!("{value:.1} {}", UNITS[unit]),
    }
}