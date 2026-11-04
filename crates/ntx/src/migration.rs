// SPDX-License-Identifier: MIT
//! Offline import of a protected, quiescent legacy export. No Android access,
//! archive extraction, source deletion, URI grants or native restore occurs here.
use crate::{ProfileLock, library::Collection, storage};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, Read, Write},
    path::Path,
};

const RECEIPT: &str = "migration.json";
const MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const MAX_FILE: u64 = 512 * 1024 * 1024;
const MAX_FILES: usize = 16384;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Image {
    length: u64,
    sha256: String,
}
type Inventory = BTreeMap<String, Image>;
#[derive(PartialEq, Eq, Serialize, Deserialize)]
struct Receipt {
    schema: u32,
    source: Inventory,
    published: Inventory,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Imported,
    AlreadyImported,
}

fn invalid(message: &str) -> io::Error {
    io::Error::other(message)
}
fn inventory(root: &Path) -> io::Result<Inventory> {
    fn walk(
        root: &Path,
        dir: &Path,
        out: &mut Inventory,
        total: &mut u64,
        depth: usize,
    ) -> io::Result<()> {
        if depth > 16 || !dir.symlink_metadata()?.is_dir() {
            return Err(invalid("unsafe export directory"));
        }
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            let relative = path.strip_prefix(root).map_err(io::Error::other)?;
            let name = relative
                .to_str()
                .ok_or_else(|| invalid("non-UTF8 export path"))?;
            let info = path.symlink_metadata()?;
            if info.file_type().is_symlink() {
                return Err(invalid("symlink in export; source retained"));
            }
            // Runtime locks never transfer. Resource copies are reconstructed
            // from the new package, independently of machine fingerprints.
            if relative == Path::new("Sys")
                || relative == Path::new(RECEIPT)
                || path.file_name().is_some_and(|n| n == "writer.lock")
            {
                continue;
            }
            if depth == 0
                && !matches!(
                    name,
                    "collection.json" | "controls.json" | "profiles" | "media-identities.json"
                )
            {
                return Err(invalid(
                    "unrecognized export root entry; preserve and reconcile",
                ));
            }
            if name.contains("pending") {
                return Err(invalid(
                    "unfinished export publication; retain recovery material",
                ));
            }
            if info.is_dir() {
                walk(root, &path, out, total, depth + 1)?;
            } else if info.is_file() {
                use std::os::unix::fs::MetadataExt;
                if info.nlink() != 1 || info.len() > MAX_FILE || out.len() >= MAX_FILES {
                    return Err(invalid("export file exceeds bounds or has hard links"));
                }
                *total = total
                    .checked_add(info.len())
                    .ok_or_else(|| invalid("export size overflow"))?;
                if *total > MAX_BYTES {
                    return Err(invalid("export exceeds migration byte bound"));
                }
                let mut hash = Sha256::new();
                let mut file = File::open(&path)?;
                let mut buffer = [0; 65536];
                let mut read = 0u64;
                loop {
                    let n = file.read(&mut buffer)?;
                    if n == 0 {
                        break;
                    }
                    read += n as u64;
                    if read > info.len() {
                        return Err(invalid("export changed during hashing"));
                    }
                    hash.update(&buffer[..n]);
                }
                if read != info.len() {
                    return Err(invalid("export truncated during hashing"));
                }
                out.insert(
                    name.into(),
                    Image {
                        length: read,
                        sha256: format!("{:x}", hash.finalize()),
                    },
                );
            } else {
                return Err(invalid("non-file export entry"));
            }
        }
        Ok(())
    }
    let mut out = Inventory::new();
    walk(root, root, &mut out, &mut 0, 0)?;
    Ok(out)
}

