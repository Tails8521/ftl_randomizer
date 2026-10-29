//! Checks the enemy gun list of a run: how many `<weaponList>`s were pointed at it,
//! that no `<weaponList>` still draws from a list of its own, that the list is filed
//! once, holds every gun of the pool and only guns the game defines, and that the
//! pass run again over its own output changes nothing.

use std::collections::BTreeSet;
use std::fs;

use ftl_randomizer::blueprints::{self, Element};
use ftl_randomizer::enemies::{self, Report};
use ftl_randomizer::randomizer::ALLOWED_ENEMY_WEAPONS_NAMES;
use ftl_randomizer::FtlArchive;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = fs::read("data/ftl.dat")?;
    let mut archive = FtlArchive::from_bytes(&bytes)?;

    let first = enemies::randomize(&mut archive)?;
    say("first run", &first);
    let second = enemies::randomize(&mut archive)?;
    say("second run", &second);

    let mut problems: Vec<String> = Vec::new();
    if second.pointed != 0 || !second.changed.is_empty() {
        problems.push("the second run changed something".to_string());
    }

    // What the ships draw from now, and what the files hold of the list.
    let mut held: BTreeSet<String> = BTreeSet::new();
    let mut defined: BTreeSet<String> = BTreeSet::new();
    let mut filed = 0usize;
    let mut guns: Vec<String> = Vec::new();
    for (path, payload) in archive.files() {
        if !path.starts_with("data/") || !path.ends_with(".xml") {
            continue;
        }
        let text = String::from_utf8(payload.to_vec())?;
        if !text.contains("<weaponList") && !text.contains("<weaponBlueprint") {
            continue;
        }
        let inside = blueprints::elements(&text, 0)?;
        for element in inside.iter().filter(|e| e.name == "weaponList") {
            if let Some((_, value)) = blueprints::attribute(&text[element.tag.clone()], "load") {
                held.insert(format!("{path} {value}"));
            }
        }
        for element in inside.iter().filter(|e| e.name == "weaponBlueprint") {
            if let Some((_, value)) = blueprints::attribute(&text[element.tag.clone()], "name") {
                defined.insert(value.to_string());
            }
        }
        for list in inside.iter().filter(|e| e.name == "blueprintList") {
            if blueprints::attribute(&text[list.tag.clone()], "name").map(|(_, v)| v)
                == Some(enemies::LIST)
            {
                filed += 1;
                guns = names(&text, &inside, list);
            }
        }
    }
    let stray: Vec<&String> = held.iter().filter(|row| !row.ends_with(enemies::LIST)).collect();
    if !stray.is_empty() {
        problems.push(format!(
            "{} <weaponList>s still draw from another list: {stray:?}",
            stray.len()
        ));
    }
    if filed != 1 {
        problems.push(format!("{} is filed {filed} times, not once", enemies::LIST));
    }
    if guns.iter().collect::<BTreeSet<_>>().len() != guns.len() {
        problems.push("the list names one gun twice".to_string());
    }
    for missing in guns.iter().filter(|gun| !defined.contains(*gun)) {
        problems.push(format!("{missing} is in the list and is no <weaponBlueprint>"));
    }
    for want in ALLOWED_ENEMY_WEAPONS_NAMES.iter().filter(|gun| !guns.contains(&gun.to_string())) {
        problems.push(format!("{want} is allowed for the enemies and not in the list"));
    }

    if problems.is_empty() {
        println!(
            "\nok: {} guns, {} <weaponList>s pointed, filed once, idempotent",
            guns.len(),
            first.pointed
        );
        Ok(())
    } else {
        for problem in &problems {
            println!("problem: {problem}");
        }
        Err(format!("{} problems", problems.len()).into())
    }
}

/// The `<name>` children of one `<blueprintList>`, in the order the file holds them.
fn names(text: &str, inside: &[Element], list: &Element) -> Vec<String> {
    inside
        .iter()
        .filter(|element| {
            element.name == "name"
                && element.depth == list.depth + 1
                && element.span.start > list.span.start
                && element.span.end < list.span.end
        })
        .filter_map(|element| {
            // A `<name/>` that closes itself holds nothing, and is not a gun.
            let body = text[element.tag.end..element.span.end].strip_suffix("</name>")?;
            let held = body.trim();
            (!held.is_empty()).then(|| held.to_string())
        })
        .collect()
}

/// One run of the pass, as it reads.
fn say(when: &str, report: &Report) {
    println!(
        "{when}: {} <weaponList>s pointed, {} guns, {} files changed: {}",
        report.pointed,
        report.entries,
        report.changed.len(),
        report.changed.join(", ")
    );
    for warning in &report.warnings {
        println!("  warning: {warning}");
    }
}