//! Inject downloaded `.mb64` levels into the game's virtual SD card.
//!
//! The game backs its "SD card" with a FAT16 image (`mb64_sd.img`) in its working
//! directory — which is the launcher's data dir — and reads custom levels from a
//! `/Mario Builder 64 Levels` directory, listing every file whose name ends in
//! `.mb64` (see `app/src/mb64_sdcard.cpp` and the decomp's `src/mb64/file.c`). We
//! open that same image with a host-side FAT driver and drop the downloaded bytes
//! in as `<name>.mb64`, so the level shows up in the in-game level browser the next
//! time the game launches.
//!
//! The game must not be running while we write (it holds the image open and would
//! clobber our changes), so callers gate installs on a stopped game.

use anyhow::{Context, Result};
use fatfs::{FatType, FileSystem, FormatVolumeOptions, FsOptions};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

/// Image file the game creates/uses, relative to the data dir.
const IMG_NAME: &str = "mb64_sd.img";
/// Directory the game scans for custom levels (long filename, with spaces).
const LEVELS_DIR: &str = "Mario Builder 64 Levels";
/// Size the game formats the card to (`mb64_sdcard.cpp`); used only if we have to
/// create the image ourselves (i.e. the game was never launched).
const IMG_BYTES: u64 = 32 * 1024 * 1024;
/// Max length of the on-card filename, in bytes, including the `.mb64` suffix. The
/// decomp skips any entry longer than `MAX_FILE_NAME_SIZE - 1` (= 40), so the stem
/// gets 40 - len(".mb64") = 35 bytes — the same cap the website uses.
const MAX_NAME_BYTES: usize = 40;
const SUFFIX: &str = ".mb64";

/// Turn a level title into a safe on-card filename, mirroring the website's rule:
/// normalize smart quotes, drop FAT-illegal and control characters, collapse
/// whitespace, then cap the stem so `<stem>.mb64` fits the game's filename limit.
pub fn level_filename(name: &str) -> String {
    let cleaned: String = name
        .replace(['\u{201c}', '\u{201d}'], "\"")
        .replace(['\u{2018}', '\u{2019}'], "'")
        .chars()
        .filter(|c| !matches!(c, '\\' | '/' | '?' | '%' | '*' | ':' | '|' | '"' | '<' | '>'))
        .filter(|c| !c.is_control())
        .collect();
    // Collapse internal whitespace runs and trim the ends.
    let mut stem = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");

    // Byte-truncate the stem so stem + ".mb64" fits, respecting char boundaries.
    let budget = MAX_NAME_BYTES - SUFFIX.len();
    if stem.len() > budget {
        let mut end = budget;
        while end > 0 && !stem.is_char_boundary(end) {
            end -= 1;
        }
        stem.truncate(end);
        stem = stem.trim_end().to_string();
    }
    if stem.is_empty() {
        stem = "untitled".to_string();
    }
    format!("{stem}{SUFFIX}")
}

/// Write `bytes` into the SD card image as `<level_name>.mb64`, creating the image
/// and the levels directory if they don't exist. Returns the filename used.
pub fn install(data_dir: &Path, level_name: &str, bytes: &[u8]) -> Result<String> {
    std::fs::create_dir_all(data_dir).ok();
    let img_path = data_dir.join(IMG_NAME);
    if !img_path.exists() {
        create_image(&img_path)
            .with_context(|| format!("creating SD card image at {}", img_path.display()))?;
    }

    let fname = level_filename(level_name);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&img_path)
        .with_context(|| format!("opening {}", img_path.display()))?;
    let fs = FileSystem::new(file, FsOptions::new())
        .context("reading the SD card image (mb64_sd.img)")?;
    {
        let root = fs.root_dir();
        let dir = match root.open_dir(LEVELS_DIR) {
            Ok(d) => d,
            Err(_) => root
                .create_dir(LEVELS_DIR)
                .context("creating the “Mario Builder 64 Levels” folder")?,
        };
        let mut f = dir.create_file(&fname).context("creating the level file")?;
        // create_file opens an existing file without truncating; clear it so a
        // re-download doesn't leave a stale tail from a larger previous version.
        f.truncate().context("truncating the level file")?;
        f.write_all(bytes).context("writing the level file")?;
        f.flush().context("flushing the level file")?;
    }
    fs.unmount().context("finalizing the SD card image")?;
    Ok(fname)
}

/// Names of `.mb64` files already on the card (used to mark levels as installed).
/// Best-effort: returns empty if the image or folder isn't there yet.
pub fn installed_filenames(data_dir: &Path) -> Vec<String> {
    let img_path = data_dir.join(IMG_NAME);
    if !img_path.exists() {
        return Vec::new();
    }
    let Ok(file) = OpenOptions::new().read(true).write(true).open(&img_path) else {
        return Vec::new();
    };
    let Ok(fs) = FileSystem::new(file, FsOptions::new()) else {
        return Vec::new();
    };
    let root = fs.root_dir();
    let Ok(dir) = root.open_dir(LEVELS_DIR) else {
        return Vec::new();
    };
    dir.iter()
        .flatten()
        .map(|e| e.file_name())
        .filter(|n| n.to_lowercase().ends_with(SUFFIX))
        .collect()
}

/// Create and FAT16-format a fresh 32 MiB card image (matches the game's geometry
/// closely enough for libcart's FatFS to mount it).
fn create_image(path: &Path) -> Result<()> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    file.set_len(IMG_BYTES)?;
    fatfs::format_volume(
        &mut file,
        FormatVolumeOptions::new().fat_type(FatType::Fat16),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Opt-in round-trip: inject real bytes into a caller-provided image dir using
    /// the exact production code, so the result can be cross-checked with the host
    /// FAT driver (a proxy for the game's libcart FatFS). Drive it with:
    ///   MB64_INJECT_DIR=<dir> MB64_INJECT_SRC=<file.mb64> \
    ///     cargo test -p mb64-launcher inject_real -- --ignored --nocapture
    #[test]
    #[ignore]
    fn inject_real() {
        let dir = std::env::var("MB64_INJECT_DIR").expect("set MB64_INJECT_DIR");
        let src = std::env::var("MB64_INJECT_SRC").expect("set MB64_INJECT_SRC");
        let dir = Path::new(&dir);
        let bytes = std::fs::read(&src).expect("read source bytes");
        let fname = install(dir, "Salem Web Test", &bytes).expect("install");
        let listed = installed_filenames(dir);
        assert!(listed.contains(&fname), "{fname:?} not in {listed:?}");
        eprintln!(
            "INJECTED {} ({} bytes) into {}",
            fname,
            bytes.len(),
            dir.join(IMG_NAME).display()
        );
    }

    #[test]
    fn filename_sanitizes_and_caps() {
        assert_eq!(level_filename("Bowser's Hell"), "Bowser's Hell.mb64");
        // Illegal characters are stripped.
        assert_eq!(level_filename("a/b:c*?"), "abc.mb64");
        // Empty / all-illegal falls back to a stable name.
        assert_eq!(level_filename("///"), "untitled.mb64");
        // Whitespace is collapsed and trimmed.
        assert_eq!(level_filename("  hi   there  "), "hi there.mb64");
        // Long names cap so the whole filename fits the game's limit (40 bytes).
        let long = "x".repeat(80);
        let f = level_filename(&long);
        assert!(f.len() <= MAX_NAME_BYTES, "{} too long", f);
        assert!(f.ends_with(SUFFIX));
    }
}
