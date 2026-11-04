// SPDX-License-Identifier: MIT
//! Collection-wide machine-resume admission, separate from ordinary game saves.
//! Observations are not reservations. The foreground owner serializes captures;
//! additional concurrent writers will require reservations at this boundary.
use crate::{
    backend::{Backend, System},
    library::{Collection, SavedSession},
};
use std::{ffi::OsString, fs, io, path::Path};

pub const COLLECTION_BUDGET: u64 = 1024 * 1024 * 1024;
const MAX_PROFILES: usize = 8192;
const MAX_FILES: u32 = 32768;

#[derive(Clone, Copy, Debug)]
pub struct Capacity {
    pub available_bytes: u64,
    pub remaining_resume_bytes: u64,
}
impl Capacity {
    pub fn observe(root: &Path, available_bytes: u64) -> io::Result<Self> {
        Ok(Self {
            available_bytes,
            remaining_resume_bytes: COLLECTION_BUDGET.saturating_sub(inventory(root)?.bytes),
        })
    }
    pub fn admit_new_file(self, maximum_bytes: u64) -> io::Result<()> {
        if maximum_bytes > self.remaining_resume_bytes {
            return Err(io::Error::other(
                "Resume storage is full across your games. In-game saves and existing places are kept. Free resume space before retrying.",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct Inventory {
    pub bytes: u64,
    pub files: u32,
    pub profiles: Vec<ProfileUsage>,
}
#[derive(Debug)]
pub struct ProfileUsage {
    pub name: OsString,
    pub bytes: u64,
    pub files: u32,
}

/// A confirmation names the exact stored place the user saw, never an index or
/// a backend instance. Replacing it while a dialog is open invalidates the choice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredPlace {
    game_id: String,
    profile_id: String,
    checkpoint: String,
    media_sha256: String,
    system: System,
    backend: Backend,
}
impl StoredPlace {
    pub fn for_entry(entry: &SavedSession) -> Option<Self> {
        if entry.stopped || entry.restart_required || entry.runtime_incomplete {
            return None;
        }
        Some(Self {
            game_id: entry.game_id.0.clone(),
            profile_id: entry.profile_id.0.clone(),
            checkpoint: entry.checkpoint.clone()?,
            media_sha256: entry.media_sha256.clone()?,
            system: entry.system,
            backend: entry.backend,
        })
    }
}

/// Explicitly close one unloaded stored place, keeping the game and all ordinary
/// progress. The app owns its collection writer throughout this transaction.
/// ProfileLock refuses live/uncertain owners. Publish metadata before unlinking;
/// a failed or uncertain publication keeps both possible references reachable.
/// A successful result may report a cleanup warning: metadata is durable, but
/// retained file cleanup failed. Never roll metadata back after that boundary.
pub fn release_place(
    root: &Path,
    collection: &mut Collection,
    expected: &StoredPlace,
    mut publish: impl FnMut(&Collection) -> io::Result<()>,
) -> io::Result<Option<String>> {
    collection.validate().map_err(io::Error::other)?;
    let index = collection
        .resolve_game(&expected.game_id)
        .map_err(io::Error::other)?;
    if StoredPlace::for_entry(&collection.entries[index]).as_ref() != Some(expected) {
        return Err(io::Error::other(
            "This game's stored place changed. Review it again; the current place is kept.",
        ));
    }
    let profiles = root.join("profiles");
    let profile = profiles.join(&expected.profile_id);
    if !directory(root)? || !directory(&profiles)? || !directory(&profile)? {
        return Err(io::Error::other(
            "This game's resume data is unavailable; nothing was removed.",
        ));
    }
    let _writer = crate::ProfileLock::acquire(&profile)?;
    let (game, owner, media) = match (expected.system, expected.backend) {
        (System::GameCube, Backend::Dolphin) => {
            let binding = crate::checkpoint::inspect_retained(&profile, &expected.checkpoint)?;
            (binding.game_id, binding.profile_id, binding.media_sha256)
        }
        (System::Nintendo64, Backend::Mupen64PlusNext) => {
            let binding = crate::n64_checkpoint::inspect_retained(&profile, &expected.checkpoint)?;
            (binding.game_id, binding.profile_id, binding.media_sha256)
        }
        _ => {
            return Err(io::Error::other(
                "Unsupported stored place; originals retained",
            ));
        }
    };
    if game != expected.game_id || owner != expected.profile_id || media != expected.media_sha256 {
        return Err(io::Error::other(
            "Stored place ownership differs; originals retained",
        ));
    }
    let earlier = collection.entries[index].clone();
    let entry = &mut collection.entries[index];
    entry.checkpoint = None;
    entry.paused = true;
    entry.stopped = true;
    if let Err(error) = publish(collection) {
        collection.entries[index] = earlier;
        return Err(error);
    }
    let discarded = match expected.system {
        System::GameCube => crate::checkpoint::discard_successful(&profile, &expected.checkpoint),
        System::Nintendo64 => {
            crate::n64_checkpoint::discard_successful(&profile, &expected.checkpoint)
        }
    };
    Ok(discarded.err().map(|_| "The stored place is closed, but its recovery file could not be removed. In-game saves are kept.".into()))
}

fn directory(path: &Path) -> io::Result<bool> {
    match path.symlink_metadata() {
        Ok(info) if info.is_dir() => Ok(true),
        Ok(_) => Err(io::Error::other(
            "Unrecognized resume storage entry; original data retained",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Count every regular file in both known machine directories, including
/// incomplete, legacy and orphaned candidates. Never traverse a symlink, scan
/// media/ordinary saves, or decide an unreferenced file is disposable.
pub fn inventory(root: &Path) -> io::Result<Inventory> {
    let mut inventory = Inventory::default();
    if !directory(root)? {
        return Ok(inventory);
    }
    let profiles = root.join("profiles");
    if !directory(&profiles)? {
        return Ok(inventory);
    }
    for entry in fs::read_dir(profiles)? {
        let entry = entry?;
        if inventory.profiles.len() >= MAX_PROFILES || !directory(&entry.path())? {
            return Err(io::Error::other(
                "Resume storage inventory exceeds its bounds",
            ));
        }
        let mut usage = ProfileUsage {
            name: entry.file_name(),
            bytes: 0,
            files: 0,
        };
        for name in ["checkpoints", "n64-checkpoints"] {
            let path = entry.path().join(name);
            if !directory(&path)? {
                continue;
            }
            for candidate in fs::read_dir(path)? {
                let info = candidate?.path().symlink_metadata()?;
                if !info.is_file() || inventory.files >= MAX_FILES {
                    return Err(io::Error::other(
                        "Unrecognized or excessive resume recovery data; originals retained",
                    ));
                }
                inventory.bytes = inventory
                    .bytes
                    .checked_add(info.len())
                    .ok_or_else(|| io::Error::other("Resume storage size is out of range"))?;
                usage.bytes = usage
                    .bytes
                    .checked_add(info.len())
                    .ok_or_else(|| io::Error::other("Resume storage size is out of range"))?;
                inventory.files += 1;
                usage.files += 1;
            }
        }
        inventory.profiles.push(usage);
    }
    Ok(inventory)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs::File,
        os::unix::fs::symlink,
        time::{SystemTime, UNIX_EPOCH},
    };

    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new(name: &str) -> Self {
            Self(std::env::temp_dir().join(format!(
                    "ntx-resume-{name}-{}-{}",
                    std::process::id(),
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                )))
        }
        fn file(&self, name: &str, bytes: u64) {
            let path = self.0.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            File::create(path).unwrap().set_len(bytes).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn stored_place(system: System) -> (Fixture, Collection, StoredPlace) {
        use crate::{checkpoint, n64_checkpoint, n64_save::SaveStore};
        let fixture = Fixture::new("release");
        let profile = fixture.0.join("profiles").join("b".repeat(32));
        fs::create_dir_all(&profile).unwrap();
        let capacity = Capacity {
            available_bytes: u64::MAX,
            remaining_resume_bytes: u64::MAX,
        };
        let checkpoint = match system {
            System::GameCube => {
                use std::{ffi::CString, os::unix::ffi::OsStrExt};
                fs::create_dir(profile.join("GC")).unwrap();
                let _lock = crate::ProfileLock::acquire(&profile).unwrap();
                let _writer = crate::storage::CardStore::bind(&profile).unwrap();
                let write = |name: &str, bytes: &[u8]| {
                    let path = CString::new(profile.join(name).as_os_str().as_bytes()).unwrap();
                    // SAFETY: the test retains its writer, path and borrowed data.
                    assert_eq!(
                        unsafe {
                            crate::storage::write_card(path.as_ptr(), bytes.as_ptr(), bytes.len())
                        },
                        1
                    );
                };
                write("GC/MemoryCardA.fixture.raw", b"first in-game save");
                write("GC/SRAM.raw", b"console identity");
                let binding = checkpoint::Binding {
                    game_id: "a".repeat(32),
                    profile_id: "b".repeat(32),
                    media_sha256: "d".repeat(64),
                    engine: "c".repeat(64),
                    state_abi: 1,
                    resources: "original fixture".into(),
                    disc: 0,
                };
                let id = checkpoint::publish(
                    &profile,
                    &binding,
                    true,
                    b"old unsaved position",
                    capacity,
                )
                .unwrap();
                write("GC/MemoryCardA.fixture.raw", b"latest in-game save");
                assert!(checkpoint::load(&profile, &id, &binding).is_err());
                id
            }
            System::Nintendo64 => {
                let mut store = SaveStore::open(&profile).unwrap();
                store
                    .commit(&vec![0; crate::n64::SAVE_IMAGE_BYTES])
                    .unwrap();
                let binding = n64_checkpoint::Binding {
                    game_id: "a".repeat(32),
                    profile_id: "b".repeat(32),
                    media_sha256: "d".repeat(64),
                    core_sha256: "c".repeat(64),
                    options_sha256: "e".repeat(64),
                    architecture: "aarch64-android-gles3-mupen-state1".into(),
                };
                let id = n64_checkpoint::publish(
                    &profile,
                    &binding,
                    &store,
                    b"old unsaved position",
                    capacity,
                )
                .unwrap();
                store
                    .commit(&vec![1; crate::n64::SAVE_IMAGE_BYTES])
                    .unwrap();
                assert!(n64_checkpoint::load(&profile, &id, &binding, &store).is_err());
                id
            }
        };
        let collection = Collection {
            schema_version: 2,
            selected: None,
            entries: vec![SavedSession {
                game_id: crate::library::GameId("a".repeat(32)),
                profile_id: crate::library::SaveProfileId("b".repeat(32)),
                system,
                backend: match system {
                    System::GameCube => Backend::Dolphin,
                    System::Nintendo64 => Backend::Mupen64PlusNext,
                },
                media_sha256: Some("d".repeat(64)),
                uri: "fixture://retained-game".into(),
                title: "Original game".into(),
                media_relink_required: false,
                paused: true,
                last_error: Some("retained storage warning".into()),
                restart_required: false,
                runtime_incomplete: false,
                stopped: false,
                checkpoint: Some(checkpoint),
                shortcut_requested: false,
                shortcut_pinned: None,
            }],
        };
        let place = StoredPlace::for_entry(&collection.entries[0]).unwrap();
        crate::storage::atomic_replace(
            &fixture.0.join("collection.json"),
            &serde_json::to_vec(&collection).unwrap(),
        )
        .unwrap();
        (fixture, collection, place)
    }
    fn scene_path(fixture: &Fixture, place: &StoredPlace) -> std::path::PathBuf {
        fixture
            .0
            .join("profiles")
            .join(&place.profile_id)
            .join(match place.system {
                System::GameCube => "checkpoints",
                System::Nintendo64 => "n64-checkpoints",
            })
            .join(format!("{}.ntx", place.checkpoint))
    }
    fn ordinary_files(path: &Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
        let mut files = Vec::new();
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                if !["checkpoints", "n64-checkpoints"]
                    .contains(&entry.file_name().to_str().unwrap())
                {
                    files.extend(ordinary_files(&entry.path()));
                }
            } else {
                files.push((entry.path(), fs::read(entry.path()).unwrap()));
            }
        }
        files
    }
    #[test]
    fn confirmed_release_preserves_newer_saves_identities_errors_and_unknown_candidates_for_both_systems()
     {
        let _serial = crate::storage::STORE_TEST_LOCK.lock().unwrap();
        for system in [System::GameCube, System::Nintendo64] {
            let (fixture, mut collection, place) = stored_place(system);
            let scene = scene_path(&fixture, &place);
            let unknown = scene
                .parent()
                .unwrap()
                .join("retained-uncertain.ntx-pending");
            fs::write(&unknown, b"unique recovery data").unwrap();
            let ordinary = ordinary_files(&fixture.0.join("profiles"));
            assert_eq!(
                release_place(&fixture.0, &mut collection, &place, |value| {
                    assert!(scene.is_file(), "publish before unlink");
                    assert!(
                        crate::ProfileLock::acquire(scene.parent().unwrap().parent().unwrap())
                            .is_err(),
                        "retain writer during publication"
                    );
                    crate::storage::atomic_replace(
                        &fixture.0.join("collection.json"),
                        &serde_json::to_vec(value)?,
                    )
                })
                .unwrap(),
                None
            );
            let disk =
                Collection::decode(&fs::read(fixture.0.join("collection.json")).unwrap()).unwrap();
            assert!(disk.entries[0].checkpoint.is_none());
            assert!(disk.entries[0].stopped && disk.entries[0].paused);
            assert_eq!(disk.entries[0].game_id.0, place.game_id);
            assert_eq!(disk.entries[0].profile_id.0, place.profile_id);
            assert_eq!(
                disk.entries[0].last_error.as_deref(),
                Some("retained storage warning")
            );
            assert!(!scene.exists());
            assert_eq!(fs::read(unknown).unwrap(), b"unique recovery data");
            for (path, bytes) in ordinary {
                assert_eq!(fs::read(path).unwrap(), bytes);
            }
        }
    }
    #[test]
    fn aggregate_observation_refuses_both_native_publication_boundaries_after_ordinary_commit() {
        let _serial = crate::storage::STORE_TEST_LOCK.lock().unwrap();
        for system in [System::GameCube, System::Nintendo64] {
            let (fixture, collection, place) = stored_place(system);
            let profile = fixture.0.join("profiles").join(&place.profile_id);
            let used = inventory(&fixture.0).unwrap().bytes;
            fixture.file(
                "profiles/another-game/checkpoints/retained-recovery",
                COLLECTION_BUDGET - used,
            );
            let ordinary = ordinary_files(&fixture.0.join("profiles").join(&place.profile_id));
            let capacity = Capacity::observe(&fixture.0, u64::MAX).unwrap();
            assert_eq!(capacity.remaining_resume_bytes, 0);
            let result = match system {
                System::GameCube => {
                    let binding =
                        crate::checkpoint::inspect_retained(&profile, &place.checkpoint).unwrap();
                    let _lock = crate::ProfileLock::acquire(&profile).unwrap();
                    let _writer = crate::storage::CardStore::bind(&profile).unwrap();
                    crate::checkpoint::publish_captured(
                        &profile,
                        &binding,
                        true,
                        b"latest machine",
                        Ok(capacity),
                    )
                }
                System::Nintendo64 => {
                    let binding =
                        crate::n64_checkpoint::inspect_retained(&profile, &place.checkpoint)
                            .unwrap();
                    let store = crate::n64_save::SaveStore::open(&profile).unwrap();
                    crate::n64_checkpoint::publish_captured(
                        &profile,
                        &binding,
                        &store,
                        b"latest machine",
                        Ok(capacity),
                    )
                }
            };
            assert!(matches!(
                result,
                Err(crate::lifecycle::CheckpointError::StorageUnavailable(_))
            ));
            assert_eq!(inventory(&fixture.0).unwrap().bytes, COLLECTION_BUDGET);
            assert_eq!(
                StoredPlace::for_entry(&collection.entries[0]).as_ref(),
                Some(&place)
            );
            assert!(scene_path(&fixture, &place).is_file());
            for (path, bytes) in ordinary {
                assert_eq!(fs::read(path).unwrap(), bytes);
            }
        }
    }
    #[test]
    fn freeing_another_systems_place_preserves_the_live_writer_and_admits_its_retry() {
        use crate::{lifecycle::CheckpointError, n64_checkpoint, n64_save::SaveStore};
        let _serial = crate::storage::STORE_TEST_LOCK.lock().unwrap();
        let (fixture, mut collection, stored) = stored_place(System::GameCube);
        let active_profile = fixture.0.join("profiles").join("d".repeat(32));
        let mut active_store = SaveStore::open(&active_profile).unwrap();
        active_store
            .commit(&vec![2; crate::n64::SAVE_IMAGE_BYTES])
            .unwrap();
        let active_binding = n64_checkpoint::Binding {
            game_id: "c".repeat(32),
            profile_id: "d".repeat(32),
            media_sha256: "e".repeat(64),
            core_sha256: "f".repeat(64),
            options_sha256: "0".repeat(64),
            architecture: "aarch64-android-gles3-mupen-state1".into(),
        };
        let mut active_entry = collection.entries[0].clone();
        active_entry.system = System::Nintendo64;
        active_entry.backend = Backend::Mupen64PlusNext;
        active_entry.game_id.0 = active_binding.game_id.clone();
        active_entry.profile_id.0 = active_binding.profile_id.clone();
        active_entry.media_sha256 = Some(active_binding.media_sha256.clone());
        active_entry.uri = "fixture://active-game".into();
        active_entry.title = "Active N64 game".into();
        active_entry.checkpoint = None;
        active_entry.runtime_incomplete = true;
        active_entry.last_error = Some("Resume storage full; live RAM retained".into());
        collection.selected = Some(active_binding.game_id.clone());
        collection.entries.push(active_entry);
        let active_metadata = serde_json::to_vec(&collection.entries[1]).unwrap();
        let current_ordinary = active_store.generation().cloned();
        let used = inventory(&fixture.0).unwrap().bytes;
        // Sparse recovery bytes leave less than the minimum bounded header
        // headroom, without allocating or filling the actual host filesystem.
        let orphan_length = COLLECTION_BUDGET - used - 65547;
        fixture.file(
            "profiles/orphan/checkpoints/retained-recovery",
            orphan_length,
        );
        let refused = Capacity::observe(&fixture.0, u64::MAX).unwrap();
        assert!(matches!(
            n64_checkpoint::publish_captured(
                &active_profile,
                &active_binding,
                &active_store,
                b"live machine",
                Ok(refused)
            ),
            Err(CheckpointError::StorageUnavailable(_))
        ));
        assert_eq!(
            release_place(&fixture.0, &mut collection, &stored, |value| {
                assert_eq!(
                    serde_json::to_vec(&value.entries[1]).unwrap(),
                    active_metadata
                );
                assert!(crate::ProfileLock::acquire(&active_profile).is_err());
                crate::storage::atomic_replace(
                    &fixture.0.join("collection.json"),
                    &serde_json::to_vec(value)?,
                )
            })
            .unwrap(),
            None
        );
        assert!(collection.entries[0].stopped && collection.entries[0].checkpoint.is_none());
        assert_eq!(
            serde_json::to_vec(&collection.entries[1]).unwrap(),
            active_metadata
        );
        assert!(crate::ProfileLock::acquire(&active_profile).is_err());
        assert_eq!(active_store.generation(), current_ordinary.as_ref());
        active_store.verify_current().unwrap();
        let capacity = Capacity::observe(&fixture.0, u64::MAX).unwrap();
        let id = n64_checkpoint::publish_captured(
            &active_profile,
            &active_binding,
            &active_store,
            b"live machine",
            Ok(capacity),
        )
        .unwrap();
        assert_eq!(
            n64_checkpoint::load(&active_profile, &id, &active_binding, &active_store)
                .unwrap()
                .payload,
            b"live machine"
        );
        assert_eq!(
            fs::metadata(
                fixture
                    .0
                    .join("profiles/orphan/checkpoints/retained-recovery")
            )
            .unwrap()
            .len(),
            orphan_length
        );
        assert_eq!(
            collection.selected.as_deref(),
            Some(active_binding.game_id.as_str())
        );
    }
    #[test]
    fn retirement_failure_after_publication_keeps_durable_closed_metadata_and_reports_retained_bytes()
     {
        let (fixture, mut collection, place) = stored_place(System::Nintendo64);
        let scene = scene_path(&fixture, &place);
        let original = scene.with_extension("preserved");
        let bytes = fs::read(&scene).unwrap();
        let warning = release_place(&fixture.0, &mut collection, &place, |value| {
            crate::storage::atomic_replace(
                &fixture.0.join("collection.json"),
                &serde_json::to_vec(value)?,
            )?;
            // Fault at the unlink boundary, after the reference is durable.
            fs::rename(&scene, &original)?;
            symlink(&original, &scene)?;
            Ok(())
        })
        .unwrap();
        assert!(warning.is_some());
        assert!(collection.entries[0].checkpoint.is_none() && collection.entries[0].stopped);
        let disk =
            Collection::decode(&fs::read(fixture.0.join("collection.json")).unwrap()).unwrap();
        assert!(disk.entries[0].checkpoint.is_none() && disk.entries[0].stopped);
        assert_eq!(fs::read(original).unwrap(), bytes);
        assert!(scene.symlink_metadata().unwrap().file_type().is_symlink());
    }
    #[test]
    fn failed_or_uncertain_reference_publication_keeps_every_possible_scene_reachable() {
        let _serial = crate::storage::STORE_TEST_LOCK.lock().unwrap();
        for system in [System::GameCube, System::Nintendo64] {
            for after_rename in [false, true] {
                let (fixture, mut collection, place) = stored_place(system);
                let scene = scene_path(&fixture, &place);
                let bytes = fs::read(&scene).unwrap();
                let before = fs::read(fixture.0.join("collection.json")).unwrap();
                assert!(
                    release_place(&fixture.0, &mut collection, &place, |value| {
                        if after_rename {
                            crate::storage::atomic_replace(
                                &fixture.0.join("collection.json"),
                                &serde_json::to_vec(value)?,
                            )?;
                        }
                        Err(io::Error::other("injected reference sync failure"))
                    })
                    .is_err()
                );
                assert_eq!(
                    StoredPlace::for_entry(&collection.entries[0]).as_ref(),
                    Some(&place)
                );
                assert_eq!(fs::read(scene).unwrap(), bytes);
                if !after_rename {
                    assert_eq!(fs::read(fixture.0.join("collection.json")).unwrap(), before);
                }
            }
        }
    }
    #[test]
    fn live_owner_stale_confirmation_quarantine_wrong_binding_and_damaged_bytes_cannot_be_released()
    {
        let (fixture, mut collection, place) = stored_place(System::Nintendo64);
        let profile = fixture.0.join("profiles").join(&place.profile_id);
        let scene = scene_path(&fixture, &place);
        let before = fs::read(&scene).unwrap();
        let writer = crate::ProfileLock::acquire(&profile).unwrap();
        let never_publish =
            |_: &Collection| -> io::Result<()> { panic!("unsafe release published metadata") };
        assert!(release_place(&fixture.0, &mut collection, &place, never_publish).is_err());
        drop(writer);
        collection.entries[0].checkpoint = Some("e".repeat(64));
        assert!(release_place(&fixture.0, &mut collection, &place, never_publish).is_err());
        collection.entries[0].checkpoint = Some(place.checkpoint.clone());
        collection.entries[0].restart_required = true;
        assert!(release_place(&fixture.0, &mut collection, &place, never_publish).is_err());
        collection.entries[0].restart_required = false;
        collection.entries[0].runtime_incomplete = true;
        assert!(StoredPlace::for_entry(&collection.entries[0]).is_none());
        collection.entries[0].runtime_incomplete = false;
        collection.entries[0].game_id.0 = "f".repeat(32);
        let wrong = StoredPlace::for_entry(&collection.entries[0]).unwrap();
        assert!(release_place(&fixture.0, &mut collection, &wrong, never_publish).is_err());
        collection.entries[0].game_id.0 = place.game_id.clone();
        let mut damaged = before.clone();
        *damaged.last_mut().unwrap() ^= 1;
        fs::write(&scene, &damaged).unwrap();
        assert!(release_place(&fixture.0, &mut collection, &place, never_publish).is_err());
        assert_eq!(fs::read(&scene).unwrap(), damaged);
        fs::write(&scene, &before).unwrap();
        fs::rename(&scene, scene.with_extension("original")).unwrap();
        symlink(scene.with_extension("original"), &scene).unwrap();
        assert!(release_place(&fixture.0, &mut collection, &place, never_publish).is_err());
        assert!(scene.symlink_metadata().unwrap().file_type().is_symlink());
    }

    #[test]
    fn independent_systems_orphans_and_pending_files_share_one_budget_without_counting_saves() {
        let fixture = Fixture::new("collection");
        assert_eq!(inventory(&fixture.0).unwrap().bytes, 0);
        fixture.file("profiles/a/checkpoints/current.ntx", COLLECTION_BUDGET / 2);
        fixture.file(
            "profiles/b/n64-checkpoints/unfinished.ntx-pending",
            COLLECTION_BUDGET / 2 - 100,
        );
        fixture.file("profiles/orphan/checkpoints/unknown.gcxstate", 100);
        fixture.file("profiles/a/GC/MemoryCardA.raw", COLLECTION_BUDGET * 2);
        fixture.file("profiles/b/n64/ordinary.img", COLLECTION_BUDGET * 2);
        fixture.file("media/game.rvz", COLLECTION_BUDGET * 2);
        let usage = inventory(&fixture.0).unwrap();
        assert_eq!(
            (usage.bytes, usage.files, usage.profiles.len()),
            (COLLECTION_BUDGET, 3, 3)
        );
        assert_eq!(
            usage
                .profiles
                .iter()
                .find(|p| p.name == "orphan")
                .unwrap()
                .bytes,
            100
        );
        let capacity = Capacity::observe(&fixture.0, u64::MAX).unwrap();
        assert_eq!(capacity.remaining_resume_bytes, 0);
        assert!(capacity.admit_new_file(1).is_err());
        // Existing oversized histories remain; inventory never trims them.
        fixture.file("profiles/orphan/n64-checkpoints/recovery", 101);
        assert_eq!(
            Capacity::observe(&fixture.0, 37)
                .unwrap()
                .remaining_resume_bytes,
            0
        );
        assert_eq!(
            inventory(&fixture.0).unwrap().bytes,
            COLLECTION_BUDGET + 101
        );
        assert_eq!(
            fs::metadata(fixture.0.join("profiles/a/GC/MemoryCardA.raw"))
                .unwrap()
                .len(),
            COLLECTION_BUDGET * 2
        );
    }

    #[test]
    fn publication_headroom_includes_the_existing_scene_and_bounded_new_container() {
        let fixture = Fixture::new("headroom");
        let needed = 16 * 1024 * 1024 + 65548;
        fixture.file(
            "profiles/other/n64-checkpoints/current.ntx",
            COLLECTION_BUDGET - needed,
        );
        let capacity = Capacity::observe(&fixture.0, 99).unwrap();
        assert_eq!(capacity.available_bytes, 99);
        assert!(capacity.admit_new_file(needed).is_ok());
        assert!(capacity.admit_new_file(needed + 1).is_err());
        fixture.file("profiles/other/checkpoints/uncertain", 1);
        assert!(
            Capacity::observe(&fixture.0, u64::MAX)
                .unwrap()
                .admit_new_file(needed)
                .is_err()
        );
        assert!(
            fixture
                .0
                .join("profiles/other/checkpoints/uncertain")
                .is_file()
        );
    }

    #[test]
    fn unsafe_entries_at_every_traversed_level_refuse_without_following_or_deleting_them() {
        for name in [
            "profiles",
            "profiles/a",
            "profiles/a/checkpoints",
            "profiles/a/n64-checkpoints",
            "profiles/a/checkpoints/unknown",
        ] {
            let fixture = Fixture::new("unsafe");
            let outside = fixture.0.join("outside");
            fs::create_dir_all(&outside).unwrap();
            fs::write(outside.join("preserved"), b"untouched").unwrap();
            let path = fixture.0.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            symlink(&outside, &path).unwrap();
            assert!(inventory(&fixture.0).is_err(), "{name}");
            assert!(path.symlink_metadata().unwrap().file_type().is_symlink());
            assert_eq!(fs::read(outside.join("preserved")).unwrap(), b"untouched");
        }
        let fixture = Fixture::new("nested");
        fs::create_dir_all(
            fixture
                .0
                .join("profiles/a/checkpoints/unknown-subdirectory"),
        )
        .unwrap();
        assert!(inventory(&fixture.0).is_err());
        let root_link = fixture.0.with_extension("link");
        symlink(&fixture.0, &root_link).unwrap();
        assert!(inventory(&root_link).is_err());
        fs::remove_file(root_link).unwrap();
    }
}
