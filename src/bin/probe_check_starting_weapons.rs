//! Checks `ALLOWED_STARTING_WEAPONS_NAMES` against the game data.
//!
//! Reads the `<weaponBlueprint>` elements of `blueprints.xml` and
//! `dlcBlueprints.xml`, builds a `StartingWeapon` from every blueprint the list
//! in `randomizer.rs` names, and asks `is_weapon_loadout_viable` whether the
//! weapon is good enough on its own - alone in the loadout, with no second
//! weapon to break the shields for it.
//!
//! Reports the names the data does not have, the weapons that pass, the weapons
//! that fail, and how the verdict moves when missile ammo drops below the
//! threshold in `randomizer.rs`. Exits non-zero when a name is missing or the
//! data holds a weapon the randomizer cannot model.
//!
//! `--with NAME` judges every weapon in a two weapon loadout with that blueprint
//! next to it, for example `--with ION_1`: an Ion Blast does no hull damage on
//! its own, yet its shots count as shield breaking for a partner.
//!
//! ```text
//! cargo run --bin probe_check_starting_weapons
//! cargo run --bin probe_check_starting_weapons -- --with ION_1
//! cargo run --bin probe_check_starting_weapons -- --missiles 40 --extra
//! cargo run --bin probe_check_starting_weapons -- --dat data/ftl.dat
//! ```

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use quick_xml::events::BytesStart;
use quick_xml::events::Event;
use quick_xml::reader::Reader;

use ftl_randomizer::randomizer::{
    ALLOWED_STARTING_WEAPONS_NAMES, MIN_EFFECTIVE_MISSILES, StartingWeapon,
    is_weapon_loadout_viable,
};
use ftl_randomizer::{FtlArchive, randomizer::WeaponType};

/// Files that define `<weaponBlueprint>` elements, in load order. The overwrite
/// files change ships and events, never weapons.
const BLUEPRINT_FILES: [&str; 2] = ["blueprints.xml", "dlcBlueprints.xml"];

/// Default location of the extracted data.
const DEFAULT_DATA_DIR: &str = "extracted/data";

// ---------------------------------------------------------------------------
// Arguments
// ---------------------------------------------------------------------------

struct Args {
    dir: PathBuf,
    dat: Option<PathBuf>,
    /// Missile supply the verdict is computed for.
    missiles: i8,
    /// Blueprint name of a second weapon every row is judged together with.
    with: Option<String>,
    /// Also list the weapon blueprints the allowed list does not name.
    extra: bool,
    help: bool,
}

impl Args {
    fn parse<I: Iterator<Item = String>>(mut argv: I) -> Result<Self, String> {
        let mut args = Args {
            dir: PathBuf::from(DEFAULT_DATA_DIR),
            dat: None,
            missiles: MIN_EFFECTIVE_MISSILES,
            with: None,
            extra: false,
            help: false,
        };
        while let Some(flag) = argv.next() {
            let mut value = |what: &str| argv.next().ok_or_else(|| format!("{what} needs a value"));
            match flag.as_str() {
                "--dir" => args.dir = PathBuf::from(value("--dir")?),
                "--dat" => args.dat = Some(PathBuf::from(value("--dat")?)),
                "--missiles" => {
                    let text = value("--missiles")?;
                    args.missiles = text
                        .parse()
                        .map_err(|_| format!("--missiles: {text} is not a number"))?;
                }
                "--with" => args.with = Some(value("--with")?),
                "--extra" => args.extra = true,
                "-h" | "--help" => args.help = true,
                other => return Err(format!("unknown option {other}")),
            }
        }
        Ok(args)
    }
}

fn usage() {
    println!(
        "usage: check_starting_weapons [OPTIONS]

  --dir PATH       extracted data directory (default {DEFAULT_DATA_DIR})
  --dat PATH       read the xml out of an ftl.dat instead, ignores --dir
  --missiles N     missile supply to judge the weapons by (default \
                   {MIN_EFFECTIVE_MISSILES}, the threshold in randomizer.rs)
  --with NAME      judge each weapon in a loadout with this second weapon
  --extra          list the weapon blueprints the allowed list does not name
  -h, --help       show this text"
    );
}

