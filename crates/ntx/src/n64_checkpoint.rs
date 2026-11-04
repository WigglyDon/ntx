// SPDX-License-Identifier: MIT
//! N64 continuation binds the pinned machine to its coherent ordinary save
//! generation. No core/architecture/option fingerprint is rewritten on import.
use crate::{
    library::valid_id,
    n64_save::{Generation, SaveStore},
    storage,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io,
    path::{Path, PathBuf},
};
const MAGIC: &[u8; 8] = b"NTXN6401";
const LIMIT: u64 = 64 * 1024 * 1024;
/// At most three on-disk candidates, including incomplete/recovery files. Normal
/// acknowledged replacement retires the prior scene and leaves one current file.
pub const MAX_RETAINED_FILES: usize = 3;
pub const FREE_SPACE_RESERVE: u64 = 64 * 1024 * 1024;
const FILE_LIMIT: u64 = LIMIT + 65548;

/// Host adapters supply their observed free space; policy remains portable.
/// This is admission, not a reservation: atomic writes must still handle failure.
pub fn admit_capture(profile: &Path, payload_bytes: usize, available: u64) -> io::Result<()> {
    if payload_bytes == 0 || payload_bytes as u64 > LIMIT {
        return Err(error("Invalid N64 scene size"));
    }
    let needed = payload_bytes as u64 + 65548 + crate::n64::SAVE_IMAGE_BYTES as u64;
    if available < needed + FREE_SPACE_RESERVE {
        return Err(error(
            "Not enough free space to keep your current place. In-game saves and previous recovery data are kept.",
        ));
    }
    let dir = path(profile, &"0".repeat(64))?
        .parent()
        .unwrap()
        .to_path_buf();
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let mut count = 0;
    let mut bytes = 0u64;
    for entry in entries {
        let entry = entry?;
        let info = entry.path().symlink_metadata()?;
        if !info.is_file() {
            return Err(error(
                "Unrecognized N64 recovery entry; original data retained",
            ));
        }
        count += 1;
        bytes = bytes
            .checked_add(info.len())
            .ok_or_else(|| error("N64 resume storage exceeds its budget"))?;
        if count >= MAX_RETAINED_FILES
            || bytes
                .checked_add(needed)
                .is_none_or(|n| n > MAX_RETAINED_FILES as u64 * FILE_LIMIT)
        {
            return Err(error(
                "Resume recovery storage is full. In-game saves and existing recovery data are kept; free space before retrying.",
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub game_id: String,
    pub profile_id: String,
    pub media_sha256: String,
    pub core_sha256: String,
    pub options_sha256: String,
    pub architecture: String,
}
impl Binding {
    fn validate(&self) -> io::Result<()> {
        if !valid_id(&self.game_id)
            || !valid_id(&self.profile_id)
            || ![&self.media_sha256, &self.core_sha256, &self.options_sha256]
                .iter()
                .all(|s| storage::valid_digest(s))
            || self.architecture != "aarch64-android-gles3-mupen-state1"
        {
            return Err(error(
                "Unsupported N64 scene binding; current saves retained",
            ));
        }
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
struct Metadata {
    schema: u32,
    binding: Binding,
    ordinary: Generation,
    payload_sha256: String,
    payload_bytes: usize,
}
pub struct Checkpoint {
    pub payload: Vec<u8>,
    ordinary: Generation,
}
impl Checkpoint {
    pub fn verify_current(&self, store: &SaveStore) -> io::Result<()> {
        store.verify_current()?;
        if store.generation() != Some(&self.ordinary) {
            return Err(error(
                "N64 scene belongs to an older ordinary-save generation; retain it and cold start explicitly with current saves",
            ));
        }
        Ok(())
    }
}
fn error(s: &str) -> io::Error {
    io::Error::other(s)
}
fn path(profile: &Path, id: &str) -> io::Result<PathBuf> {
    if !storage::valid_digest(id) {
        return Err(error("Invalid N64 scene identity"));
    }
    let dir = profile.join("n64-checkpoints");
    if dir.symlink_metadata().is_ok_and(|m| !m.is_dir()) {
        return Err(error("Unsafe N64 scene directory"));
    }
    Ok(dir.join(format!("{id}.ntx")))
}
/// The caller has yielded immediately after the actual native save job, then
/// committed the ordinary image from that same boundary. Keep the writer owned
/// until the collection references this returned complete generation durably.
pub fn publish(
    profile: &Path,
    binding: &Binding,
    store: &SaveStore,
    payload: &[u8],
    available: u64,
) -> io::Result<String> {
    binding.validate()?;
    store.ensure_profile(profile)?;
    store.verify_current()?;
    if payload.is_empty() || payload.len() as u64 > LIMIT {
        return Err(error("Invalid N64 scene size"));
    }
    let metadata = Metadata {
        schema: 1,
        binding: binding.clone(),
        ordinary: store
            .generation()
            .ok_or_else(|| error("N64 scene has no committed ordinary generation"))?
            .clone(),
        payload_sha256: storage::digest(payload),
        payload_bytes: payload.len(),
    };
    let header = serde_json::to_vec(&metadata)?;
    if header.len() > 65536 {
        return Err(error("N64 scene header too large"));
    }
    let mut bytes = Vec::with_capacity(12 + header.len() + payload.len());
    bytes.extend(MAGIC);
    bytes.extend((header.len() as u32).to_le_bytes());
    bytes.extend(header);
    bytes.extend(payload);
    let id = storage::digest(&bytes);
    let destination = path(profile, &id)?;
    if destination.try_exists()? {
        if fs::read(&destination)? != bytes {
            return Err(error("N64 retained scene collision"));
        }
    } else {
        admit_capture(profile, payload.len(), available)?;
        fs::create_dir_all(destination.parent().unwrap())?;
        storage::atomic_replace(&destination, &bytes)?;
    }
    if fs::read(&destination)? != bytes {
        return Err(error("N64 scene readback differs"));
    }
    store.verify_current()?;
    Ok(id)
}
pub fn load(
    profile: &Path,
    id: &str,
    binding: &Binding,
    store: &SaveStore,
) -> io::Result<Checkpoint> {
    binding.validate()?;
    store.ensure_profile(profile)?;
    let source = path(profile, id)?;
    let info = source.symlink_metadata()?;
    if !info.is_file() || info.len() < 12 || info.len() > LIMIT + 65548 {
        return Err(error("Invalid N64 scene file"));
    }
    let bytes = fs::read(source)?;
    if storage::digest(&bytes) != id || &bytes[..8] != MAGIC {
        return Err(error("N64 scene integrity differs"));
    }
    let size = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    if size > 65536 || 12 + size >= bytes.len() {
        return Err(error("N64 scene header is truncated"));
    }
    let metadata: Metadata = serde_json::from_slice(&bytes[12..12 + size])?;
    let payload = &bytes[12 + size..];
    if metadata.schema != 1
        || metadata.binding != *binding
        || metadata.payload_bytes != payload.len()
        || storage::digest(payload) != metadata.payload_sha256
    {
        return Err(error(
            "N64 scene compatibility differs; exact bytes and current progress retained",
        ));
    }
    let state = Checkpoint {
        payload: payload.to_vec(),
        ordinary: metadata.ordinary,
    };
    state.verify_current(store)?;
    Ok(state)
}
/// Retained scenes, including recovery candidates, pin their ordinary images.
/// Call after an acknowledged ordinary reference while retaining the profile
/// writer. Every scene still on disk pins its image, including uncertain scenes.
pub fn retire_ordinary_unreferenced(
    profile: &Path,
    binding: &Binding,
    store: &mut SaveStore,
) -> io::Result<()> {
    use std::collections::BTreeSet;
    store.ensure_profile(profile)?;
    let mut retained = BTreeSet::new();
    let dir = path(profile, &"0".repeat(64))?
        .parent()
        .unwrap()
        .to_path_buf();
    let entries = match fs::read_dir(dir) {
        Ok(entries) => Some(entries),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    if let Some(entries) = entries {
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let id = name
                .to_str()
                .and_then(|n| n.strip_suffix(".ntx"))
                .filter(|id| storage::valid_digest(id))
                .ok_or_else(|| {
                    error("Unrecognized N64 recovery candidate; ordinary history retained")
                })?;
            let info = entry.path().symlink_metadata()?;
            if !info.is_file() || info.len() < 12 || info.len() > FILE_LIMIT {
                return Err(error(
                    "Invalid N64 recovery candidate; ordinary history retained",
                ));
            }
            let bytes = fs::read(entry.path())?;
            if storage::digest(&bytes) != id || &bytes[..8] != MAGIC {
                return Err(error(
                    "Damaged N64 recovery candidate; ordinary history retained",
                ));
            }
            let size = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
            if size > 65536 || 12 + size >= bytes.len() {
                return Err(error(
                    "Truncated N64 recovery candidate; ordinary history retained",
                ));
            }
            let metadata: Metadata = serde_json::from_slice(&bytes[12..12 + size])?;
            if metadata.schema != 1
                || metadata.binding.game_id != binding.game_id
                || metadata.binding.profile_id != binding.profile_id
                || metadata.binding.media_sha256 != binding.media_sha256
                || metadata.ordinary.schema != 1
                || metadata.ordinary.number == 0
                || !storage::valid_digest(&metadata.ordinary.image_sha256)
                || metadata.payload_bytes != bytes.len() - 12 - size
                || storage::digest(&bytes[12 + size..]) != metadata.payload_sha256
            {
                return Err(error(
                    "Unrecognized N64 recovery binding; ordinary history retained",
                ));
            }
            retained.insert(metadata.ordinary.image_sha256);
        }
    }
    store.retire_unreferenced_images(&retained)
}

/// Only after the caller durably publishes `current` while retaining the
/// profile writer. An uncertain publication never calls this cleanup boundary.
pub fn retire_previous(
    profile: &Path,
    old: &str,
    current: &str,
    binding: &Binding,
    store: &SaveStore,
) -> io::Result<()> {
    // Verify the replacement and current ordinary generation before any removal.
    load(profile, current, binding, store)?;
    if old != current {
        discard_successful(profile, old)?;
    }
    Ok(())
}

/// Caller must own the profile through acknowledged replacement or a joined,
/// durably published explicit Stop. Never scan/delete unknown recovery scenes.
pub fn discard_successful(profile: &Path, id: &str) -> io::Result<()> {
    let old = path(profile, id)?;
    match old.symlink_metadata() {
        Ok(info) if !info.is_file() || info.len() < 12 || info.len() > FILE_LIMIT => {
            return Err(error("Unsafe N64 scene retained"));
        }
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    }
    let bytes = fs::read(&old)?;
    if storage::digest(&bytes) != id || &bytes[..8] != MAGIC {
        return Err(error("Damaged N64 scene retained for recovery"));
    }
    fs::remove_file(old)?;
    File::open(profile.join("n64-checkpoints"))?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup(name: &str) -> (PathBuf, SaveStore, Binding) {
        let root =
            std::env::temp_dir().join(format!("ntx-n64-retention-{name}-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let mut store = SaveStore::open(&root).unwrap();
        store
            .commit(&vec![0; crate::n64::SAVE_IMAGE_BYTES])
            .unwrap();
        let binding = Binding {
            game_id: "a".repeat(32),
            profile_id: "b".repeat(32),
            media_sha256: "c".repeat(64),
            core_sha256: "d".repeat(64),
            options_sha256: "e".repeat(64),
            architecture: "aarch64-android-gles3-mupen-state1".into(),
        };
        (root, store, binding)
    }
    fn count(root: &Path) -> usize {
        fs::read_dir(root.join("n64-checkpoints")).unwrap().count()
    }
    #[test]
    fn different_profile_writer_cannot_publish_or_prune_another_games_data() {
        let (a, mut a_store, binding) = setup("writer-a");
        let (b, mut b_store, _) = setup("writer-b");
        let id = publish(&a, &binding, &a_store, b"a scene", u64::MAX).unwrap();
        assert!(publish(&a, &binding, &b_store, b"wrong writer", u64::MAX).is_err());
        assert!(load(&a, &id, &binding, &b_store).is_err());
        assert!(retire_ordinary_unreferenced(&a, &binding, &mut b_store).is_err());
        assert!(path(&a, &id).unwrap().exists());
        retire_ordinary_unreferenced(&a, &binding, &mut a_store).unwrap();
        assert!(a_store.load().unwrap().is_some() && b_store.load().unwrap().is_some());
        drop(a_store);
        drop(b_store);
        fs::remove_dir_all(a).unwrap();
        fs::remove_dir_all(b).unwrap();
    }
    #[test]
    fn successful_replacement_and_stop_bound_storage_without_sweeping_recovery() {
        let (root, mut store, binding) = setup("replace");
        let recovery = publish(
            &root,
            &binding,
            &store,
            b"unacknowledged recovery",
            u64::MAX,
        )
        .unwrap();
        let mut current = publish(&root, &binding, &store, b"current", u64::MAX).unwrap();
        let mut ordinary = vec![0; crate::n64::SAVE_IMAGE_BYTES];
        for n in 0..40 {
            ordinary[0x800] = n + 1;
            store.commit(&ordinary).unwrap();
            let next = publish(
                &root,
                &binding,
                &store,
                format!("next {n}").as_bytes(),
                u64::MAX,
            )
            .unwrap();
            assert_eq!(count(&root), 3); // current, replacement and untouched recovery
            retire_previous(&root, &current, &next, &binding, &store).unwrap();
            assert_eq!(count(&root), 2);
            retire_ordinary_unreferenced(&root, &binding, &mut store).unwrap();
            assert_eq!(
                fs::read_dir(root.join("N64"))
                    .unwrap()
                    .filter(|e| e
                        .as_ref()
                        .unwrap()
                        .path()
                        .extension()
                        .is_some_and(|e| e == "m64ram"))
                    .count(),
                2
            );
            assert_eq!(store.load().unwrap().unwrap(), ordinary);
            assert!(path(&root, &recovery).unwrap().exists());
            current = next;
        }
        assert!(retire_previous(&root, &current, &"0".repeat(64), &binding, &store).is_err());
        assert!(path(&root, &current).unwrap().exists());
        retire_previous(&root, &current, &current, &binding, &store).unwrap();
        discard_successful(&root, &current).unwrap();
        discard_successful(&root, &current).unwrap();
        assert_eq!(count(&root), 1);
        assert!(path(&root, &recovery).unwrap().exists());
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn capacity_and_incomplete_candidates_refuse_new_bytes_without_losing_progress() {
        let (root, mut store, binding) = setup("capacity");
        let needed = FREE_SPACE_RESERVE + 65548 + crate::n64::SAVE_IMAGE_BYTES as u64 + 1;
        assert!(publish(&root, &binding, &store, b"a", needed - 1).is_err());
        assert!(!root.join("n64-checkpoints").exists());
        let a = publish(&root, &binding, &store, b"a", needed).unwrap();
        let b = publish(&root, &binding, &store, b"b", u64::MAX).unwrap();
        let pending = root.join("n64-checkpoints/interrupted.ntx-pending");
        fs::write(&pending, b"keep uncertain bytes").unwrap();
        assert!(publish(&root, &binding, &store, b"c", u64::MAX).is_err());
        assert_eq!(count(&root), 3);
        assert_eq!(publish(&root, &binding, &store, b"a", 0).unwrap(), a);
        assert!(retire_ordinary_unreferenced(&root, &binding, &mut store).is_err());
        assert!(path(&root, &a).unwrap().exists() && path(&root, &b).unwrap().exists());
        assert_eq!(fs::read(pending).unwrap(), b"keep uncertain bytes");
        assert_eq!(
            store.load().unwrap().unwrap(),
            vec![0; crate::n64::SAVE_IMAGE_BYTES]
        );
        assert!(admit_capture(&root, LIMIT as usize + 1, u64::MAX).is_err());
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn retained_scenes_pin_ordinary_history_until_acknowledged_retirement() {
        let (root, mut store, binding) = setup("ordinary");
        let old_generation = store.generation().unwrap().clone();
        let old = publish(&root, &binding, &store, b"old scene", u64::MAX).unwrap();
        let mut image = vec![0; crate::n64::SAVE_IMAGE_BYTES];
        image[0x800] = 7;
        let new_generation = store.commit(&image).unwrap();
        let new = publish(&root, &binding, &store, b"new scene", u64::MAX).unwrap();
        retire_ordinary_unreferenced(&root, &binding, &mut store).unwrap();
        let old_image = root
            .join("N64")
            .join(format!("{}.m64ram", old_generation.image_sha256));
        assert!(old_image.exists());
        retire_previous(&root, &old, &new, &binding, &store).unwrap();
        retire_ordinary_unreferenced(&root, &binding, &mut store).unwrap();
        assert!(!old_image.exists());
        assert_eq!(store.generation(), Some(&new_generation));
        assert_eq!(store.load().unwrap().unwrap(), image);
        // An invalid remaining candidate blocks history cleanup rather than guessing.
        fs::write(path(&root, &new).unwrap(), b"damaged").unwrap();
        assert!(discard_successful(&root, &new).is_err());
        assert!(retire_ordinary_unreferenced(&root, &binding, &mut store).is_err());
        assert!(path(&root, &new).unwrap().exists());
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn stale_scene_and_wrong_game_cannot_restore_over_newer_progress() {
        let root = std::env::temp_dir().join(format!("ntx-n64-scene-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let mut store = SaveStore::open(&root).unwrap();
        let mut ordinary = vec![0; crate::n64::SAVE_IMAGE_BYTES];
        store.commit(&ordinary).unwrap();
        let mut binding = Binding {
            game_id: "1234567890abcdef".repeat(2),
            profile_id: "abcdef1234567890".repeat(2),
            media_sha256: "a".repeat(64),
            core_sha256: "b".repeat(64),
            options_sha256: "c".repeat(64),
            architecture: "aarch64-android-gles3-mupen-state1".into(),
        };
        let id = publish(&root, &binding, &store, b"retained native state", u64::MAX).unwrap();
        let scene = load(&root, &id, &binding, &store).unwrap();
        ordinary[0] = 9;
        store.commit(&ordinary).unwrap();
        assert!(publish(&root, &binding, &store, b"new scene cannot fit", 0).is_err());
        assert!(scene.verify_current(&store).is_err());
        assert!(load(&root, &id, &binding, &store).is_err());
        binding.game_id = "b".repeat(32);
        assert!(load(&root, &id, &binding, &store).is_err());
        assert_eq!(store.load().unwrap().unwrap()[0], 9);
        assert!(path(&root, &id).unwrap().exists());
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}
