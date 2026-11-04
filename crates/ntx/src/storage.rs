// SPDX-License-Identifier: MIT
//! Durable card/console generations under the runtime's exclusive profile ownership.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{CStr, c_char},
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::Mutex,
};
static ROOT: Mutex<Option<Store>> = Mutex::new(None);
static ERROR: Mutex<Option<String>> = Mutex::new(None);
#[cfg(test)]
pub(crate) static STORE_TEST_LOCK: Mutex<()> = Mutex::new(());
const GENERATION_FILE: &str = "save-generation.json";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct PendingWrite {
    path: String,
    digest: String,
    existed: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveGeneration {
    schema: u32,
    pub number: u64,
    pub images: BTreeMap<String, String>,
    pending: Option<PendingWrite>,
}
impl Default for SaveGeneration {
    fn default() -> Self {
        Self {
            schema: 1,
            number: 0,
            images: BTreeMap::new(),
            pending: None,
        }
    }
}
pub(crate) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn image_name(value: &str) -> bool {
    value == "GC/SRAM.raw"
        || value.strip_prefix("GC/MemoryCardA.").is_some_and(|suffix| {
            suffix.ends_with(".raw")
                && suffix.len() <= 32
                && suffix
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.')
        })
}
fn image_digest(root: &Path, name: &str) -> io::Result<Option<String>> {
    if !image_name(name) {
        return Err(io::Error::other("invalid ordinary-save path"));
    }
    let path = root.join(name);
    match path.symlink_metadata() {
        Ok(metadata)
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.len() > 64 * 1024 * 1024 =>
        {
            return Err(io::Error::other(
                "invalid ordinary-save file; original retained",
            ));
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
        _ => {}
    }
    if !path
        .parent()
        .ok_or_else(|| io::Error::other("missing save parent"))?
        .canonicalize()?
        .starts_with(root)
    {
        return Err(io::Error::other("ordinary-save path escaped its profile"));
    }
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(Some(format!("{:x}", hash.finalize())))
}
impl SaveGeneration {
    /// Read-only migration check. Never reconcile or publish a source generation.
    pub fn verify_retained(root: &Path) -> io::Result<()> {
        Self::read(root)?.verify(root)
    }
    fn read(root: &Path) -> io::Result<Self> {
        let path = root.join(GENERATION_FILE);
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e),
        };
        if path.symlink_metadata()?.file_type().is_symlink() || file.metadata()?.len() > 65536 {
            return Err(io::Error::other(
                "invalid save generation; original retained",
            ));
        }
        let mut bytes = Vec::new();
        file.take(65537).read_to_end(&mut bytes)?;
        if bytes.len() > 65536 {
            return Err(io::Error::other("save generation grew beyond its limit"));
        }
        let generation: Self = serde_json::from_slice(&bytes)?;
        if generation.schema != 1
            || generation.images.len() > 8
            || generation
                .images
                .iter()
                .any(|(name, hash)| !image_name(name) || !valid_digest(hash))
            || generation
                .pending
                .as_ref()
                .is_some_and(|p| !image_name(&p.path) || !valid_digest(&p.digest))
        {
            return Err(io::Error::other(
                "unsupported save generation; original retained",
            ));
        }
        Ok(generation)
    }
    fn publish(&self, root: &Path) -> io::Result<()> {
        atomic_replace(&root.join(GENERATION_FILE), &serde_json::to_vec(self)?)
    }
    fn next(&self) -> io::Result<u64> {
        self.number
            .checked_add(1)
            .ok_or_else(|| io::Error::other("save generation exhausted"))
    }
    fn reconcile(&mut self, root: &Path) -> io::Result<()> {
        let mut actual = self.images.clone();
        // Admit existing ABI5/profile files before native startup. They must be
        // tracked before any callback could replace stale in-memory data.
        match std::fs::read_dir(root.join("GC")) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry?;
                    let Some(name) = entry.file_name().to_str().map(|s| format!("GC/{s}")) else {
                        continue;
                    };
                    if image_name(&name) && !actual.contains_key(&name) {
                        let hash = image_digest(root, &name)?.ok_or_else(|| {
                            io::Error::other("ordinary save disappeared during admission")
                        })?;
                        actual.insert(name, hash);
                        if actual.len() > 8 {
                            return Err(io::Error::other("too many ordinary-save images"));
                        }
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        for name in self.images.keys() {
            let hash = image_digest(root, name)?.ok_or_else(|| {
                io::Error::other("ordinary save is missing; recover its backup before starting")
            })?;
            actual.insert(name.clone(), hash);
        }
        if let Some(pending) = &self.pending {
            if let Some(hash) = image_digest(root, &pending.path)? {
                actual.insert(pending.path.clone(), hash);
            } else if pending.existed {
                return Err(io::Error::other(
                    "interrupted ordinary save is missing; backup retained",
                ));
            }
        }
        if self.pending.is_some() || actual != self.images {
            let recovered = Self {
                number: self.next()?,
                images: actual,
                pending: None,
                schema: 1,
            };
            recovered.publish(root)?;
            *self = recovered;
        }
        Ok(())
    }
    pub fn verify(&self, root: &Path) -> io::Result<()> {
        let canonical = root.canonicalize()?;
        let root = canonical.as_path();
        if self.pending.is_some() || self.images.is_empty() || Self::read(root)? != *self {
            return Err(io::Error::other(
                "ordinary saves changed after this checkpoint; restart with current saves",
            ));
        }
        for (name, hash) in &self.images {
            if image_digest(root, name)?.as_ref() != Some(hash) {
                return Err(io::Error::other(
                    "ordinary save bytes changed; checkpoint and current saves retained",
                ));
            }
        }
        Ok(())
    }
}
pub(crate) fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
struct Store {
    root: PathBuf,
    generation: SaveGeneration,
    missing_reads: BTreeSet<String>,
}
impl Store {
    fn owned_image(&self, path: &Path) -> io::Result<(PathBuf, String)> {
        let normalized = path
            .parent()
            .ok_or_else(|| io::Error::other("missing save parent"))?
            .canonicalize()?
            .join(
                path.file_name()
                    .ok_or_else(|| io::Error::other("missing save name"))?,
            );
        let path = normalized.as_path();
        let relative = path
            .strip_prefix(&self.root)
            .ok()
            .and_then(|p| p.to_str())
            .filter(|s| image_name(s))
            .ok_or_else(|| io::Error::other("unexpected native save path"))?
            .to_owned();
        Ok((normalized, relative))
    }
    fn write(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let (normalized, relative) = self.owned_image(path)?;
        let path = normalized.as_path();
        // A failed earlier publication is reconciled from disk, never rolled back.
        if let Some(pending) = &self.generation.pending {
            // Only our acknowledged old bytes or our interrupted replacement are
            // admissible while native RAM is live. Rebinding before a fresh boot
            // may adopt external files; retrying an old runtime must not.
            for (name, old) in &self.generation.images {
                let actual = image_digest(&self.root, name)?;
                if actual.as_ref() != Some(old)
                    && !(name == &pending.path && actual.as_ref() == Some(&pending.digest))
                {
                    return Err(io::Error::other(
                        "ordinary save changed during a pending native write; restart with current saves",
                    ));
                }
            }
            let actual = image_digest(&self.root, &pending.path)?;
            if actual.as_ref() != self.generation.images.get(&pending.path)
                && actual.as_ref() != Some(&pending.digest)
            {
                return Err(io::Error::other(
                    "pending save changed outside this runtime; current file retained",
                ));
            }
            self.generation.reconcile(&self.root)?;
        }
        let actual = image_digest(&self.root, &relative)?;
        if self
            .generation
            .images
            .get(&relative)
            .is_some_and(|h| Some(h) != actual.as_ref())
        {
            return Err(io::Error::other(
                "ordinary save changed outside this runtime; restart with current saves",
            ));
        }
        if actual.is_some()
            && !self.generation.images.contains_key(&relative)
            && self.missing_reads.contains(&relative)
        {
            return Err(io::Error::other(
                "a save appeared after the native missing-file read; current file retained",
            ));
        }
        let hash = digest(bytes);
        if self.generation.images.get(&relative) == Some(&hash) {
            // Still acknowledge real file/directory sync on an explicit barrier.
            return atomic_replace(path, bytes);
        }
        let mut pending = self.generation.clone();
        pending.number = pending.next()?;
        pending.pending = Some(PendingWrite {
            path: relative.clone(),
            digest: hash.clone(),
            existed: actual.is_some(),
        });
        pending.publish(&self.root)?;
        self.generation = pending;
        atomic_replace(path, bytes)?;
        let mut committed = self.generation.clone();
        committed.pending = None;
        committed.images.insert(relative.clone(), hash);
        committed.publish(&self.root)?;
        self.generation = committed;
        self.missing_reads.remove(&relative);
        Ok(())
    }
}
/// Caller keeps the profile writer lock; no native execution may run between
/// capture, this verification and checkpoint publication.
pub fn save_generation(profile: &Path) -> io::Result<SaveGeneration> {
    let profile = profile.canonicalize()?;
    let active = ROOT
        .lock()
        .map_err(|_| io::Error::other("card store poisoned"))?;
    let store = active
        .as_ref()
        .filter(|s| s.root == profile)
        .ok_or_else(|| io::Error::other("profile does not own the native writer"))?;
    store.generation.verify(&profile)?;
    Ok(store.generation.clone())
}
pub struct CardStore;
impl CardStore {
    /// The caller holds ProfileLock until native teardown and this binding finish.
    pub fn bind(profile: &Path) -> io::Result<Self> {
        let mut active = ROOT
            .lock()
            .map_err(|_| io::Error::other("card store poisoned"))?;
        if active.is_some() {
            return Err(io::Error::other("another native card store is active"));
        }
        let profile = profile.canonicalize()?;
        let mut generation = SaveGeneration::read(&profile)?;
        // Only metadata changes here, before native startup. Current files win.
        generation.reconcile(&profile)?;
        // Earlier failures are delivered with their runtime identity before
        // teardown. Do not attribute an old profile's last write to a new one.
        let mut error = ERROR
            .lock()
            .map_err(|_| io::Error::other("card error store poisoned"))?;
        *error = None;
        *active = Some(Store {
            root: profile,
            generation,
            missing_reads: BTreeSet::new(),
        });
        Ok(Self)
    }
}
impl Drop for CardStore {
    fn drop(&mut self) {
        if let Ok(mut root) = ROOT.lock() {
            *root = None;
        }
    }
}
pub fn atomic_replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    atomic_replace_parts(path, &[bytes])
}
/// Publish a new immutable generation under the caller's exclusive profile
/// writer. Preserve any incomplete prior attempt rather than truncating it.
pub(crate) fn publish_immutable(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("missing storage parent"))?;
    match path.symlink_metadata() {
        Ok(_) => return Err(io::Error::other("immutable generation already exists")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let mut name = path
        .file_name()
        .ok_or_else(|| io::Error::other("missing storage name"))?
        .to_os_string();
    name.push(".ntx-pending");
    let temporary = parent.join(name);
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                io::Error::other("An incomplete save or resume write is retained for recovery")
            } else {
                error
            }
        })?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(temporary, path)?;
    File::open(parent)?.sync_all()
}
pub(crate) fn atomic_replace_parts(path: &Path, parts: &[&[u8]]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("missing storage parent"))?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("missing storage name"))?;
    let mut temporary_name = name.to_os_string();
    temporary_name.push(".ntx-pending");
    let temporary = parent.join(temporary_name);
    if temporary
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
    {
        return Err(io::Error::other("storage candidate is a symlink"));
    }
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&temporary)?;
    for bytes in parts {
        file.write_all(bytes)?;
    }
    file.sync_all()?;
    drop(file);
    std::fs::rename(&temporary, path)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}