// ---------------------------------------------------------------------------
// Where the xml comes from
// ---------------------------------------------------------------------------

/// A read-only view of the game data: loose files, or an `ftl.dat` that borrows
/// the bytes read from disk.
enum Data<'a> {
    Dir(PathBuf),
    Archive(FtlArchive<'a>),
}

impl Data<'_> {
    /// Bytes of `name`, or `None` when the source has no such file.
    fn read(&self, name: &str) -> io::Result<Option<Vec<u8>>> {
        match self {
            Data::Dir(dir) => match fs::read(dir.join(name)) {
                Ok(bytes) => Ok(Some(bytes)),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error),
            },
            Data::Archive(archive) => Ok(archive
                .get_file(&format!("data/{name}"))
                .map(|bytes| bytes.to_vec())),
        }
    }

    /// Names of the files that start with `prefix` and end in `.xml`, sorted.
    fn names(&self, prefix: &str) -> io::Result<Vec<String>> {
        let mut names: Vec<String> = match self {
            Data::Dir(dir) => fs::read_dir(dir)?
                .filter_map(|entry| entry.ok())
                .filter_map(|entry| entry.file_name().into_string().ok())
                .filter(|name| name.starts_with(prefix) && name.ends_with(".xml"))
                .collect(),
            Data::Archive(archive) => archive
                .paths()
                .filter_map(|path| path.strip_prefix("data/"))
                .filter(|name| name.starts_with(prefix) && name.ends_with(".xml"))
                .map(str::to_string)
                .collect(),
        };
        names.sort();
        Ok(names)
    }

    fn describe(&self) -> String {
        match self {
            Data::Dir(dir) => format!("{} (extracted files)", dir.display()),
            Data::Archive(_) => "ftl.dat".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// English text
// ---------------------------------------------------------------------------

/// The English strings of the game, keyed by the `name` attribute.
///
/// Blueprint text is a pointer: `<title id="weapon_BEAM_1_title"/>`. The English
/// files are `text_*.xml`; the translated ones are `text-<lang>.xml` and do not
/// match the prefix.
fn load_texts(data: &Data) -> io::Result<HashMap<String, String>> {
    let mut texts = HashMap::new();
    for file in data.names("text_")? {
        let Some(bytes) = data.read(&file)? else {
            continue;
        };
        let mut reader = Reader::from_reader(bytes.as_slice());
        reader.config_mut().trim_text(true);

        let mut buf = Vec::new();
        let mut key: Option<String> = None;
        let mut value = String::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(start)) if start.local_name().as_ref() == b"text" => {
                    key = attribute(&start, "name");
                    value.clear();
                }
                Ok(Event::Text(text)) if key.is_some() => {
                    value.push_str(&text.decode().map_err(io::Error::other)?);
                }
                Ok(Event::End(end)) if end.local_name().as_ref() == b"text" => {
                    if let Some(name) = key.take() {
                        texts.entry(name).or_insert_with(|| value.clone());
                    }
                }
                Ok(Event::Eof) => break,
                Err(error) => return Err(io::Error::other(format!("{file}: {error}"))),
                _ => {}
            }
            buf.clear();
        }
    }
    Ok(texts)
}

/// Value of an attribute of a start tag, owned.
fn attribute(start: &BytesStart<'_>, key: &str) -> Option<String> {
    start
        .attributes()
        .flatten()
        .find(|item| item.key.local_name().as_ref() == key.as_bytes())
        .map(|item| String::from_utf8_lossy(item.value.as_ref()).into_owned())
}

// ---------------------------------------------------------------------------
// Weapon blueprints
// ---------------------------------------------------------------------------

/// One `<weaponBlueprint>`, kept down to what `StartingWeapon` needs.
#[derive(Clone)]
struct Blueprint {
    name: String,
    /// Blueprint file it was read from.
    source: String,
    /// English name, the blueprint name when the text files have nothing.
    title: String,
    /// `<type>` as written in the data.
    wtype: String,
    /// Values of the fields below, `None` where the blueprint has no such
    /// element. `StartingWeapon` takes them as `0`, `1` for shots and charges.
    power: Option<i8>,
    sp: Option<i8>,
    damage: Option<i8>,
    shots: Option<i8>,
    ion: Option<i8>,
    missiles: Option<i8>,
    charge_levels: Option<i8>,
}

