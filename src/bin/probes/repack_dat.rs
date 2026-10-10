//! Read a `ftl.dat`, optionally replace payloads from a directory tree, write a
//! new `ftl.dat`, and report whether the result matches the input.
//!
//! All filesystem access lives here. `FtlArchive` works on bytes only.
//!
//! Run with no arguments to round trip `data/ftl.dat` into
//! `target/ftl.repack.dat` and compare it with the original.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::OnceLock;
use std::time::Instant;

use ftl_randomizer::FtlArchive;

const USAGE: &str = "\
repack_dat - read, edit and rewrite an FTL ftl.dat archive

USAGE:
  repack_dat [OPTIONS] [INPUT] [OUTPUT]

  INPUT   archive to read    (default: data/ftl.dat)
  OUTPUT  archive to write   (default: target/ftl.repack.dat)

OPTIONS:
  --dir <PATH>   replace payloads with the files found under PATH, which must
                 hold archive paths (for example data/kestrel.xml). Files that
                 match the archive byte for byte are skipped, files whose path
                 is not in the archive are reported and ignored.
  --check-only   do everything except write OUTPUT
  -h, --help     this text

With no --dir nothing is edited, so a correct repack must be byte for byte
identical to the input: same length, same per-file checksums, same whole-file
crc32.
";

struct Args {
    input: PathBuf,
    output: PathBuf,
    dir: Option<PathBuf>,
    check_only: bool,
}

fn parse_args(argv: Vec<String>) -> Result<Args, String> {
    let mut positional: Vec<PathBuf> = Vec::new();
    let mut dir = None;
    let mut check_only = false;
    let mut argv = argv.into_iter().skip(1).peekable();

    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "-h" | "--help" => return Err(String::new()),
            "--dir" => dir = Some(PathBuf::from(argv.next().ok_or("--dir needs a value")?)),
            "--check-only" => check_only = true,
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            other => positional.push(PathBuf::from(other)),
        }
    }
    if positional.len() > 2 {
        return Err("too many positional arguments".into());
    }
    Ok(Args {
        input: positional
            .first()
            .cloned()
            .unwrap_or_else(|| PathBuf::from("data/ftl.dat")),
        output: positional
            .get(1)
            .cloned()
            .unwrap_or_else(|| PathBuf::from("target/ftl.repack.dat")),
        dir,
        check_only,
    })
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().collect()) {
        Ok(args) => args,
        Err(message) => {
            if !message.is_empty() {
                eprintln!("error: {message}\n");
            }
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    match run(args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(1)
        }
    }
}

