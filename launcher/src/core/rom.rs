//! ROM verification + provisioning.
//!
//! This mirrors `recomp::select_rom` in librecomp (recomp.cpp:348-398) so the
//! launcher accepts exactly the ROMs the game accepts:
//!   1. read the file bytes,
//!   2. pad up to a multiple of 4 bytes with zeros,
//!   3. detect/normalize byteswapping from the first 4 bytes,
//!   4. `XXH3_64bits(data, len)` and compare to the expected hash.
//!
//! The game only needs the built **Mario Builder 64** ROM (`mb64.z64`) at runtime,
//! NOT the base US SM64 ROM. The expected hash is the one registered in
//! `mb64_main.cpp` (`GameEntry.rom_hash`).

use std::path::{Path, PathBuf};
use xxhash_rust::xxh3::xxh3_64;

/// `GameEntry.rom_hash` from app/src/mb64_main.cpp — XXH3_64bits of the
/// (4-byte-padded, byteswap-normalized) built US `mb64.z64`.
pub const EXPECTED_ROM_HASH: u64 = 0xd82b_295c_5a4d_30f5;

/// A correctly-ordered (`.z64`, big-endian) N64 ROM begins with these 4 bytes.
const FIRST_ROM_BYTES: [u8; 4] = [0x80, 0x37, 0x12, 0x40];

/// The name the launcher drops the verified ROM under, in the data dir, for the
/// game to provision on first launch (mb64_main.cpp reads `mb64.z64` from cwd).
pub const SOURCE_ROM_NAME: &str = "mb64.z64";

/// The name librecomp stores the provisioned ROM under (game_id + ".z64"); once
/// this exists and validates, the game no longer reads `mb64.z64`.
pub const PROVISIONED_ROM_NAME: &str = "mb64.us.z64";

/// Result of checking a candidate ROM file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RomCheck {
    /// Hash matches — this is the right ROM.
    Good,
    /// File doesn't start with a recognizable N64 ROM header.
    NotARom,
    /// A valid N64 ROM, but not Mario Builder 64 (or the wrong build/version).
    WrongRom,
    /// Couldn't read the file.
    Unreadable(String),
}

/// In-place byteswap of `data` in 4-byte groups, swizzling each byte's index by
/// `index_xor` (mirrors `byteswap_data` in recomp.cpp).
fn byteswap(data: &mut [u8], index_xor: usize) {
    let mut pos = 0;
    while pos + 4 <= data.len() {
        let t = [data[pos], data[pos + 1], data[pos + 2], data[pos + 3]];
        for (i, b) in t.iter().enumerate() {
            data[pos + (i ^ index_xor)] = *b;
        }
        pos += 4;
    }
}

/// Normalize a padded ROM into canonical (non-byteswapped) order. Returns `false`
/// if the header isn't recognizable as any N64 ROM ordering.
fn normalize_byteorder(data: &mut [u8]) -> bool {
    if data.len() < 4 {
        return false;
    }
    let f = &FIRST_ROM_BYTES;
    let matches = |a: usize, b: usize, c: usize, d: usize| {
        data[0] == f[a] && data[1] == f[b] && data[2] == f[c] && data[3] == f[d]
    };
    if matches(0, 1, 2, 3) {
        // NotByteswapped
        true
    } else if matches(3, 2, 1, 0) {
        byteswap(data, 3); // Byteswapped4
        true
    } else if matches(1, 0, 3, 2) {
        byteswap(data, 1); // Byteswapped2
        true
    } else {
        false
    }
}

/// Hash already-loaded ROM bytes the same way librecomp does. Returns `None` if
/// the bytes aren't a recognizable N64 ROM.
pub fn hash_rom_bytes(mut data: Vec<u8>) -> Option<u64> {
    // Pad to a multiple of 4 with zeros (recomp.cpp: resize((size+3) & ~3)).
    let padded = (data.len() + 3) & !3;
    data.resize(padded, 0);
    if !normalize_byteorder(&mut data) {
        return None;
    }
    Some(xxh3_64(&data))
}

