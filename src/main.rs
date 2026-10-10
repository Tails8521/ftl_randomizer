//! Loads `data/ftl.dat`, extracts it, and writes it back out again.
//!
//! This binary owns all filesystem access. `FtlArchive` only ever sees bytes.

use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::ExitCode;

use ftl_randomizer::FtlArchive;

const INPUT: &str = "data/ftl.dat";
const EXTRACT_TO: &str = "extracted";
const OUTPUT: &str = "target/ftl.repack.dat";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> io::Result<()> {
    let bytes = fs::read(INPUT)?;
    println!("read {} bytes from {INPUT}", bytes.len());

    let archive = FtlArchive::from_bytes(&bytes).map_err(io::Error::other)?;
    println!(
        "{} files, {} trailing byte(s) after the last payload",
        archive.len(),
        archive.trailing_bytes()
    );

    extract_all(&archive, Path::new(EXTRACT_TO))?;
    println!("extracted every file to {EXTRACT_TO}");

    let repacked = archive.to_bytes().map_err(io::Error::other)?;
    println!(
        "repacked: {} bytes, byte for byte equal to the input: {}",
        repacked.len(),
        repacked == bytes
    );
    write(Path::new(OUTPUT), &repacked)?;
    println!("wrote the repacked archive to {OUTPUT}");
    Ok(())
}

/// Write every file of the archive under `dest`, making directories as needed.
fn extract_all(archive: &FtlArchive<'_>, dest: &Path) -> io::Result<()> {
    for (path, bytes) in archive.files() {
        let target = dest.join(path);
        write(&target, bytes)?;
    }
    Ok(())
}

/// Write one file, creating its parent directories.
fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::File::create(path)?;
    file.write_all(bytes)?;
    file.flush()
}
