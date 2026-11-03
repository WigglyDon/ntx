// SPDX-License-Identifier: MIT
//! Bounds the selected GLideN64 derived cache before its native stream loader.
//! Format facts: pinned glsl_ShaderStorage.{h,cpp}, CombinerProgramImpl and
//! CombinerKey. Opaque GPU binaries remain driver-owned, never ordinary saves.
use std::{
    collections::BTreeSet,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_FILE: u64 = 64 * 1024 * 1024;
const MAX_BINARY: usize = 16 * 1024 * 1024;
const MAX_PROGRAMS: usize = 4096;
fn invalid() -> io::Error {
    io::Error::other("Invalid derived N64 shader cache")
}

struct Cursor<'a> {
    bytes: &'a [u8],
}
impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        let value = self.bytes.get(..n).ok_or_else(invalid)?;
        self.bytes = &self.bytes[n..];
        Ok(value)
    }
    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_ne_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn string(&mut self) -> io::Result<()> {
        let n = self.u32()? as usize;
        if !(1..=4096).contains(&n) || self.take(n)?.contains(&0) {
            return Err(invalid());
        }
        Ok(())
    }
}
fn binary(bytes: &[u8]) -> io::Result<()> {
    if bytes.len() as u64 > MAX_FILE {
        return Err(invalid());
    }
    let mut c = Cursor { bytes };
    if c.u32()? != 0x3b {
        return Err(invalid());
    }
    c.u32()?; // Renderer options are checked against the actual core by GLideN64.
    c.string()?;
    c.string()?;
    let n = c.u32()? as usize;
    if !(1..=MAX_PROGRAMS).contains(&n) {
        return Err(invalid());
    }
    let mut seen = BTreeSet::new();
    for _ in 0..n {
        let key = u64::from_ne_bytes(c.take(8)?.try_into().unwrap());
        if !seen.insert(key) {
            return Err(invalid());
        }
        c.take(4)?; // Inputs bit mask.
        if c.u32()? == 0 {
            return Err(invalid());
        } // Driver's binary format token.
        let n = i32::from_ne_bytes(c.take(4)?.try_into().unwrap());
        if n <= 0 || n as usize > MAX_BINARY {
            return Err(invalid());
        }
        c.take(n as usize)?;
    }
    if !c.bytes.is_empty() {
        return Err(invalid());
    }
    Ok(())
}
fn keys(bytes: &[u8]) -> io::Result<()> {
    if bytes.len() > MAX_PROGRAMS * 32 + 64 {
        return Err(invalid());
    }
    let s = std::str::from_utf8(bytes).map_err(|_| invalid())?;
    let mut words = s.split_ascii_whitespace();
    let hex = |word: Option<&str>| -> io::Result<u64> {
        let word = word.ok_or_else(invalid)?;
        let digits = word.strip_prefix("0x").ok_or_else(invalid)?;
        if digits.is_empty() || digits.len() > 16 {
            return Err(invalid());
        }
        u64::from_str_radix(digits, 16).map_err(|_| invalid())
    };
    if hex(words.next())? != 5 {
        return Err(invalid());
    }
    let n = hex(words.next())?;
    if !(1..=MAX_PROGRAMS as u64).contains(&n) {
        return Err(invalid());
    }
    let mut seen = BTreeSet::new();
    for _ in 0..n {
        if !seen.insert(hex(words.next())?) {
            return Err(invalid());
        }
    }
    if words.next().is_some() {
        return Err(invalid());
    }
    Ok(())
}
fn kind(name: &str) -> Option<bool> {
    let mut p = name.split('.');
    if p.next()? != "GLideN64" {
        return None;
    }
    let hash = p.next()?;
    if hash.is_empty() || hash.len() > 8 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    if p.next()? != "GLES" {
        return None;
    }
    let binary = match p.next()? {
        "shaders" => true,
        "keys" => false,
        _ => return None,
    };
    p.next().is_none().then_some(binary)
}