impl Blueprint {
    fn new(name: String, source: &str) -> Self {
        Blueprint {
            name,
            source: source.to_string(),
            title: String::new(),
            wtype: String::new(),
            power: None,
            sp: None,
            damage: None,
            shots: None,
            ion: None,
            missiles: None,
            charge_levels: None,
        }
    }

    /// The weapon `randomizer.rs` reasons about, or `None` when its `<type>` is
    /// not one of the five the randomizer has a variant for. No weapon in the
    /// stock data hits that path, so a future type is reported rather than
    /// guessed at.
    fn starting_weapon(&self) -> Option<StartingWeapon> {
        let weapon_type = match self.wtype.as_str() {
            "BEAM" => WeaponType::BEAM,
            "BOMB" => WeaponType::BOMB,
            "BURST" => WeaponType::BURST,
            "LASER" => WeaponType::LASER,
            "MISSILES" => WeaponType::MISSILES,
            _ => return None,
        };
        let number = |value: Option<i8>, default| value.unwrap_or(default);
        Some(StartingWeapon {
            // The probe runs once and exits, so a leaked name is a free way to
            // get the `&'static str` the struct asks for.
            name: self.name.clone().leak(),
            weapon_type,
            power: number(self.power, 0),
            sp: number(self.sp, 0),
            damage: number(self.damage, 0),
            shots: number(self.shots, 1),
            ion: number(self.ion, 0),
            missiles: number(self.missiles, 0),
            charge_levels: number(self.charge_levels, 1),
        })
    }

    /// Fields the randomizer reads that the blueprint does not carry. Beams have
    /// no `<shots>` and only charge weapons have `<chargeLevels>`, so those two
    /// are expected; the rest would mean the data is not what we think it is.
    fn surprises(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if self.wtype.is_empty() {
            missing.push("type");
        }
        for (field, value) in [
            ("power", self.power),
            ("sp", self.sp),
            ("damage", self.damage),
        ] {
            if value.is_none() {
                missing.push(field);
            }
        }
        missing
    }
}

/// A direct child element of `<weaponBlueprint>`, with its text collected.
struct Child {
    tag: String,
    /// `id` attribute: a pointer into the text files.
    id: Option<String>,
    text: String,
}

/// Read every `<weaponBlueprint>` of one file.
///
/// `<weaponBlueprint>` is used two ways in the data: as a definition, a direct
/// child of the root `<FTL>`, and as a reference inside a `<droneBlueprint>`,
/// which holds the weapon name as element text and has no `name` attribute.
/// Only the definitions are read, and only their direct children, so the
/// `<type>` of a `<boost>` block cannot be taken for the weapon type. Comments
/// are skipped by the reader, which drops the blueprints the data keeps
/// commented out.
fn parse_blueprints(
    bytes: &[u8],
    source: &str,
    texts: &HashMap<String, String>,
) -> io::Result<Vec<Blueprint>> {
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);

    let mut found = Vec::new();
    let mut buf = Vec::new();
    let mut current: Option<Blueprint> = None;
    let mut child: Option<Child> = None;
    // Elements open right now: the root `<FTL>` makes it 1. `level` is the
    // nesting level of the element the event is about - 1 for the root, 2 for a
    // blueprint definition, 3 for a direct child of one. A self-closing tag
    // opens nothing, so its level is one above what is open.
    let mut open = 0usize;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(start)) => {
                open += 1;
                let tag = String::from_utf8_lossy(start.local_name().as_ref()).into_owned();
                if open == 2 && tag == "weaponBlueprint" {
                    let name = attribute(&start, "name").unwrap_or_default();
                    current = Some(Blueprint::new(name, source));
                } else if open == 3 && current.is_some() {
                    child = Some(Child {
                        tag,
                        id: attribute(&start, "id"),
                        text: String::new(),
                    });
                }
            }
            Ok(Event::Empty(start)) => {
                let level = open + 1;
                let tag = String::from_utf8_lossy(start.local_name().as_ref()).into_owned();
                if level == 2 && tag == "weaponBlueprint" {
                    // A definition with no children at all.
                    let name = attribute(&start, "name").unwrap_or_default();
                    current = Some(Blueprint::new(name, source));
                    found.push(finish(current.take().expect("just opened"), source)?);
                } else if level == 3 && current.is_some() {
                    let empty = Child {
                        tag,
                        id: attribute(&start, "id"),
                        text: String::new(),
                    };
                    apply_child(&mut current, &empty, texts, source)?;
                }
            }
            Ok(Event::Text(text)) => {
                if let Some(open_child) = child.as_mut() {
                    open_child
                        .text
                        .push_str(&text.decode().map_err(io::Error::other)?);
                }
            }
            Ok(Event::End(end)) => {
                let tag = String::from_utf8_lossy(end.local_name().as_ref()).into_owned();
                if open == 2 && tag == "weaponBlueprint" && current.is_some() {
                    found.push(finish(current.take().expect("just opened"), source)?);
                } else if open == 3 && current.is_some() {
                    let closed = child.take().expect("inside a child element");
                    apply_child(&mut current, &closed, texts, source)?;
                }
                open -= 1;
            }
            Ok(Event::Eof) => break,
            Err(error) => return Err(io::Error::other(format!("{source}: {error}"))),
            _ => {}
        }
        buf.clear();
    }

    Ok(found)
}