/// Native saved-image reader. Query with a null buffer/capacity0, then fill the
/// exact queried size. Return1present,0genuinely absent,-1error. Existing images
/// must match the bound durable generation; an I/O error is never a new card.
///
/// # Safety
/// `filename` is a NUL-terminated string, `length` is writable, and nonzero
/// `capacity` lends a writable buffer of that size until this call returns.
pub unsafe extern "C" fn read_image(
    filename: *const c_char,
    data: *mut u8,
    capacity: usize,
    length: *mut usize,
) -> i32 {
    if filename.is_null() || length.is_null() || capacity > 64 * 1024 * 1024 {
        return -1;
    }
    unsafe {
        *length = 0;
    }
    let result = (|| -> io::Result<i32> {
        let path = Path::new(std::ffi::OsStr::from_bytes(
            unsafe { CStr::from_ptr(filename) }.to_bytes(),
        ));
        let mut active = ROOT
            .lock()
            .map_err(|_| io::Error::other("save store poisoned"))?;
        let store = active
            .as_mut()
            .ok_or_else(|| io::Error::other("no bound saved-image owner"))?;
        let (path, relative) = store.owned_image(path)?;
        if store.generation.pending.is_some()
            || SaveGeneration::read(&store.root)? != store.generation
        {
            return Err(io::Error::other(
                "save metadata changed or has a pending write; restart with current files",
            ));
        }
        let metadata = match path.symlink_metadata() {
            Ok(metadata) => metadata,
            Err(e)
                if e.kind() == io::ErrorKind::NotFound
                    && !store.generation.images.contains_key(&relative) =>
            {
                store.missing_reads.insert(relative);
                return Ok(0);
            }
            Err(e) => return Err(e),
        };
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(io::Error::other(
                "saved image is not an ordinary file; original retained",
            ));
        }
        // ENOENT here is an error: an admitted/present file disappeared between
        // the existence check and open, rather than a genuinely new profile.
        let mut file = File::open(&path)?;
        let size = usize::try_from(file.metadata()?.len()).map_err(io::Error::other)?;
        let valid_size = if relative == "GC/SRAM.raw" {
            size == 68
        } else {
            (512 * 1024..=16 * 1024 * 1024).contains(&size) && size.is_power_of_two()
        };
        if !valid_size {
            return Err(io::Error::other(
                "unsupported or truncated saved image; original retained",
            ));
        }
        let expected = store.generation.images.get(&relative).ok_or_else(|| {
            io::Error::other("saved image appeared after writer admission; original retained")
        })?;
        if capacity == 0 {
            unsafe {
                *length = size;
            }
            return Ok(1);
        }
        if data.is_null() || capacity != size {
            return Err(io::Error::other(
                "saved image size changed during native load",
            ));
        }
        let bytes = unsafe { std::slice::from_raw_parts_mut(data, capacity) };
        file.read_exact(bytes)?;
        let mut trailing = [0];
        if file.read(&mut trailing)? != 0 || digest(bytes) != *expected {
            return Err(io::Error::other(
                "saved image changed or could not be read exactly; original retained",
            ));
        }
        unsafe {
            *length = size;
        }
        Ok(1)
    })();
    match result {
        Ok(status) => status,
        Err(error) => {
            if let Ok(mut saved) = ERROR.lock() {
                *saved = Some(error.to_string());
            }
            -1
        }
    }
}
/// Native lends the path and complete card bytes only for this synchronous call.
/// Returns success only after file sync, atomic rename and directory sync.
/// # Safety
/// `path` is a valid NUL-terminated string and `data` lends `length` readable bytes.
pub unsafe extern "C" fn write_card(path: *const c_char, data: *const u8, length: usize) -> i32 {
    let result = (|| -> io::Result<()> {
        if path.is_null() || data.is_null() || length == 0 || length > 64 * 1024 * 1024 {
            return Err(io::Error::other("invalid native card buffer"));
        }
        // SAFETY: the native writer contract lends valid bytes for this call.
        let path = Path::new(std::ffi::OsStr::from_bytes(
            unsafe { CStr::from_ptr(path) }.to_bytes(),
        ));
        let bytes = unsafe { std::slice::from_raw_parts(data, length) };
        let mut root = ROOT
            .lock()
            .map_err(|_| io::Error::other("card store poisoned"))?;
        let root = root
            .as_mut()
            .ok_or_else(|| io::Error::other("no owned card store"))?;
        if !path.is_absolute()
            || path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            || !path
                .parent()
                .ok_or_else(|| io::Error::other("missing card parent"))?
                .canonicalize()?
                .starts_with(&root.root)
        {
            return Err(io::Error::other("card path escaped its owned profile"));
        }
        root.write(path, bytes)
    })();
    match result {
        Ok(()) => {
            if let Ok(mut saved) = ERROR.lock() {
                *saved = None;
            }
            1
        }
        Err(error) => {
            if let Ok(mut saved) = ERROR.lock() {
                *saved = Some(error.to_string());
            }
            0
        }
    }
}
pub fn last_card_error() -> Option<String> {
    ERROR.lock().ok().and_then(|e| e.clone())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn isolated(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "ntx-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join("GC")).unwrap();
        root
    }
    fn native_write(path: &Path, bytes: &[u8]) -> i32 {
        let path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        unsafe { write_card(path.as_ptr(), bytes.as_ptr(), bytes.len()) }
    }
    fn native_read(path: &Path, buffer: Option<&mut [u8]>) -> (i32, usize) {
        let path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        let (data, capacity) =
            buffer.map_or((std::ptr::null_mut(), 0), |b| (b.as_mut_ptr(), b.len()));
        let mut length = usize::MAX;
        let result = unsafe { read_image(path.as_ptr(), data, capacity, &mut length) };
        (result, length)
    }
    #[test]
    fn checked_reader_loads_exact_card_and_console_but_refuses_changed_or_truncated_images() {
        let _serial = STORE_TEST_LOCK.lock().unwrap();
        let root = isolated("checked-read");
        let writer = CardStore::bind(&root).unwrap();
        let card = root.join("GC/MemoryCardA.fixture.raw");
        let console = root.join("GC/SRAM.raw");
        let card_bytes = vec![0x51; 512 * 1024];
        assert_eq!(native_write(&card, &card_bytes), 1);
        assert_eq!(native_write(&console, &[0x21; 68]), 1);
        assert_eq!(native_read(&card, None), (1, card_bytes.len()));
        let mut loaded = vec![0; card_bytes.len()];
        assert_eq!(native_read(&card, Some(&mut loaded)), (1, card_bytes.len()));
        assert_eq!(loaded, card_bytes);
        let mut sram = [0; 68];
        assert_eq!(native_read(&console, Some(&mut sram)), (1, 68));
        assert_eq!(sram, [0x21; 68]);
        // Same capacity but changed bytes cannot silently become native RAM.
        atomic_replace(&card, &vec![0x52; card_bytes.len()]).unwrap();
        assert_eq!(native_read(&card, Some(&mut loaded)), (-1, 0));
        assert_eq!(std::fs::read(&card).unwrap(), vec![0x52; card_bytes.len()]);
        // A truncate between query and fill never becomes a new card.
        atomic_replace(&card, b"truncated original fixture").unwrap();
        assert_eq!(native_read(&card, Some(&mut loaded)), (-1, 0));
        assert_eq!(std::fs::read(&card).unwrap(), b"truncated original fixture");
        atomic_replace(&console, &[0x31; 67]).unwrap();
        assert_eq!(native_read(&console, Some(&mut sram)), (-1, 0));
        assert_eq!(std::fs::read(&console).unwrap(), [0x31; 67]);
        assert!(last_card_error().is_some());
        drop(writer);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn checked_missing_read_refuses_external_creation_until_fresh_admission() {
        let _serial = STORE_TEST_LOCK.lock().unwrap();
        let root = isolated("checked-missing");
        let writer = CardStore::bind(&root).unwrap();
        let card = root.join("GC/MemoryCardA.fixture.raw");
        assert_eq!(native_read(&card, None), (0, 0));
        let current = vec![0x61; 512 * 1024];
        std::fs::write(&card, &current).unwrap();
        assert_eq!(native_read(&card, None), (-1, 0));
        assert_eq!(native_write(&card, &vec![0x62; current.len()]), 0);
        assert_eq!(std::fs::read(&card).unwrap(), current);
        drop(writer);
        let writer = CardStore::bind(&root).unwrap();
        let mut loaded = vec![0; current.len()];
        assert_eq!(native_read(&card, Some(&mut loaded)), (1, current.len()));
        assert_eq!(loaded, current);
        // Dangling links are errors, not absence/default creation.
        let console = root.join("GC/SRAM.raw");
        std::os::unix::fs::symlink(root.join("missing-target"), &console).unwrap();
        assert_eq!(native_read(&console, None), (-1, 0));
        drop(writer);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn legacy_card_and_sram_are_admitted_before_first_native_write() {
        let _serial = STORE_TEST_LOCK.lock().unwrap();
        let root = isolated("generation-legacy");
        let card = root.join("GC/MemoryCardA.fixture.raw");
        let sram = root.join("GC/SRAM.raw");
        std::fs::write(&card, b"existing card").unwrap();
        std::fs::write(&sram, b"existing console").unwrap();
        let writer = CardStore::bind(&root).unwrap();
        let admitted = save_generation(&root).unwrap();
        assert_eq!(admitted.images.len(), 2);
        assert_eq!(std::fs::read(&card).unwrap(), b"existing card");
        atomic_replace(&sram, b"newer console data").unwrap();
        assert_eq!(native_write(&sram, b"stale native console"), 0);
        assert_eq!(std::fs::read(&sram).unwrap(), b"newer console data");
        assert!(admitted.verify(&root).is_err());
        drop(writer);
        let writer = CardStore::bind(&root).unwrap();
        assert!(save_generation(&root).unwrap().number > admitted.number);
        drop(writer);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn generation_rejects_old_checkpoint_even_after_card_bytes_return_to_the_old_value() {
        let _serial = STORE_TEST_LOCK.lock().unwrap();
        let root = isolated("generation-history");
        let writer = CardStore::bind(&root).unwrap();
        let path = root.join("GC/MemoryCardA.fixture.raw");
        assert_eq!(native_write(&path, b"original save"), 1);
        assert_eq!(
            native_write(&root.join("GC/SRAM.raw"), b"console identity"),
            1
        );
        let old = save_generation(&root).unwrap();
        assert_eq!(native_write(&path, b"original save"), 1);
        assert_eq!(save_generation(&root).unwrap(), old);
        assert_eq!(native_write(&path, b"newer save"), 1);
        assert_eq!(native_write(&path, b"original save"), 1);
        assert!(old.verify(&root).is_err());
        let current = save_generation(&root).unwrap();
        assert!(current.number > old.number);
        assert_eq!(current.images, old.images);
        drop(writer);
        let writer = CardStore::bind(&root).unwrap();
        assert_eq!(save_generation(&root).unwrap(), current);
        drop(writer);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn interrupted_generation_publication_keeps_newer_data_and_recovers_without_rollback() {
        let _serial = STORE_TEST_LOCK.lock().unwrap();
        let root = isolated("generation-interruption");
        let writer = CardStore::bind(&root).unwrap();
        let name = "GC/MemoryCardA.fixture.raw";
        let path = root.join(name);
        assert_eq!(native_write(&path, b"old save"), 1);
        let old = save_generation(&root).unwrap();
        drop(writer);
        // Actual crash boundary: durable intent and card rename succeeded;
        // publication of the clean generation has not happened.
        let mut pending = old.clone();
        pending.number = pending.next().unwrap();
        pending.pending = Some(PendingWrite {
            path: name.into(),
            digest: digest(b"new save"),
            existed: true,
        });
        pending.publish(&root).unwrap();
        atomic_replace(&path, b"new save").unwrap();
        let obstruction = root.join("save-generation.json.ntx-pending");
        std::fs::create_dir(&obstruction).unwrap();
        assert!(CardStore::bind(&root).is_err());
        assert!(old.verify(&root).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"new save");
        assert_eq!(SaveGeneration::read(&root).unwrap(), pending);
        std::fs::remove_dir(obstruction).unwrap();
        let writer = CardStore::bind(&root).unwrap();
        let recovered = save_generation(&root).unwrap();
        assert!(recovered.number > pending.number);
        assert_eq!(std::fs::read(&path).unwrap(), b"new save");
        assert!(old.verify(&root).is_err());
        drop(writer);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn failed_intent_and_outside_change_cannot_overwrite_current_progress() {
        let _serial = STORE_TEST_LOCK.lock().unwrap();
        let root = isolated("generation-write-fault");
        let writer = CardStore::bind(&root).unwrap();
        let path = root.join("GC/MemoryCardA.fixture.raw");
        assert_eq!(native_write(&path, b"old save"), 1);
        let obstruction = root.join("save-generation.json.ntx-pending");
        std::fs::create_dir(&obstruction).unwrap();
        assert_eq!(native_write(&path, b"new save"), 0);
        assert_eq!(std::fs::read(&path).unwrap(), b"old save");
        assert!(last_card_error().is_some());
        std::fs::remove_dir(obstruction).unwrap();
        assert_eq!(native_write(&path, b"new save"), 1);
        let generation = save_generation(&root).unwrap();
        atomic_replace(&path, b"externally imported newer save").unwrap();
        assert_eq!(native_write(&path, b"stale native RAM"), 0);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"externally imported newer save"
        );
        drop(writer);
        let writer = CardStore::bind(&root).unwrap();
        assert!(generation.verify(&root).is_err());
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"externally imported newer save"
        );
        drop(writer);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn pending_native_retry_cannot_adopt_and_overwrite_an_external_newer_save() {
        let _serial = STORE_TEST_LOCK.lock().unwrap();
        let root = isolated("generation-live-pending");
        let writer = CardStore::bind(&root).unwrap();
        let name = "GC/MemoryCardA.fixture.raw";
        let path = root.join(name);
        assert_eq!(native_write(&path, b"old card"), 1);
        {
            let mut store = ROOT.lock().unwrap();
            let store = store.as_mut().unwrap();
            let mut pending = store.generation.clone();
            pending.number = pending.next().unwrap();
            pending.pending = Some(PendingWrite {
                path: name.into(),
                digest: digest(b"native replacement"),
                existed: true,
            });
            pending.publish(&root).unwrap();
            store.generation = pending;
        }
        atomic_replace(&path, b"newer external card").unwrap();
        assert_eq!(native_write(&path, b"native replacement"), 0);
        assert_eq!(std::fs::read(&path).unwrap(), b"newer external card");
        drop(writer);
        let writer = CardStore::bind(&root).unwrap();
        assert_eq!(
            save_generation(&root).unwrap().images[name],
            digest(b"newer external card")
        );
        drop(writer);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn native_writer_requires_one_bound_profile_and_rejects_other_paths() {
        let _serial = STORE_TEST_LOCK.lock().unwrap();
        let root = std::env::temp_dir().join(format!("ntx-bound-card-{}", std::process::id()));
        let owned = root.join("owned");
        std::fs::create_dir_all(&owned).unwrap();
        std::fs::create_dir(owned.join("GC")).unwrap();
        let outside = root.join("outside.raw");
        std::fs::write(&outside, b"retain").unwrap();
        let store = CardStore::bind(&owned).unwrap();
        assert!(CardStore::bind(&owned).is_err());
        let outside_path = std::ffi::CString::new(outside.as_os_str().as_bytes()).unwrap();
        assert_eq!(
            unsafe { write_card(outside_path.as_ptr(), b"new".as_ptr(), 3) },
            0
        );
        assert_eq!(std::fs::read(&outside).unwrap(), b"retain");
        assert!(last_card_error().is_some());
        let card = owned.join("GC/MemoryCardA.fixture.raw");
        let card_path = std::ffi::CString::new(card.as_os_str().as_bytes()).unwrap();
        assert_eq!(
            unsafe { write_card(card_path.as_ptr(), b"new".as_ptr(), 3) },
            1
        );
        assert_eq!(std::fs::read(&card).unwrap(), b"new");
        assert!(last_card_error().is_none());
        drop(store);
        assert_eq!(
            unsafe { write_card(card_path.as_ptr(), b"old".as_ptr(), 3) },
            0
        );
        assert_eq!(std::fs::read(&card).unwrap(), b"new");
        assert!(last_card_error().is_some());
        let rebound = CardStore::bind(&owned).unwrap();
        assert!(last_card_error().is_none());
        drop(rebound);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn failed_candidate_keeps_previous_card_and_success_replaces_it() {
        let root = std::env::temp_dir().join(format!("ntx-atomic-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let card = root.join("card.raw");
        std::fs::write(&card, b"durable old generation").unwrap();
        let pending = root.join("card.raw.ntx-pending");
        std::fs::create_dir(&pending).unwrap();
        assert!(atomic_replace(&card, b"new generation").is_err());
        assert_eq!(std::fs::read(&card).unwrap(), b"durable old generation");
        std::fs::remove_dir(&pending).unwrap();
        atomic_replace(&card, b"new generation").unwrap();
        assert_eq!(std::fs::read(&card).unwrap(), b"new generation");
        assert!(!pending.exists());
        std::fs::remove_file(card).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