fn collection(root: &Path, input: &Inventory) -> io::Result<Collection> {
    let meta = input
        .get("collection.json")
        .ok_or_else(|| invalid("export has no collection"))?;
    if meta.length > 4 * 1024 * 1024 {
        return Err(invalid("collection exceeds migration bound"));
    }
    let bytes = fs::read(root.join("collection.json"))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    if value["schema_version"] != 1 {
        return Err(invalid(
            "this importer accepts only legacy schema-1 exports",
        ));
    }
    Collection::decode(&bytes).map_err(invalid_owned)
}
fn invalid_owned(message: String) -> io::Error {
    invalid(&message)
}
// A cooperating app holds the store lock, and the filesystem operation itself
// also refuses replacement, including an empty directory from another writer.
#[cfg(all(
    any(target_os = "linux", target_os = "android"),
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
fn publish_directory(source: &Path, target: &Path) -> io::Result<()> {
    use std::{
        ffi::{CString, c_long},
        os::unix::ffi::OsStrExt,
    };
    unsafe extern "C" {
        fn syscall(number: c_long, ...) -> c_long;
    }
    #[cfg(target_arch = "x86_64")]
    const RENAMEAT2: c_long = 316;
    #[cfg(target_arch = "aarch64")]
    const RENAMEAT2: c_long = 276;
    let source = CString::new(source.as_os_str().as_bytes())?;
    let target = CString::new(target.as_os_str().as_bytes())?;
    // SAFETY: two live NUL-terminated paths; AT_FDCWD and RENAME_NOREPLACE.
    if unsafe {
        syscall(
            RENAMEAT2,
            -100i32,
            source.as_ptr(),
            -100i32,
            target.as_ptr(),
            1u32,
        )
    } == 0
    {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
#[cfg(not(all(
    any(target_os = "linux", target_os = "android"),
    any(target_arch = "x86_64", target_arch = "aarch64")
)))]
fn publish_directory(_source: &Path, _target: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic directory import is not implemented for this host",
    ))
}
fn sync_tree(dir: &Path) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            sync_tree(&path)?;
        }
    }
    File::open(dir)?.sync_all()
}

