//! List the paths whose payload differs between two archives:
//! probe_diff_dat <a.dat> <b.dat>

use ftl_randomizer::FtlArchive;
use std::io::Read;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [a, b] = [args[0].as_str(), args[1].as_str()] else {
        return Err("usage: probe_diff_dat <a.dat> <b.dat>".into());
    };
    let mut one = Vec::new();
    let mut two = Vec::new();
    std::fs::File::open(a)?.read_to_end(&mut one)?;
    std::fs::File::open(b)?.read_to_end(&mut two)?;
    let one = FtlArchive::from_bytes(&one)?;
    let two = FtlArchive::from_bytes(&two)?;
    let changed = one.diff(&two);
    for path in &changed {
        println!("{path}");
    }
    println!(
        "{} of {} files differ",
        changed.len(),
        one.len().max(two.len())
    );
    Ok(())
}