fn run(args: Args) -> io::Result<bool> {
    let started = Instant::now();

    let bytes = fs::read(&args.input)?;
    println!(
        "input:   {} ({} bytes, crc32 {:08x})",
        args.input.display(),
        bytes.len(),
        crc32(&bytes)
    );
    let mut archive = FtlArchive::from_bytes(&bytes).map_err(io::Error::other)?;
    println!(
        "         {} files, path pool and index rebuilt on write, {} trailing byte(s)",
        archive.len(),
        archive.trailing_bytes()
    );

    let edited = match &args.dir {
        Some(dir) => {
            let (edited, strays) = apply_dir(&mut archive, dir)?;
            println!(
                "payloads: {} of {} files replaced from {}",
                edited.len(),
                archive.len(),
                dir.display()
            );
            for path in edited.iter().take(10) {
                println!("  {path}");
            }
            if !strays.is_empty() {
                println!(
                    "warning: {} file(s) under {} are not in the archive and were ignored",
                    strays.len(),
                    dir.display()
                );
                for stray in strays.iter().take(10) {
                    println!("  {stray}");
                }
            }
            edited
        }
        None => {
            println!("payloads: unchanged, read straight out of the input archive");
            Vec::new()
        }
    };

    println!("layout:  rebuilt from scratch (sorted path pool, 64 byte aligned payloads)");
    let out = archive.to_bytes().map_err(io::Error::other)?;
    println!(
        "output:  {} bytes, crc32 {:08x} ({} ms to build)",
        out.len(),
        crc32(&out),
        started.elapsed().as_millis()
    );

    // With nothing edited the result has to be the same file, not merely the
    // same set of files.
    let identical = out == bytes;
    if edited.is_empty() {
        println!(
            "byte identical to input: {}",
            if identical { "YES" } else { "NO" }
        );
        if !identical {
            match out.iter().zip(bytes.iter()).position(|(a, b)| a != b) {
                Some(at) => println!(
                    "         first difference at byte {at}: wrote {:#04x}, input had {:#04x}",
                    out[at], bytes[at]
                ),
                None => println!(
                    "         contents match, lengths differ ({} vs {})",
                    out.len(),
                    bytes.len()
                ),
            }
        }
    } else {
        println!("{} file(s) intentionally replaced", edited.len());
    }

    // Read the archive back through the parser: one we cannot read is broken no
    // matter what its bytes look like. When a file was written, read that file
    // back - that checks the write path as well.
    let read_back = if args.check_only {
        out
    } else {
        if let Some(parent) = args.output.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&args.output, &out)?;
        println!("wrote:   {}", args.output.display());
        drop(out);
        fs::read(&args.output)?
    };
    let repacked = FtlArchive::from_bytes(&read_back).map_err(io::Error::other)?;
    // The archive now says what the output should hold: the payloads that were
    // replaced, and for every other file the original bytes it still borrows.
    // Anything this reports is the writer having corrupted something.
    let problems = archive.diff(&repacked);
    if problems.is_empty() {
        println!(
            "per-file check: all {} payloads read back correctly through the index",
            repacked.len()
        );
    } else {
        println!("per-file check: {} problem(s)", problems.len());
        for problem in problems.iter().take(20) {
            println!("  {problem}");
        }
    }

    let ok = if edited.is_empty() {
        identical && problems.is_empty()
    } else {
        problems.is_empty()
    };
    println!(
        "{} in {} ms",
        if ok { "OK" } else { "MISMATCH" },
        started.elapsed().as_millis()
    );
    Ok(ok)
}

/// Replace archive payloads with the files under `dir`. Returns the paths that
/// were replaced, and the paths that exist under `dir` but not in the archive -
/// usually a typo, which would otherwise do nothing at all.
fn apply_dir(archive: &mut FtlArchive<'_>, dir: &Path) -> io::Result<(Vec<String>, Vec<String>)> {
    let mut edited = Vec::new();
    let mut strays = Vec::new();
    let mut stack = vec![dir.to_path_buf()];

    while let Some(current) = stack.pop() {
        for item in fs::read_dir(&current)? {
            let path = item?.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Ok(relative) = path.strip_prefix(dir) else {
                continue;
            };
            let relative = relative
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");

            let bytes = fs::read(&path)?;
            if archive.get_file(&relative).is_none() {
                strays.push(relative);
                continue;
            }
            // Unchanged files are not worth keeping a copy of.
            if archive
                .get_file(&relative)
                .is_some_and(|original| original != bytes)
            {
                archive.set_file(&relative, bytes);
                edited.push(relative);
            }
        }
    }

    edited.sort();
    strays.sort();
    Ok((edited, strays))
}

/// Plain table driven crc32, only used to print a digest of a whole archive so
/// two runs can be told apart. Not part of the archive format, which is why it
/// lives here and not in the library.
fn crc32(bytes: &[u8]) -> u32 {
    fn table() -> [u32; 256] {
        let mut table = [0u32; 256];
        for (index, slot) in table.iter_mut().enumerate() {
            let mut value = index as u32;
            for _ in 0..8 {
                value = if value & 1 != 0 {
                    0xEDB8_8320 ^ (value >> 1)
                } else {
                    value >> 1
                };
            }
            *slot = value;
        }
        table
    }
    static TABLE: OnceLock<[u32; 256]> = OnceLock::new();
    let table = TABLE.get_or_init(table);

    let mut crc = 0xFFFF_FFFFu32;
    for &byte in bytes {
        crc = table[((crc ^ u32::from(byte)) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}