/// The device procedure must first stop/retire the source safely and protect its
/// export. Call only off-device, then transfer under separately admitted custody.
/// The target parent is private and must be on its final filesystem. The app and
/// publisher both hold ntx-store.lock, including through durable publication.
/// Existing targets are accepted only if EVERY imported byte is still identical.
pub fn import_export(source: &Path, target_parent: &Path) -> io::Result<Outcome> {
    if !source.symlink_metadata()?.is_dir() || !target_parent.symlink_metadata()?.is_dir() {
        return Err(invalid("migration roots must be ordinary directories"));
    }
    let source = source.canonicalize()?;
    let parent = target_parent.canonicalize()?;
    let target = parent.join("ntx");
    if source.starts_with(&target) || target.starts_with(&source) {
        return Err(invalid("source and target must be disjoint"));
    }
    let _target_writer = ProfileLock::acquire_file(&parent.join("ntx-store.lock"))?;
    let before = inventory(&source)?;
    let mut model = collection(&source, &before)?;
    if before.contains_key("controls.json") {
        crate::controls::Settings::load(&source)?;
    }
    let media: BTreeMap<String, String> = if let Some(info) = before.get("media-identities.json") {
        if info.length > 1024 * 1024 {
            return Err(invalid("media identity export exceeds bound"));
        }
        serde_json::from_slice(&fs::read(source.join("media-identities.json"))?)?
    } else {
        BTreeMap::new()
    };
    if media.iter().any(|(id, digest)| {
        !storage::valid_digest(digest) || !model.entries.iter().any(|e| e.game_id.0 == *id)
    }) {
        return Err(invalid("invalid exported media identity"));
    }
    let mut writers = Vec::new();
    let profiles = source.join("profiles");
    if profiles.try_exists()? {
        for entry in fs::read_dir(&profiles)? {
            let path = entry?.path();
            let name = path
                .file_name()
                .and_then(|v| v.to_str())
                .ok_or_else(|| invalid("invalid profile"))?;
            if !crate::library::valid_id(name) || !path.symlink_metadata()?.is_dir() {
                return Err(invalid("invalid profile directory"));
            }
            writers.push(ProfileLock::acquire(&path)?);
            if path.join("save-generation.json").try_exists()? {
                storage::SaveGeneration::verify_retained(&path)?;
            } else if path.join("GC").try_exists()? {
                return Err(invalid(
                    "ordinary data lacks a verified generation; source retained",
                ));
            }
        }
    }
    for game in &mut model.entries {
        game.media_sha256 = media.get(&game.game_id.0).cloned();
        if let Some(id) = &game.checkpoint {
            let binding =
                crate::checkpoint::inspect_retained(&profiles.join(&game.profile_id.0), id)?;
            if binding.game_id != game.game_id.0 || binding.profile_id != game.profile_id.0 {
                return Err(invalid("checkpoint belongs to a different game/profile"));
            }
            if game
                .media_sha256
                .as_ref()
                .is_some_and(|s| *s != binding.media_sha256)
            {
                return Err(invalid("exported media differs from retained checkpoint"));
            }
            game.media_sha256 = Some(binding.media_sha256);
            // Retain the reference and exact bytes. A new engine is NOT entitled
            // to rewrite this fingerprint or silently cold boot unsaved play.
            let mut message = "Retained legacy scene requires its original compatible runtime. Keep the recovery candidate, or explicitly restart with current saves after relinking media.".to_owned();
            if let Some(earlier) = &game.last_error {
                message.push_str("\nEarlier retained error: ");
                message.push_str(earlier);
            }
            game.fail(message);
        }
        game.recover_interrupted();
        game.media_relink_required = true;
        game.shortcut_pinned = None;
    }
    let transformed = serde_json::to_vec(&model)?;
    let mut published = before.clone();
    published.insert(
        "collection.json".into(),
        Image {
            length: transformed.len() as u64,
            sha256: storage::digest(&transformed),
        },
    );
    let receipt = Receipt {
        schema: 1,
        source: before.clone(),
        published,
    };
    if let Ok(info) = target.symlink_metadata() {
        if !info.is_dir() {
            return Err(invalid("existing target is not an ordinary directory"));
        }
        let receipt_path = target.join(RECEIPT);
        let info = receipt_path.symlink_metadata()?;
        if !info.is_file() || info.len() > 8 * 1024 * 1024 {
            return Err(invalid("invalid migration receipt"));
        }
        let old: Receipt = serde_json::from_slice(&fs::read(receipt_path)?)?;
        if old == receipt
            && inventory(&target)? == receipt.published
            && inventory(&source)? == before
        {
            File::open(&parent)?.sync_all()?;
            return Ok(Outcome::AlreadyImported);
        }
        return Err(invalid(
            "target contains different/newer data; no overwrite permitted",
        ));
    }
    // An interrupted stage is retained for inspection. Retry uses a new stage;
    // no recursive deletion or guessed recovery of partial state is performed.
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let mut random = [0; 16];
    File::open("/dev/urandom")?.read_exact(&mut random)?;
    let stage = parent.join(format!(".ntx-stage-{}", storage::digest(&random)));
    fs::DirBuilder::new().mode(0o700).create(&stage)?;
    for (name, image) in &before {
        let destination = stage.join(name);
        fs::create_dir_all(
            destination
                .parent()
                .ok_or_else(|| invalid("missing staged parent"))?,
        )?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(destination)?;
        if name == "collection.json" {
            output.write_all(&transformed)?;
        } else {
            let copied = io::copy(
                &mut File::open(source.join(name))?.take(image.length + 1),
                &mut output,
            )?;
            if copied != image.length {
                return Err(invalid(
                    "source changed during bounded copy; stage retained",
                ));
            }
        }
        output.sync_all()?;
    }
    if inventory(&stage)? != receipt.published || inventory(&source)? != before {
        return Err(invalid(
            "migration readback/source changed; stage and source retained",
        ));
    }
    storage::atomic_replace(&stage.join(RECEIPT), &serde_json::to_vec(&receipt)?)?;
    sync_tree(&stage)?;
    if target.try_exists()? {
        return Err(invalid("target appeared during migration; no overwrite"));
    }
    publish_directory(&stage, &target)?;
    File::open(&parent)?.sync_all()?;
    drop(writers);
    Ok(Outcome::Imported)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Fixture {
        root: std::path::PathBuf,
        source: std::path::PathBuf,
        parent: std::path::PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "ntx-migration-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            let source = root.join("export");
            let parent = root.join("destination");
            fs::create_dir(&source).unwrap();
            fs::create_dir(&parent).unwrap();
            let collection = serde_json::json!({"schema_version":1,"selected":"a".repeat(32),"entries":[{
                "game_id":"a".repeat(32),"profile_id":"b".repeat(32),"uri":"content://fixture/game",
                "title":"Fixture", "paused":true, "checkpoint":null
            }]});
            fs::write(
                source.join("collection.json"),
                serde_json::to_vec(&collection).unwrap(),
            )
            .unwrap();
            Self {
                root,
                source,
                parent,
            }
        }
        fn profile(&self) -> std::path::PathBuf {
            let profile = self.source.join("profiles").join("b".repeat(32));
            fs::create_dir_all(profile.join("GC")).unwrap();
            fs::write(profile.join("GC/SRAM.raw"), [0; 68]).unwrap();
            fs::write(
                profile.join("GC/MemoryCardA.USA.raw"),
                b"original test-only ordinary data",
            )
            .unwrap();
            let mut images = BTreeMap::new();
            for path in ["GC/SRAM.raw", "GC/MemoryCardA.USA.raw"] {
                images.insert(
                    path,
                    storage::digest(&fs::read(profile.join(path)).unwrap()),
                );
            }
            fs::write(
                profile.join("save-generation.json"),
                serde_json::to_vec(
                    &serde_json::json!({"schema":1,"number":7,"images":images,"pending":null}),
                )
                .unwrap(),
            )
            .unwrap();
            profile
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }
    #[test]
    fn imports_without_reidentification_and_retry_refuses_newer_target_progress() {
        let f = Fixture::new();
        f.profile();
        let source = inventory(&f.source).unwrap();
        assert_eq!(
            import_export(&f.source, &f.parent).unwrap(),
            Outcome::Imported
        );
        let target = f.parent.join("ntx");
        let model = Collection::decode(&fs::read(target.join("collection.json")).unwrap()).unwrap();
        assert_eq!(model.schema_version, 2);
        assert_eq!(model.entries[0].game_id.0, "a".repeat(32));
        assert_eq!(model.entries[0].profile_id.0, "b".repeat(32));
        assert!(model.entries[0].media_relink_required && !model.entries[0].boot_allowed());
        assert_eq!(model.entries[0].system, crate::backend::System::GameCube);
        assert_eq!(model.entries[0].backend, crate::backend::Backend::Dolphin);
        assert_eq!(source, inventory(&f.source).unwrap());
        assert_eq!(
            import_export(&f.source, &f.parent).unwrap(),
            Outcome::AlreadyImported
        );
        let card = target
            .join("profiles")
            .join("b".repeat(32))
            .join("GC/MemoryCardA.USA.raw");
        fs::write(&card, b"newer committed target progress").unwrap();
        assert!(import_export(&f.source, &f.parent).is_err());
        assert_eq!(fs::read(card).unwrap(), b"newer committed target progress");
        assert_eq!(source, inventory(&f.source).unwrap());
    }
    #[test]
    fn checkpoint_bytes_and_fingerprints_survive_as_explicit_recovery() {
        let f = Fixture::new();
        let profile = f.profile();
        let binding = crate::checkpoint::Binding {
            game_id: "a".repeat(32),
            profile_id: "b".repeat(32),
            engine: "c".repeat(64),
            state_abi: 1,
            resources: "retained-fixture-resources".into(),
            media_sha256: "d".repeat(64),
            disc: 0,
        };
        let _serial = storage::STORE_TEST_LOCK.lock().unwrap();
        let native_store = storage::CardStore::bind(&profile).unwrap();
        let id = crate::checkpoint::publish(
            &profile,
            &binding,
            true,
            b"unsaved test-only scene",
            u64::MAX,
        )
        .unwrap();
        drop(native_store);
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(f.source.join("collection.json")).unwrap()).unwrap();
        value["entries"][0]["checkpoint"] = serde_json::json!(id);
        fs::write(
            f.source.join("collection.json"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
        let original = fs::read(profile.join("checkpoints").join(format!("{id}.ntx"))).unwrap();
        assert_eq!(
            import_export(&f.source, &f.parent).unwrap(),
            Outcome::Imported
        );
        let target = f.parent.join("ntx");
        let model = Collection::decode(&fs::read(target.join("collection.json")).unwrap()).unwrap();
        assert_eq!(model.entries[0].checkpoint.as_ref(), Some(&id));
        assert!(model.entries[0].restart_required && model.entries[0].paused);
        let target_profile = target.join("profiles").join("b".repeat(32));
        assert_eq!(
            crate::checkpoint::inspect_retained(&target_profile, &id).unwrap(),
            binding
        );
        assert_eq!(
            fs::read(target_profile.join("checkpoints").join(format!("{id}.ntx"))).unwrap(),
            original
        );
    }
    #[test]
    fn busy_writers_partial_generations_symlinks_and_existing_targets_refuse() {
        let f = Fixture::new();
        let profile = f.profile();
        let writer = ProfileLock::acquire(&profile).unwrap();
        assert!(import_export(&f.source, &f.parent).is_err());
        drop(writer);
        fs::write(profile.join("GC/SRAM.raw"), b"changed after generation").unwrap();
        assert!(import_export(&f.source, &f.parent).is_err());
        assert!(!f.parent.join("ntx").exists());
        let f = Fixture::new();
        std::os::unix::fs::symlink("/tmp", f.source.join("profiles")).unwrap();
        assert!(import_export(&f.source, &f.parent).is_err());
        let f = Fixture::new();
        fs::create_dir(f.parent.join("ntx")).unwrap();
        fs::write(f.parent.join("ntx/collection.json"), b"existing target").unwrap();
        assert!(import_export(&f.source, &f.parent).is_err());
        assert_eq!(
            fs::read(f.parent.join("ntx/collection.json")).unwrap(),
            b"existing target"
        );
    }
    #[test]
    fn interrupted_stage_is_retained_and_never_treated_as_published_data() {
        let f = Fixture::new();
        fs::create_dir(f.parent.join(".ntx-stage-interrupted")).unwrap();
        fs::write(
            f.parent.join(".ntx-stage-interrupted/partial"),
            b"retain me",
        )
        .unwrap();
        assert_eq!(
            import_export(&f.source, &f.parent).unwrap(),
            Outcome::Imported
        );
        assert_eq!(
            fs::read(f.parent.join(".ntx-stage-interrupted/partial")).unwrap(),
            b"retain me"
        );
    }
    #[test]
    fn atomic_publication_refuses_even_an_empty_existing_destination() {
        let f = Fixture::new();
        let stage = f.parent.join("stage");
        let target = f.parent.join("ntx");
        fs::create_dir(&stage).unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(stage.join("protected"), b"retained stage").unwrap();
        assert!(publish_directory(&stage, &target).is_err());
        assert!(target.read_dir().unwrap().next().is_none());
        assert_eq!(
            fs::read(stage.join("protected")).unwrap(),
            b"retained stage"
        );
    }
    #[test]
    fn private_incoming_sibling_imports_without_recursive_source_target_overlap() {
        let f = Fixture::new();
        let incoming = f.parent.join("ntx-import");
        fs::rename(&f.source, &incoming).unwrap();
        assert_eq!(
            import_export(&incoming, &f.parent).unwrap(),
            Outcome::Imported
        );
        assert!(incoming.join("collection.json").is_file());
        assert!(f.parent.join("ntx/migration.json").is_file());
        assert!(import_export(&f.parent.join("ntx"), &f.parent).is_err());
    }
}