/// A blueprint that has been read to its end: the title falls back to the name.
fn finish(blueprint: Blueprint, source: &str) -> io::Result<Blueprint> {
    if blueprint.name.is_empty() {
        return Err(io::Error::other(format!(
            "{source}: a <weaponBlueprint> definition has no name attribute"
        )));
    }
    let title = if blueprint.title.is_empty() {
        blueprint.name.clone()
    } else {
        blueprint.title
    };
    Ok(Blueprint { title, ..blueprint })
}

/// Copy one direct child element into the blueprint being read.
fn apply_child(
    current: &mut Option<Blueprint>,
    child: &Child,
    texts: &HashMap<String, String>,
    source: &str,
) -> io::Result<()> {
    let blueprint = current.as_mut().expect("inside a blueprint");
    let number = |tag: &str| -> io::Result<i8> {
        child.text.trim().parse().map_err(|_| {
            io::Error::other(format!(
                "{}: {} has <{tag}>{}</{tag}>, not a whole number",
                source, blueprint.name, child.text
            ))
        })
    };
    match child.tag.as_str() {
        "type" => blueprint.wtype = child.text.trim().to_string(),
        // `<title id=".."/>` points at the text files, `<title>text</title>` is
        // the literal text of a NOLOC blueprint.
        "title" => {
            blueprint.title = match &child.id {
                Some(id) => texts
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| blueprint.name.clone()),
                None if !child.text.trim().is_empty() => child.text.trim().to_string(),
                None => blueprint.name.clone(),
            };
        }
        "power" => blueprint.power = Some(number("power")?),
        "sp" => blueprint.sp = Some(number("sp")?),
        "damage" => blueprint.damage = Some(number("damage")?),
        "shots" => blueprint.shots = Some(number("shots")?),
        "ion" => blueprint.ion = Some(number("ion")?),
        "missiles" => blueprint.missiles = Some(number("missiles")?),
        "chargeLevels" => blueprint.charge_levels = Some(number("chargeLevels")?),
        _ => {}
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

/// One name of the allowed list, the blueprint behind it, and the verdict.
struct Row {
    name: &'static str,
    blueprint: Option<Blueprint>,
    /// Good enough alone with `--missiles`, with the type not modelled being
    /// `None` as well.
    verdict: Option<bool>,
    /// Same with one missile less.
    tight: Option<bool>,
    /// Why there is no verdict.
    problem: Option<String>,
}

fn header() {
    println!(
        "{:<24} {:<27} {:<8} {:>3} {:>2} {:>3} {:>5} {:>3} {:>4} {:>3}  {:>5} {:>5}",
        "NAME", "TITLE", "TYPE", "PWR", "SP", "DMG", "SHOTS", "ION", "MSIL", "CHG", "GOOD", "TIGHT",
    );
}

/// One row of the report. `GOOD` is the verdict at `--missiles`, `TIGHT` the
/// verdict with one missile less.
fn print_row(row: &Row) {
    let yes = |value: Option<bool>| match value {
        Some(true) => "yes",
        Some(false) => "no",
        None => "-",
    };
    match &row.blueprint {
        Some(b) => println!(
            "{:<24} {:<27} {:<8} {:>3} {:>2} {:>3} {:>5} {:>3} {:>4} {:>3}  {:>5} {:>5}",
            row.name,
            b.title,
            b.wtype,
            b.power.unwrap_or_default(),
            b.sp.unwrap_or_default(),
            b.damage.unwrap_or_default(),
            b.shots.unwrap_or(1),
            b.ion.unwrap_or_default(),
            b.missiles.unwrap_or_default(),
            b.charge_levels.unwrap_or(1),
            yes(row.verdict),
            yes(row.tight),
        ),
        None => println!(
            "{:<24} {:<27}",
            row.name,
            row.problem.clone().unwrap_or_default()
        ),
    }
}

fn list(title: &str, rows: &[&Row]) {
    println!("\n== {title} ({})", rows.len());
    if rows.is_empty() {
        println!("  none");
        return;
    }
    header();
    for entry in rows {
        print_row(entry);
    }
}

fn run(args: &Args) -> io::Result<bool> {
    let raw = match &args.dat {
        Some(path) => Some(fs::read(path)?),
        None => None,
    };
    let data = match &raw {
        Some(bytes) => {
            let archive = FtlArchive::from_bytes(bytes)
                .map_err(|error| io::Error::other(error.to_string()))?;
            Data::Archive(archive)
        }
        None => Data::Dir(args.dir.clone()),
    };

    let texts = load_texts(&data)?;
    let mut blueprints: Vec<Blueprint> = Vec::new();
    let mut counts = Vec::new();
    for file in BLUEPRINT_FILES {
        let bytes = data
            .read(file)?
            .ok_or_else(|| io::Error::other(format!("{} has no {file}", data.describe())))?;
        let parsed = parse_blueprints(&bytes, file, &texts)?;
        counts.push(format!("{file}: {}", parsed.len()));
        blueprints.extend(parsed);
    }

    // Index by blueprint name. A name defined twice would make the verdict
    // depend on file order, so it is reported as a problem.
    let mut index: HashMap<&str, &Blueprint> = HashMap::new();
    let mut duplicated: Vec<&str> = Vec::new();
    for blueprint in &blueprints {
        if index.insert(blueprint.name.as_str(), blueprint).is_some() {
            duplicated.push(blueprint.name.as_str());
        }
    }

    // The second weapon of every loadout, resolved once.
    let partner: Option<StartingWeapon> = match &args.with {
        None => None,
        Some(name) => {
            let blueprint = index.get(name.as_str()).ok_or_else(|| {
                io::Error::other(format!("--with {name}: no such weapon in the data"))
            })?;
            Some(blueprint.starting_weapon().ok_or_else(|| {
                io::Error::other(format!(
                    "--with {name}: unmodelled type <{}>",
                    blueprint.wtype
                ))
            })?)
        }
    };

    let rows: Vec<Row> = ALLOWED_STARTING_WEAPONS_NAMES
        .iter()
        .map(|name| {
            let blueprint = index.get(name).map(|found| (*found).clone());
            let mut row = Row {
                name,
                blueprint,
                verdict: None,
                tight: None,
                problem: None,
            };
            match &row.blueprint {
                None => row.problem = Some(format!("not in {}", BLUEPRINT_FILES.join(" / "))),
                Some(blueprint) => match blueprint.starting_weapon() {
                    None => row.problem = Some(format!("unmodelled type <{}>", blueprint.wtype)),
                    Some(weapon) => {
                        // The loadout the weapon is judged in: alone, or with
                        // the weapon `--with` named next to it.
                        let loadout: Vec<StartingWeapon> = match &partner {
                            Some(second) => vec![weapon, *second],
                            None => vec![weapon],
                        };
                        row.verdict = Some(is_weapon_loadout_viable(&loadout, args.missiles));
                        row.tight = Some(is_weapon_loadout_viable(&loadout, args.missiles - 1));
                    }
                },
            }
            row
        })
        .collect();

    let mut problems: Vec<String> = Vec::new();
    if !duplicated.is_empty() {
        problems.push(format!(
            "defined twice in the data: {}",
            duplicated
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for row in &rows {
        if let Some(problem) = &row.problem {
            problems.push(format!("{}: {problem}", row.name));
        }
        if let Some(blueprint) = &row.blueprint {
            let missing = blueprint.surprises();
            if !missing.is_empty() {
                problems.push(format!(
                    "{}: no <{}> in the data",
                    blueprint.name,
                    missing.join(">, <")
                ));
            }
        }
    }

    let good: Vec<&Row> = rows
        .iter()
        .filter(|row| row.verdict == Some(true))
        .collect();
    let poor: Vec<&Row> = rows
        .iter()
        .filter(|row| row.problem.is_none() && row.verdict != Some(true))
        .collect();
    let movers: Vec<&Row> = rows
        .iter()
        .filter(|row| row.verdict.is_some() && row.verdict != row.tight)
        .collect();

    println!("data      {}", data.describe());
    println!("texts     {} english strings", texts.len());
    println!(
        "weapons   {} blueprints ({})",
        blueprints.len(),
        counts.join(", ")
    );
    println!(
        "allowed   {} names, {} read as weapons, {} missing or unmodelled",
        ALLOWED_STARTING_WEAPONS_NAMES.len(),
        rows.iter().filter(|row| row.verdict.is_some()).count(),
        rows.iter().filter(|row| row.problem.is_some()).count(),
    );
    println!(
        "ammo      {} missiles for GOOD, {} for TIGHT (the threshold in randomizer.rs is {MIN_EFFECTIVE_MISSILES})",
        args.missiles,
        args.missiles - 1,
    );
    println!(
        "verdict   GOOD = is_weapon_loadout_viable on {}",
        match &partner {
            None => "the weapon alone".to_string(),
            Some(second) => format!("the weapon together with {}", second.name),
        }
    );
    println!("          TIGHT = the same test at the lower ammo count");
    println!("problems  {}", problems.len());

    let (good_title, poor_title) = match &args.with {
        None => (
            "GOOD ENOUGH ON ITS OWN".to_string(),
            "NOT GOOD ENOUGH ON ITS OWN".to_string(),
        ),
        Some(name) => (
            format!("GOOD ENOUGH TOGETHER WITH {name}"),
            format!("NOT GOOD ENOUGH TOGETHER WITH {name}"),
        ),
    };
    list(&good_title, &good);
    list(&poor_title, &poor);
    list("VERDICT DEPENDS ON MISSILE AMMO", &movers);

    if args.extra {
        let extra: Vec<&Blueprint> = blueprints
            .iter()
            .filter(|blueprint| !ALLOWED_STARTING_WEAPONS_NAMES.contains(&blueprint.name.as_str()))
            .collect();
        println!("\n== NOT IN THE ALLOWED LIST ({})", extra.len());
        println!("{:<24} {:<27} {:<8} SOURCE", "NAME", "TITLE", "TYPE");
        for blueprint in extra {
            println!(
                "{:<24} {:<27} {:<8} {}",
                blueprint.name, blueprint.title, blueprint.wtype, blueprint.source
            );
        }
    }

    println!("\n== PROBLEMS ({})", problems.len());
    for problem in &problems {
        println!("  {problem}");
    }

    Ok(problems.is_empty())
}

fn main() -> ExitCode {
    let args = match Args::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("error: {message}\n");
            usage();
            return ExitCode::FAILURE;
        }
    };
    if args.help {
        usage();
        return ExitCode::SUCCESS;
    }
    match run(&args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => {
            eprintln!("\nthe allowed list does not match the data");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