#[derive(Default)]
pub struct Report {
    pub accepted: usize,
    pub retained_invalid: usize,
}
fn recovery(root: &Path) -> io::Result<PathBuf> {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for ordinal in 0..16 {
        let path = root.join(format!(
            "ntx-recovery-{}-{time}-{ordinal}",
            std::process::id()
        ));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(invalid())
}
/// Call while quiescent under the existing application/profile writer, before
/// loading a core. Only exact selected-cache filenames are inspected. Invalid
/// derivatives are atomically moved intact into a new, non-overwriting recovery
/// directory; saves, checkpoints, other files and unknown cache formats stay put.
pub fn prepare(backend: &Path) -> io::Result<Report> {
    // getStorageFileName falls back to backend itself if native mkdir fails.
    // Check both possible loader locations, even when shaders does not exist.
    let fallback = prepare_directory(backend)?;
    let nested = prepare_directory(&backend.join("shaders"))?;
    Ok(Report {
        accepted: fallback.accepted + nested.accepted,
        retained_invalid: fallback.retained_invalid + nested.retained_invalid,
    })
}
fn prepare_directory(root: &Path) -> io::Result<Report> {
    match root.symlink_metadata() {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Report::default()),
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {}
        Err(e) => return Err(e),
        _ => return Err(invalid()),
    }
    let mut report = Report::default();
    let mut retained = None;
    let mut entries = Vec::new();
    for (index, entry) in fs::read_dir(root)?.enumerate() {
        if index == MAX_PROGRAMS {
            return Err(invalid());
        }
        entries.push(entry?);
    }
    // Enumerate before moving anything so a directory mutation cannot hide a
    // second malformed file from this preparation.
    for entry in entries {
        let name = entry.file_name();
        let Some(is_binary) = name.to_str().and_then(kind) else {
            continue;
        };
        let path = entry.path();
        let meta = path.symlink_metadata()?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(invalid());
        }
        let valid = if meta.len() > MAX_FILE {
            false
        } else {
            let mut bytes = Vec::new();
            fs::File::open(&path)?
                .take(MAX_FILE + 1)
                .read_to_end(&mut bytes)?;
            if is_binary {
                binary(&bytes).is_ok()
            } else {
                keys(&bytes).is_ok()
            }
        };
        if valid {
            report.accepted += 1;
            continue;
        }
        if retained.is_none() {
            retained = Some(recovery(root)?);
        }
        // This new directory belongs exclusively to this preparation; rename
        // preserves the original inode/bytes and cannot replace an older cache.
        let destination = retained.as_ref().unwrap().join(name);
        fs::rename(&path, &destination)?;
        fs::File::open(&destination)?.sync_all()?;
        fs::File::open(retained.as_ref().unwrap())?.sync_all()?;
        fs::File::open(root)?.sync_all()?;
        report.retained_invalid += 1;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Vec<u8> {
        let mut b = Vec::new();
        for n in [0x3b_u32, 0, 3] {
            b.extend(n.to_ne_bytes());
        }
        b.extend(b"gpu");
        b.extend(3_u32.to_ne_bytes());
        b.extend(b"api");
        b.extend(1_u32.to_ne_bytes());
        b.extend(1_u64.to_ne_bytes());
        for n in [0_u32, 1, 3] {
            b.extend(n.to_ne_bytes());
        }
        b.extend(b"bin");
        b
    }
    #[test]
    fn native_envelope_rejects_every_truncation_and_unbounded_lengths() {
        let good = sample();
        binary(&good).unwrap();
        for end in 0..good.len() {
            assert!(binary(&good[..end]).is_err());
        }
        let mut huge = good.clone();
        huge[8..12].copy_from_slice(&u32::MAX.to_ne_bytes());
        assert!(binary(&huge).is_err());
        let mut negative = good.clone();
        let n = negative.len();
        negative[n - 7..n - 3].copy_from_slice(&(-1_i32).to_ne_bytes());
        assert!(binary(&negative).is_err());
        let mut duplicate = good.clone();
        duplicate[22..26].copy_from_slice(&2_u32.to_ne_bytes());
        duplicate.extend_from_slice(&good[26..]);
        assert!(binary(&duplicate).is_err());
        keys(b"0x00000005\n0x00000002\n0x1\n0xffff\n").unwrap();
        for bad in [
            b"0x5 0xffffffff".as_slice(),
            b"0x5 0x2 0x1",
            b"0x5 0x2 0x1 0x1",
            b"0x5 0x1 0x1 extra",
        ] {
            assert!(keys(bad).is_err());
        }
    }
    #[test]
    fn invalid_derivative_is_retained_exactly_and_retry_leaves_valid_and_unknown_files_alone() {
        let tmp = std::env::temp_dir().join(format!(
            "ntx-cache-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let root = tmp.join("shaders");
        fs::create_dir_all(&root).unwrap();
        let good = sample();
        fs::write(root.join("GLideN64.123.GLES.shaders"), &good).unwrap();
        let bad = b"0x5 0xfffffff";
        fs::write(root.join("GLideN64.123.GLES.keys"), bad).unwrap();
        fs::write(root.join("unknown.shaders"), b"untouched").unwrap();
        let report = prepare(&tmp).unwrap();
        assert_eq!((report.accepted, report.retained_invalid), (1, 1));
        assert_eq!(
            fs::read(root.join("GLideN64.123.GLES.shaders")).unwrap(),
            good
        );
        let retained = fs::read_dir(&root)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.is_dir())
            .unwrap();
        assert_eq!(
            fs::read(retained.join("GLideN64.123.GLES.keys")).unwrap(),
            bad
        );
        assert_eq!(
            fs::read(root.join("unknown.shaders")).unwrap(),
            b"untouched"
        );
        let retry = prepare(&tmp).unwrap();
        assert_eq!((retry.accepted, retry.retained_invalid), (1, 0));
        fs::remove_dir_all(tmp).unwrap();
    }
    #[test]
    fn native_fallback_and_nested_caches_are_both_checked_without_recovery_collisions() {
        let tmp = std::env::temp_dir().join(format!(
            "ntx-cache-fallback-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&tmp).unwrap();
        // Native falls back here if creation of its shaders directory fails.
        let name = "GLideN64.123.GLES.keys";
        fs::write(tmp.join(name), b"truncated fallback").unwrap();
        let report = prepare(&tmp).unwrap();
        assert_eq!((report.accepted, report.retained_invalid), (0, 1));
        assert!(!tmp.join(name).exists());
        let nested = tmp.join("shaders");
        fs::create_dir(&nested).unwrap();
        fs::write(tmp.join(name), b"second fallback").unwrap();
        fs::write(nested.join(name), b"nested bad data").unwrap();
        fs::write(tmp.join("ordinary-progress"), b"untouched").unwrap();
        let report = prepare(&tmp).unwrap();
        assert_eq!((report.accepted, report.retained_invalid), (0, 2));
        let mut retained = BTreeSet::new();
        for root in [&tmp, &nested] {
            for e in fs::read_dir(root).unwrap() {
                let p = e.unwrap().path();
                if p.file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with("ntx-recovery-")
                {
                    retained.insert(fs::read(p.join(name)).unwrap());
                }
            }
        }
        assert_eq!(
            retained,
            [
                b"truncated fallback".to_vec(),
                b"second fallback".to_vec(),
                b"nested bad data".to_vec()
            ]
            .into()
        );
        assert_eq!(
            fs::read(tmp.join("ordinary-progress")).unwrap(),
            b"untouched"
        );
        let retry = prepare(&tmp).unwrap();
        assert_eq!((retry.accepted, retry.retained_invalid), (0, 0));
        fs::remove_dir_all(tmp).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn cache_symlinks_are_never_followed_or_moved() {
        let tmp = std::env::temp_dir().join(format!(
            "ntx-cache-link-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let root = tmp.join("shaders");
        fs::create_dir_all(&root).unwrap();
        fs::write(tmp.join("progress"), b"kept").unwrap();
        std::os::unix::fs::symlink(tmp.join("progress"), root.join("GLideN64.1.GLES.keys"))
            .unwrap();
        assert!(prepare(&tmp).is_err());
        assert_eq!(fs::read(tmp.join("progress")).unwrap(), b"kept");
        assert!(
            root.join("GLideN64.1.GLES.keys")
                .symlink_metadata()
                .unwrap()
                .file_type()
                .is_symlink()
        );
        fs::remove_dir_all(tmp).unwrap();
    }
}