/// Check a candidate ROM file against the expected Mario Builder 64 hash.
pub fn check_rom(path: &Path) -> RomCheck {
    let data = match std::fs::read(path) {
        Ok(d) if !d.is_empty() => d,
        Ok(_) => return RomCheck::Unreadable("file is empty".into()),
        Err(e) => return RomCheck::Unreadable(e.to_string()),
    };
    match hash_rom_bytes(data) {
        None => RomCheck::NotARom,
        Some(h) if h == EXPECTED_ROM_HASH => RomCheck::Good,
        Some(_) => RomCheck::WrongRom,
    }
}

/// Where the launcher stands on the ROM for a given data dir.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataDirRom {
    /// A valid ROM is in place (either already provisioned, or staged for first run).
    Ready,
    /// No usable ROM — the user needs to pick one.
    Missing,
    /// A ROM file is present but does not validate.
    Invalid,
}

/// Inspect a data dir and report whether the game will find a valid ROM there.
pub fn data_dir_rom_status(data_dir: &Path) -> DataDirRom {
    let provisioned = data_dir.join(PROVISIONED_ROM_NAME);
    if provisioned.exists() {
        return match check_rom(&provisioned) {
            RomCheck::Good => DataDirRom::Ready,
            RomCheck::Unreadable(_) => DataDirRom::Missing,
            _ => DataDirRom::Invalid,
        };
    }
    let source = data_dir.join(SOURCE_ROM_NAME);
    if source.exists() {
        return match check_rom(&source) {
            RomCheck::Good => DataDirRom::Ready,
            RomCheck::Unreadable(_) => DataDirRom::Missing,
            _ => DataDirRom::Invalid,
        };
    }
    DataDirRom::Missing
}

/// Verify `src` and, if good, copy it into `data_dir` as `mb64.z64` so the game
/// provisions it on next launch. Also clears any stale provisioned copy so the
/// game re-provisions from the freshly-placed source.
pub fn provision(src: &Path, data_dir: &Path) -> anyhow::Result<()> {
    match check_rom(src) {
        RomCheck::Good => {}
        RomCheck::NotARom => anyhow::bail!("that file is not an N64 ROM"),
        RomCheck::WrongRom => anyhow::bail!("that ROM is not the expected Mario Builder 64 build"),
        RomCheck::Unreadable(e) => anyhow::bail!("could not read the ROM: {e}"),
    }
    std::fs::create_dir_all(data_dir)?;
    let dest = data_dir.join(SOURCE_ROM_NAME);
    std::fs::copy(src, &dest)?;
    // Drop a stale provisioned ROM (and its sidecars) so librecomp re-provisions
    // from the new source on next launch.
    let provisioned: PathBuf = data_dir.join(PROVISIONED_ROM_NAME);
    let _ = std::fs::remove_file(&provisioned);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byteorder_roundtrips() {
        // A non-byteswapped header normalizes to itself.
        let mut a = vec![0x80, 0x37, 0x12, 0x40, 0xDE, 0xAD, 0xBE, 0xEF];
        assert!(normalize_byteorder(&mut a));
        assert_eq!(&a[0..4], &FIRST_ROM_BYTES);

        // A 4-byte-swapped header normalizes back.
        let mut b = vec![0x40, 0x12, 0x37, 0x80, 0xEF, 0xBE, 0xAD, 0xDE];
        assert!(normalize_byteorder(&mut b));
        assert_eq!(&b[0..4], &FIRST_ROM_BYTES);

        // Garbage header is rejected.
        let mut c = vec![0x00, 0x11, 0x22, 0x33];
        assert!(!normalize_byteorder(&mut c));
    }

    /// If the real built ROM is present in the repo, hashing it must reproduce the
    /// exact value the game checks — this is the ground-truth test that our
    /// replication of librecomp's algorithm is byte-correct.
    #[test]
    fn real_rom_matches_expected_hash() {
        let Some(repo) = crate::core::paths::find_repo_root() else { return };
        let rom = repo.join("app").join(SOURCE_ROM_NAME);
        if !rom.exists() {
            eprintln!("skipping: {} not present", rom.display());
            return;
        }
        assert_eq!(check_rom(&rom), RomCheck::Good);
    }
}
