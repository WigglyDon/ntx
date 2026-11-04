// SPDX-License-Identifier: MIT
//! Private complete machine generations. Ordinary saves remain authoritative.
//! Callers hold ProfileLock and keep native execution paused until publication.
use crate::{
    library::valid_id,
    resume_storage::Capacity,
    storage::{self, SaveGeneration},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};

const MAGIC: &[u8; 8] = b"NTXCP001";
const HEADER_LIMIT: usize = 65536;
// Safety ceiling for this private format, not a promised device allocation.
// Record actual native payload sizes; an oversized capture retains the runtime.
pub const PAYLOAD_LIMIT: usize = 256 * 1024 * 1024;
/// Include incomplete and unknown recovery candidates; never evict one to fit.
pub const MAX_RETAINED_FILES: usize = 3;
pub const FREE_SPACE_RESERVE: u64 = 64 * 1024 * 1024;
const FILE_LIMIT: u64 = PAYLOAD_LIMIT as u64 + HEADER_LIMIT as u64 + 12;

/// Admission is an observation, not reserved space. Ordinary cards have already
/// flushed before native capture calls the writer; publication may still fail.
pub fn admit_capture(profile: &Path, payload_bytes: usize, available: Capacity) -> io::Result<()> {
    if payload_bytes == 0 || payload_bytes > PAYLOAD_LIMIT {
        return Err(io::Error::other("invalid GameCube scene size"));
    }
    let needed = payload_bytes as u64 + HEADER_LIMIT as u64 + 12;
    if available.available_bytes < needed + FREE_SPACE_RESERVE {
        return Err(io::Error::other(
            "Not enough free space to keep your current place. In-game saves and previous recovery data are kept.",
        ));
    }
    available.admit_new_file(payload_bytes as u64 + 65548)?;
    let entries = match fs::read_dir(directory(profile)?) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let mut count = 0;
    let mut bytes = 0u64;
    for entry in entries {
        let info = entry?.path().symlink_metadata()?;
        if !info.is_file() {
            return Err(io::Error::other(
                "Unrecognized GameCube recovery entry; original data retained",
            ));
        }
        count += 1;
        bytes = bytes
            .checked_add(info.len())
            .ok_or_else(|| io::Error::other("GameCube resume storage exceeds its budget"))?;
        if count >= MAX_RETAINED_FILES
            || bytes
                .checked_add(needed)
                .is_none_or(|total| total > MAX_RETAINED_FILES as u64 * FILE_LIMIT)
        {
            return Err(io::Error::other(
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
    pub engine: String,
    pub state_abi: u32,
    pub resources: String,
    pub media_sha256: String,
    pub disc: u8,
}
impl Binding {
    fn validate(&self) -> io::Result<()> {
        if !valid_id(&self.game_id)
            || !valid_id(&self.profile_id)
            || !storage::valid_digest(&self.engine)
            || !storage::valid_digest(&self.media_sha256)
            || self.state_abi != 1
            || self.resources.is_empty()
            || self.resources.len() > 4096
        {
            return Err(io::Error::other(
                "unsupported checkpoint binding; current saves retained",
            ));
        }
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
struct Metadata {
    binding: Binding,
    generation: SaveGeneration,
    paused: bool,
    payload_sha256: String,
    payload_length: usize,
}
pub struct Checkpoint {
    pub payload: Vec<u8>,
    pub paused: bool,
    generation: SaveGeneration,
}
impl Checkpoint {
    /// Recheck immediately before native mutation, including after paused boot.
    pub fn verify_current_saves(&self, profile: &Path) -> io::Result<()> {
        self.generation.verify(profile)
    }
}
fn directory(profile: &Path) -> io::Result<PathBuf> {
    let path = profile.join("checkpoints");
    if path
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
    {
        return Err(io::Error::other("checkpoint directory is a symlink"));
    }
    Ok(path)
}
fn path(profile: &Path, id: &str) -> io::Result<PathBuf> {
    if !storage::valid_digest(id) {
        return Err(io::Error::other("invalid checkpoint identity"));
    }
    Ok(directory(profile)?.join(format!("{id}.ntx")))
}
fn existing_path(profile: &Path, id: &str) -> io::Result<PathBuf> {
    let canonical = path(profile, id)?;
    match canonical.symlink_metadata() {
        Ok(_) => Ok(canonical),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            Ok(directory(profile)?.join(format!("{id}.{}", crate::legacy::CHECKPOINT_EXTENSION)))
        }
        Err(e) => Err(e),
    }
}
fn parts_digest(parts: &[&[u8]]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update(part);
    }
    format!("{:x}", hash.finalize())
}
/// Stream a selected media descriptor without copying the game image.
pub fn media_digest(mut file: impl Read) -> io::Result<String> {
    let mut hash = Sha256::new();
    let mut bytes = [0; 65536];
    loop {
        let n = file.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
/// Native invokes its writer only after complete capture and card/SRAM flush.
/// A storage-only refusal is resumable while that exact committed generation
/// still verifies. This does not acknowledge a durable session reference.
pub fn publish_captured(
    profile: &Path,
    binding: &Binding,
    paused: bool,
    payload: &[u8],
    available: io::Result<Capacity>,
) -> Result<String, crate::lifecycle::CheckpointError> {
    use crate::lifecycle::CheckpointError;
    binding.validate()?;
    if payload.is_empty() || payload.len() > PAYLOAD_LIMIT {
        return Err(CheckpointError::RecoveryRequired(
            "GameCube capture has no valid complete machine payload".into(),
        ));
    }
    let generation = storage::save_generation(profile)?;
    validate_generation(&generation)?;
    match available.and_then(|space| publish(profile, binding, paused, payload, space)) {
        Ok(id) => Ok(id),
        Err(error) => {
            generation.verify(profile)?;
            if storage::save_generation(profile)? != generation {
                return Err(CheckpointError::RecoveryRequired(
                    "GameCube ordinary generation changed during capture; current saves retained"
                        .into(),
                ));
            }
            Err(CheckpointError::StorageUnavailable(error.to_string()))
        }
    }
}
fn validate_generation(generation: &SaveGeneration) -> io::Result<()> {
    if !generation.images.contains_key("GC/SRAM.raw")
        || !generation
            .images
            .keys()
            .any(|name| name.starts_with("GC/MemoryCardA."))
    {
        return Err(io::Error::other(
            "checkpoint requires acknowledged card and console data",
        ));
    }
    Ok(())
}
/// Synchronous native capture directly into the durable Rust container. The
/// caller owns the initialized, paused engine and its ProfileLock throughout.
///
/// # Safety
/// Serialize this call with all native lifecycle work; retain callbacks, profile
/// ownership and native runtime until it returns.
#[cfg(feature = "native-engine")]
pub unsafe fn capture(
    profile: &Path,
    binding: &Binding,
    paused: bool,
    available: &mut dyn FnMut(&Path) -> io::Result<Capacity>,
) -> Result<String, crate::lifecycle::CheckpointError> {
    struct Writer<'a> {
        profile: &'a Path,
        binding: &'a Binding,
        paused: bool,
        available: &'a mut dyn FnMut(&Path) -> io::Result<Capacity>,
        result: Option<Result<String, crate::lifecycle::CheckpointError>>,
    }
    unsafe extern "C" fn write(
        context: *mut std::ffi::c_void,
        bytes: *const u8,
        length: usize,
    ) -> i32 {
        if context.is_null() || bytes.is_null() || length == 0 || length > PAYLOAD_LIMIT {
            return 0;
        }
        // SAFETY: synchronous callback; native lends its complete bounded buffer
        // and the stack context remains live until capture returns.
        let writer = unsafe { &mut *context.cast::<Writer<'_>>() };
        let result = publish_captured(
            writer.profile,
            writer.binding,
            writer.paused,
            unsafe { std::slice::from_raw_parts(bytes, length) },
            (writer.available)(writer.profile),
        );
        let ok = result.is_ok();
        writer.result = Some(result);
        i32::from(ok)
    }
    let mut writer = Writer {
        profile,
        binding,
        paused,
        available,
        result: None,
    };
    // SAFETY: public engine lifecycle is serialized by the host, and the bridge
    // refuses an absent/running/failed runtime before borrowing the context.
    let acknowledged = unsafe {
        crate::backend::dolphin::ntx_engine_checkpoint_capture(
            PAYLOAD_LIMIT,
            write,
            std::ptr::from_mut(&mut writer).cast(),
        )
    };
    // CaptureNtx returns the writer result directly. An intentional storage
    // refusal therefore returns zero; native recovery flags still take priority.
    finish_capture(
        acknowledged,
        unsafe { crate::backend::dolphin::ntx_engine_recovery_required() } != 0,
        writer.result,
    )
}
#[cfg(any(feature = "native-engine", test))]
fn finish_capture(
    acknowledged: i32,
    native_failed: bool,
    result: Option<Result<String, crate::lifecycle::CheckpointError>>,
) -> Result<String, crate::lifecycle::CheckpointError> {
    use crate::lifecycle::CheckpointError;
    if native_failed {
        return Err(crate::lifecycle::CheckpointError::RecoveryRequired(
            "native GameCube persistence requires recovery; current saves retained".into(),
        ));
    }
    let result = result.ok_or_else(|| {
        CheckpointError::RecoveryRequired(
            "native checkpoint capture refused; paused runtime and prior checkpoint retained"
                .into(),
        )
    })?;
    if acknowledged != i32::from(result.is_ok()) {
        return Err(crate::lifecycle::CheckpointError::RecoveryRequired(
            "native checkpoint did not acknowledge completion".into(),
        ));
    }
    result
}
/// Publish an immutable payload first. The caller atomically publishes this ID
/// in SavedSession afterward; its old reference stays usable on metadata error.
pub fn publish(
    profile: &Path,
    binding: &Binding,
    paused: bool,
    payload: &[u8],
    available: Capacity,
) -> io::Result<String> {
    binding.validate()?;
    if payload.is_empty() || payload.len() > PAYLOAD_LIMIT {
        return Err(io::Error::other(
            "invalid checkpoint size; paused runtime retained",
        ));
    }
    let generation = storage::save_generation(profile)?;
    validate_generation(&generation)?;
    let record = Metadata {
        binding: binding.clone(),
        generation,
        paused,
        payload_sha256: storage::digest(payload),
        payload_length: payload.len(),
    };
    let metadata = serde_json::to_vec(&record)?;
    if metadata.len() > HEADER_LIMIT {
        return Err(io::Error::other("checkpoint metadata too large"));
    }
    let length = (metadata.len() as u32).to_le_bytes();
    let parts: [&[u8]; 4] = [MAGIC, &length, &metadata, payload];
    let id = parts_digest(&parts);
    fs::create_dir_all(directory(profile)?)?;
    let destination = path(profile, &id)?;
    match destination.symlink_metadata() {
        Ok(_) => {
            // Identical complete data requires no new write. Any incomplete
            // same-identity attempt stays untouched for explicit disposition.
            verify_publication(profile, &id, &parts, &record.generation)?;
            return Ok(id);
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    // Reusing verified identical bytes above needs no new allocation. Every new
    // publication, including retries with recovery files, passes the same bounds.
    admit_capture(profile, payload.len(), available)?;
    storage::publish_immutable_parts(&destination, &parts)?;
    // The complete expected container is still borrowed from the paused core.
    // Compare every persisted byte without allocating/restoring another machine
    // image or hashing it twice again. Real restore retains all digest/binding
    // checks in load(). Fsync/rename/directory sync have already completed.
    verify_publication(profile, &id, &parts, &record.generation)?;
    Ok(id)
}
fn verify_publication(
    profile: &Path,
    id: &str,
    parts: &[&[u8]],
    generation: &SaveGeneration,
) -> io::Result<()> {
    let path = existing_path(profile, id)?;
    if path.symlink_metadata()?.file_type().is_symlink() {
        return Err(io::Error::other("checkpoint file is a symlink"));
    }
    verify_parts(File::open(path)?, parts)?;
    // Also reject an outside card/SRAM or generation change before acknowledging
    // publication. Matching checkpoint bytes alone cannot admit older saves.
    generation.verify(profile)
}
fn verify_parts(mut reader: impl Read, parts: &[&[u8]]) -> io::Result<()> {
    let mut buffer = [0; 65536];
    for part in parts {
        for chunk in part.chunks(buffer.len()) {
            reader.read_exact(&mut buffer[..chunk.len()])?;
            if &buffer[..chunk.len()] != chunk {
                return Err(io::Error::other(
                    "checkpoint readback failed; prior reference retained",
                ));
            }
        }
    }
    if reader.read(&mut buffer[..1])? != 0 {
        return Err(io::Error::other("checkpoint grew during readback"));
    }
    Ok(())
}
/// Validate a retained container without native restore or altering ordinary data.
/// Its binding is evidence from the file, never authorization to load a new core.
pub fn inspect_retained(profile: &Path, id: &str) -> io::Result<Binding> {
    let path = existing_path(profile, id)?;
    let info = path.symlink_metadata()?;
    if !info.is_file() || info.len() > (PAYLOAD_LIMIT + HEADER_LIMIT + 12) as u64 {
        return Err(io::Error::other("invalid retained checkpoint file"));
    }
    let mut file = File::open(path)?;
    let mut header = [0; 12];
    file.read_exact(&mut header)?;
    if &header[..8] != MAGIC && &header[..8] != crate::legacy::CHECKPOINT_MAGIC {
        return Err(io::Error::other("unknown retained checkpoint format"));
    }
    let length = u32::from_le_bytes(header[8..].try_into().unwrap()) as usize;
    if length > HEADER_LIMIT {
        return Err(io::Error::other("oversized retained checkpoint header"));
    }
    let mut bytes = vec![0; length];
    file.read_exact(&mut bytes)?;
    let metadata: Metadata = serde_json::from_slice(&bytes)?;
    metadata.binding.validate()?;
    if metadata.payload_length == 0
        || metadata.payload_length > PAYLOAD_LIMIT
        || info.len() != (12 + length + metadata.payload_length) as u64
    {
        return Err(io::Error::other("partial retained checkpoint"));
    }
    let mut container = Sha256::new();
    container.update(header);
    container.update(bytes);
    let mut payload = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        container.update(&buffer[..n]);
        payload.update(&buffer[..n]);
    }
    if format!("{:x}", container.finalize()) != id
        || format!("{:x}", payload.finalize()) != metadata.payload_sha256
    {
        return Err(io::Error::other("damaged retained checkpoint"));
    }
    Ok(metadata.binding)
}
/// Validate all inputs and current ordinary files before native deserialization.
/// Every error leaves the checkpoint and ordinary saves in place.
pub fn load(profile: &Path, id: &str, expected: &Binding) -> io::Result<Checkpoint> {
    expected.validate()?;
    let path = existing_path(profile, id)?;
    if path.symlink_metadata()?.file_type().is_symlink() {
        return Err(io::Error::other("checkpoint file is a symlink"));
    }
    let mut file = File::open(path)?;
    let file_length = file.metadata()?.len();
    if file_length > (PAYLOAD_LIMIT + HEADER_LIMIT + 12) as u64 {
        return Err(io::Error::other(
            "checkpoint is too large; original retained",
        ));
    }
    let mut header = [0; 12];
    file.read_exact(&mut header)?;
    if &header[..8] != MAGIC && &header[..8] != crate::legacy::CHECKPOINT_MAGIC {
        return Err(io::Error::other(
            "unsupported checkpoint format; original retained",
        ));
    }
    let length = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;
    if length > HEADER_LIMIT {
        return Err(io::Error::other("invalid checkpoint header"));
    }
    let mut bytes = vec![0; length];
    file.read_exact(&mut bytes)?;
    let metadata: Metadata = serde_json::from_slice(&bytes)?;
    if metadata.binding != *expected {
        return Err(io::Error::other(
            "checkpoint core/resources/game/disc changed; restart with current saves",
        ));
    }
    if metadata.payload_length == 0
        || metadata.payload_length > PAYLOAD_LIMIT
        || file_length != (12 + length + metadata.payload_length) as u64
    {
        return Err(io::Error::other("invalid checkpoint payload length"));
    }
    metadata.generation.verify(profile)?;
    let mut payload = Vec::new();
    payload
        .try_reserve_exact(metadata.payload_length)
        .map_err(io::Error::other)?;
    file.take(metadata.payload_length as u64 + 1)
        .read_to_end(&mut payload)?;
    if payload.len() != metadata.payload_length
        || storage::digest(&payload) != metadata.payload_sha256
        || parts_digest(&[&header, &bytes, &payload]) != id
    {
        return Err(io::Error::other(
            "checkpoint bytes are damaged; current saves retained",
        ));
    }
    Ok(Checkpoint {
        payload,
        paused: metadata.paused,
        generation: metadata.generation,
    })
}
/// Only after the new SavedSession reference is durably acknowledged, retire a
/// known earlier successful checkpoint. Never pass a rejected checkpoint here.
pub fn retire_previous(profile: &Path, old: &str, current: &str) -> io::Result<()> {
    if old == current {
        return Ok(());
    }
    discard_successful(profile, old)
}
/// Only after a durable explicit Stop or replacement reference; never delete a
/// rejected or unacknowledged checkpoint. Caller retains the profile lock.
pub fn discard_successful(profile: &Path, id: &str) -> io::Result<()> {
    let old = existing_path(profile, id)?;
    let info = match old.symlink_metadata() {
        Ok(info) => info,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let limit = PAYLOAD_LIMIT as u64 + HEADER_LIMIT as u64 + 12;
    if !info.is_file() || info.len() < 12 || info.len() > limit {
        return Err(io::Error::other("unsafe checkpoint retained for recovery"));
    }
    let mut source = File::open(&old)?;
    let mut magic = [0; 8];
    source.read_exact(&mut magic)?;
    if (&magic != MAGIC && &magic != crate::legacy::CHECKPOINT_MAGIC)
        || media_digest(magic.as_slice().chain(source.take(limit + 1 - 8)))? != id
    {
        return Err(io::Error::other("changed checkpoint retained for recovery"));
    }
    match fs::remove_file(old) {
        Ok(()) => File::open(directory(profile)?)?.sync_all(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    fn space(available_bytes: u64) -> crate::resume_storage::Capacity {
        crate::resume_storage::Capacity {
            available_bytes,
            remaining_resume_bytes: u64::MAX,
        }
    }

    use super::*;
    use crate::{ProfileLock, storage::CardStore};
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    fn setup(name: &str) -> (PathBuf, ProfileLock, CardStore, Binding) {
        let root = std::env::temp_dir().join(format!(
            "ntx-checkpoint-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("GC")).unwrap();
        let lock = ProfileLock::acquire(&root).unwrap();
        let writer = CardStore::bind(&root).unwrap();
        write(&root.join("GC/MemoryCardA.fixture.raw"), b"original card");
        write(&root.join("GC/SRAM.raw"), b"original console identity");
        let binding = Binding {
            game_id: "a".repeat(32),
            profile_id: "b".repeat(32),
            engine: "c".repeat(64),
            state_abi: 1,
            resources: "original resource fixture".into(),
            media_sha256: "d".repeat(64),
            disc: 0,
        };
        (root, lock, writer, binding)
    }
    #[test]
    fn collection_quota_refusal_keeps_current_progress_and_reuses_identical_bytes() {
        use crate::lifecycle::CheckpointError;
        let _serial = storage::STORE_TEST_LOCK.lock().unwrap();
        let (root, lock, writer, binding) = setup("collection-quota");
        let old = publish(&root, &binding, true, b"current", space(u64::MAX)).unwrap();
        let ordinary = storage::save_generation(&root).unwrap();
        let full = Capacity {
            available_bytes: u64::MAX,
            remaining_resume_bytes: 0,
        };
        assert!(matches!(
            publish_captured(&root, &binding, true, b"next", Ok(full)),
            Err(CheckpointError::StorageUnavailable(_))
        ));
        assert_eq!(storage::save_generation(&root).unwrap(), ordinary);
        assert_eq!(load(&root, &old, &binding).unwrap().payload, b"current");
        assert_eq!(
            publish(&root, &binding, true, b"current", full).unwrap(),
            old
        );
        let exact = Capacity {
            remaining_resume_bytes: 4 + 65548,
            ..full
        };
        let next = publish(&root, &binding, true, b"next", exact).unwrap();
        assert_eq!(load(&root, &next, &binding).unwrap().payload, b"next");
        drop((writer, lock));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn legacy_container_reader_keeps_exact_binding_and_never_rewrites_bytes() {
        let _serial = storage::STORE_TEST_LOCK.lock().unwrap();
        let (root, lock, writer, binding) = setup("legacy-reader");
        let current = publish(
            &root,
            &binding,
            true,
            b"original fixture scene",
            space(u64::MAX),
        )
        .unwrap();
        let mut bytes = fs::read(path(&root, &current).unwrap()).unwrap();
        bytes[..8].copy_from_slice(crate::legacy::CHECKPOINT_MAGIC);
        let legacy = storage::digest(&bytes);
        let retained = directory(&root)
            .unwrap()
            .join(format!("{legacy}.{}", crate::legacy::CHECKPOINT_EXTENSION));
        fs::write(&retained, &bytes).unwrap();
        assert_eq!(
            load(&root, &legacy, &binding).unwrap().payload,
            b"original fixture scene"
        );
        let mut changed = binding.clone();
        changed.engine = "e".repeat(64);
        assert!(load(&root, &legacy, &changed).is_err());
        assert_eq!(inspect_retained(&root, &legacy).unwrap(), binding);
        assert_eq!(fs::read(retained).unwrap(), bytes);
        drop(writer);
        drop(lock);
        fs::remove_dir_all(root).unwrap();
    }
    fn write(path: &Path, bytes: &[u8]) {
        let path = CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(
            unsafe { storage::write_card(path.as_ptr(), bytes.as_ptr(), bytes.len()) },
            1
        );
    }
    #[test]
    fn streamed_readback_rejects_damage_and_newer_saves_without_rewriting_either() {
        let _serial = storage::STORE_TEST_LOCK.lock().unwrap();
        let (root, lock, writer, binding) = setup("streamed-readback");
        let payload = vec![0x37; 131_081]; // cross both part and 64KiB boundaries
        let id = publish(&root, &binding, true, &payload, space(u64::MAX)).unwrap();
        let checkpoint = path(&root, &id).unwrap();
        let original = fs::read(&checkpoint).unwrap();
        let generation = storage::save_generation(&root).unwrap();
        let parts = [&original[..12], &original[12..100], &original[100..]];
        verify_publication(&root, &id, &parts, &generation).unwrap();
        let mut damaged = Vec::new();
        for offset in [0, 8, 12, 65_536, original.len() - 1] {
            let mut bytes = original.clone();
            bytes[offset] ^= 1;
            damaged.push(bytes);
        }
        damaged.push(original[..original.len() - 1].to_vec());
        damaged.push([original.as_slice(), &[1]].concat());
        for bytes in damaged {
            fs::write(&checkpoint, &bytes).unwrap();
            assert!(verify_publication(&root, &id, &parts, &generation).is_err());
            assert_eq!(fs::read(&checkpoint).unwrap(), bytes);
            assert_eq!(storage::save_generation(&root).unwrap(), generation);
        }
        fs::write(&checkpoint, &original).unwrap();
        write(&root.join("GC/MemoryCardA.fixture.raw"), b"newer save");
        assert!(verify_publication(&root, &id, &parts, &generation).is_err());
        assert_eq!(fs::read(&checkpoint).unwrap(), original);
        assert_eq!(
            fs::read(root.join("GC/MemoryCardA.fixture.raw")).unwrap(),
            b"newer save"
        );
        drop(writer);
        drop(lock);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn streamed_readback_propagates_midstream_io_failure() {
        struct FailedReader {
            bytes: io::Cursor<Vec<u8>>,
        }
        impl Read for FailedReader {
            fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
                if self.bytes.position() >= 65_536 {
                    return Err(io::Error::new(io::ErrorKind::PermissionDenied, "fixture"));
                }
                self.bytes.read(output)
            }
        }
        let bytes = vec![0x53; 131_072];
        let parts = [&bytes[..]];
        verify_parts(io::Cursor::new(&bytes), &parts).unwrap();
        assert_eq!(
            verify_parts(
                FailedReader {
                    bytes: io::Cursor::new(bytes.clone())
                },
                &parts
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::PermissionDenied
        );
    }
    #[test]
    fn media_digest_matches_independent_sha256_vectors_across_read_chunks() {
        assert_eq!(
            media_digest(&b"abc"[..]).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            media_digest(vec![b'a'; 1_000_000].as_slice()).unwrap(),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }
    #[test]
    fn complete_generation_and_pause_intent_survive_reopen_but_newer_saves_refuse_restore() {
        let _serial = storage::STORE_TEST_LOCK.lock().unwrap();
        let (root, lock, writer, binding) = setup("roundtrip");
        let id = publish(
            &root,
            &binding,
            true,
            b"original complete-machine fixture",
            space(u64::MAX),
        )
        .unwrap();
        drop(writer);
        let writer = CardStore::bind(&root).unwrap();
        let restored = load(&root, &id, &binding).unwrap();
        assert!(restored.paused);
        assert_eq!(restored.payload, b"original complete-machine fixture");
        let before = fs::read(path(&root, &id).unwrap()).unwrap();
        write(
            &root.join("GC/MemoryCardA.fixture.raw"),
            b"newer durable progress",
        );
        assert!(restored.verify_current_saves(&root).is_err());
        assert!(load(&root, &id, &binding).is_err());
        assert_eq!(fs::read(path(&root, &id).unwrap()).unwrap(), before);
        assert_eq!(
            fs::read(root.join("GC/MemoryCardA.fixture.raw")).unwrap(),
            b"newer durable progress"
        );
        drop(writer);
        drop(lock);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn failed_payload_or_session_publication_keeps_the_previous_reachable_checkpoint() {
        let _serial = storage::STORE_TEST_LOCK.lock().unwrap();
        let (root, lock, writer, binding) = setup("publication");
        let old = publish(&root, &binding, true, b"earlier machine", space(u64::MAX)).unwrap();
        let old_bytes = fs::read(path(&root, &old).unwrap()).unwrap();
        let pending = directory(&root)
            .unwrap()
            .join(format!("{old}.ntx.ntx-pending"));
        fs::create_dir(&pending).unwrap();
        assert_eq!(
            publish(&root, &binding, true, b"earlier machine", space(u64::MAX)).unwrap(),
            old
        );
        assert!(pending.is_dir());
        assert_eq!(fs::read(path(&root, &old).unwrap()).unwrap(), old_bytes);
        fs::remove_dir(pending).unwrap();
        let new = publish(
            &root,
            &binding,
            false,
            b"later complete machine",
            space(u64::MAX),
        )
        .unwrap();
        let session = root.join("session-fixture.json");
        storage::atomic_replace(&session, old.as_bytes()).unwrap();
        let blocked = root.join("session-fixture.json.ntx-pending");
        fs::create_dir(&blocked).unwrap();
        assert!(storage::atomic_replace(&session, new.as_bytes()).is_err());
        assert_eq!(fs::read(&session).unwrap(), old.as_bytes());
        assert_eq!(
            load(&root, &old, &binding).unwrap().payload,
            b"earlier machine"
        );
        assert_eq!(
            load(&root, &new, &binding).unwrap().payload,
            b"later complete machine"
        );
        fs::remove_dir(blocked).unwrap();
        storage::atomic_replace(&session, new.as_bytes()).unwrap();
        retire_previous(&root, &old, &new).unwrap();
        assert!(!path(&root, &old).unwrap().exists());
        assert!(!load(&root, &new, &binding).unwrap().paused);
        drop(writer);
        drop(lock);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn retirement_keeps_changed_or_unsafe_recovery_bytes_and_accepts_verified_legacy() {
        let _serial = storage::STORE_TEST_LOCK.lock().unwrap();
        let (root, lock, writer, binding) = setup("retirement-integrity");
        let id = publish(
            &root,
            &binding,
            true,
            b"acknowledged scene",
            space(u64::MAX),
        )
        .unwrap();
        let selected = path(&root, &id).unwrap();
        let original = fs::read(&selected).unwrap();
        let mut changed = original.clone();
        *changed.last_mut().unwrap() ^= 1;
        fs::write(&selected, &changed).unwrap();
        assert!(discard_successful(&root, &id).is_err());
        assert_eq!(fs::read(&selected).unwrap(), changed);
        fs::remove_file(&selected).unwrap();
        #[cfg(unix)]
        {
            let retained = root.join("unique-recovery.bin");
            fs::write(&retained, &original).unwrap();
            std::os::unix::fs::symlink(&retained, &selected).unwrap();
            assert!(discard_successful(&root, &id).is_err());
            assert_eq!(fs::read(&retained).unwrap(), original);
            assert!(
                selected
                    .symlink_metadata()
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            fs::remove_file(&selected).unwrap();
        }
        let mut legacy = original;
        legacy[..8].copy_from_slice(crate::legacy::CHECKPOINT_MAGIC);
        let legacy_id = storage::digest(&legacy);
        let legacy_path = directory(&root).unwrap().join(format!(
            "{legacy_id}.{}",
            crate::legacy::CHECKPOINT_EXTENSION
        ));
        fs::write(&legacy_path, &legacy).unwrap();
        discard_successful(&root, &legacy_id).unwrap();
        discard_successful(&root, &legacy_id).unwrap();
        assert!(!legacy_path.exists());
        drop(writer);
        drop(lock);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn incomplete_same_identity_retry_is_retained_and_does_not_rewrite_complete_data() {
        let _serial = storage::STORE_TEST_LOCK.lock().unwrap();
        let (root, lock, writer, binding) = setup("incomplete-retry");
        let old = publish(
            &root,
            &binding,
            true,
            b"current complete scene",
            space(u64::MAX),
        )
        .unwrap();
        let next = publish(
            &root,
            &binding,
            true,
            b"new complete scene",
            space(u64::MAX),
        )
        .unwrap();
        let destination = path(&root, &next).unwrap();
        let complete = fs::read(&destination).unwrap();
        fs::remove_file(&destination).unwrap();
        let pending = destination.with_file_name(format!("{next}.ntx.ntx-pending"));
        fs::write(&pending, b"original partial attempt").unwrap();
        assert!(
            publish(
                &root,
                &binding,
                true,
                b"new complete scene",
                space(u64::MAX)
            )
            .is_err()
        );
        assert_eq!(fs::read(&pending).unwrap(), b"original partial attempt");
        assert!(!destination.exists());
        assert_eq!(
            load(&root, &old, &binding).unwrap().payload,
            b"current complete scene"
        );
        fs::write(&destination, &complete).unwrap();
        let before = fs::metadata(&destination).unwrap().modified().unwrap();
        assert_eq!(
            publish(
                &root,
                &binding,
                true,
                b"new complete scene",
                space(u64::MAX)
            )
            .unwrap(),
            next
        );
        assert_eq!(
            fs::metadata(&destination).unwrap().modified().unwrap(),
            before
        );
        assert_eq!(fs::read(&pending).unwrap(), b"original partial attempt");
        assert_eq!(fs::read(&destination).unwrap(), complete);
        drop(writer);
        drop(lock);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn recovery_budget_refuses_new_scenes_but_keeps_current_saves_and_identical_reuse() {
        let _serial = storage::STORE_TEST_LOCK.lock().unwrap();
        let (root, lock, writer, binding) = setup("recovery-budget");
        let old = publish(&root, &binding, true, b"current", space(u64::MAX)).unwrap();
        let retained = directory(&root).unwrap().join("uncertain.ntx-pending");
        let unknown = directory(&root).unwrap().join("unknown.gcxstate");
        fs::write(&retained, b"incomplete original bytes").unwrap();
        fs::write(&unknown, b"unknown legacy bytes").unwrap();
        let generation = storage::save_generation(&root).unwrap();
        assert!(publish(&root, &binding, true, b"new", space(u64::MAX)).is_err());
        assert_eq!(
            publish(&root, &binding, true, b"current", space(0)).unwrap(),
            old
        );
        assert_eq!(fs::read(&retained).unwrap(), b"incomplete original bytes");
        assert_eq!(fs::read(&unknown).unwrap(), b"unknown legacy bytes");
        assert_eq!(load(&root, &old, &binding).unwrap().payload, b"current");
        assert_eq!(storage::save_generation(&root).unwrap(), generation);
        assert!(ProfileLock::acquire(&root).is_err());
        // Only this fixture's known file is removed, never an application sweep.
        fs::remove_file(unknown).unwrap();
        let new = publish(&root, &binding, true, b"new", space(u64::MAX)).unwrap();
        retire_previous(&root, &old, &new).unwrap();
        assert_eq!(fs::read(retained).unwrap(), b"incomplete original bytes");
        assert_eq!(load(&root, &new, &binding).unwrap().payload, b"new");
        drop(writer);
        drop(lock);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn space_byte_and_entry_bounds_never_modify_saves_or_recovery_candidates() {
        let _serial = storage::STORE_TEST_LOCK.lock().unwrap();
        let (root, lock, writer, binding) = setup("space-budget");
        let old = publish(&root, &binding, true, b"current", space(u64::MAX)).unwrap();
        let generation = storage::save_generation(&root).unwrap();
        let needed = 3 + HEADER_LIMIT as u64 + 12 + FREE_SPACE_RESERVE;
        assert!(publish(&root, &binding, true, b"new", space(needed - 1)).is_err());
        assert_eq!(fs::read_dir(directory(&root).unwrap()).unwrap().count(), 1);
        let candidate = directory(&root).unwrap().join("oversized.ntx-pending");
        // Sparse fixture exercises total bytes without allocating a giant image.
        File::create(&candidate)
            .unwrap()
            .set_len(MAX_RETAINED_FILES as u64 * FILE_LIMIT)
            .unwrap();
        assert!(publish(&root, &binding, true, b"new", space(u64::MAX)).is_err());
        assert_eq!(fs::metadata(&candidate).unwrap().len(), 3 * FILE_LIMIT);
        fs::remove_file(&candidate).unwrap();
        fs::create_dir(&candidate).unwrap();
        assert!(publish(&root, &binding, true, b"new", space(u64::MAX)).is_err());
        fs::remove_dir(&candidate).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(path(&root, &old).unwrap(), &candidate).unwrap();
            assert!(publish(&root, &binding, true, b"new", space(u64::MAX)).is_err());
            assert!(
                candidate
                    .symlink_metadata()
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            fs::remove_file(&candidate).unwrap();
        }
        let new = publish(&root, &binding, true, b"new", space(needed)).unwrap();
        assert_eq!(load(&root, &old, &binding).unwrap().payload, b"current");
        assert_eq!(load(&root, &new, &binding).unwrap().payload, b"new");
        assert_eq!(storage::save_generation(&root).unwrap(), generation);
        drop(writer);
        drop(lock);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn acknowledged_replacements_stay_bounded_without_changing_other_game_data() {
        let _serial = storage::STORE_TEST_LOCK.lock().unwrap();
        let (root, lock, writer, binding) = setup("replacement-budget");
        let other = root.join("other-game-retained");
        fs::write(&other, b"independent progress and unsaved place").unwrap();
        let mut previous = publish(&root, &binding, true, b"initial", space(u64::MAX)).unwrap();
        for frame in 0u8..40 {
            let next = publish(&root, &binding, true, &[frame], space(u64::MAX)).unwrap();
            retire_previous(&root, &previous, &next).unwrap();
            assert_eq!(fs::read_dir(directory(&root).unwrap()).unwrap().count(), 1);
            previous = next;
        }
        assert_eq!(
            fs::read(other).unwrap(),
            b"independent progress and unsaved place"
        );
        drop(writer);
        drop(lock);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn captured_storage_refusal_keeps_newer_ordinary_data_and_allows_retry() {
        use crate::lifecycle::CheckpointError;
        let _serial = storage::STORE_TEST_LOCK.lock().unwrap();
        let (root, lock, writer, binding) = setup("captured-storage");
        let old = publish(&root, &binding, true, b"older scene", space(u64::MAX)).unwrap();
        let old_bytes = fs::read(path(&root, &old).unwrap()).unwrap();
        write(
            &root.join("GC/MemoryCardA.fixture.raw"),
            b"newer committed progress",
        );
        for available in [Ok(space(0)), Err(io::Error::other("space query failed"))] {
            assert!(matches!(
                publish_captured(&root, &binding, true, b"latest complete capture", available),
                Err(CheckpointError::StorageUnavailable(_))
            ));
            assert_eq!(fs::read(path(&root, &old).unwrap()).unwrap(), old_bytes);
            assert_eq!(
                fs::read(root.join("GC/MemoryCardA.fixture.raw")).unwrap(),
                b"newer committed progress"
            );
            assert!(load(&root, &old, &binding).is_err());
            assert!(ProfileLock::acquire(&root).is_err());
        }
        // Failed new write leaves its owned pending candidate, never truncates it.
        let next = publish_captured(
            &root,
            &binding,
            true,
            b"latest complete capture",
            Ok(space(u64::MAX)),
        )
        .unwrap();
        let complete = path(&root, &next).unwrap();
        let bytes = fs::read(&complete).unwrap();
        fs::remove_file(&complete).unwrap();
        let pending = complete.with_file_name(format!("{next}.ntx.ntx-pending"));
        fs::write(&pending, b"interrupted new write").unwrap();
        assert!(matches!(
            publish_captured(
                &root,
                &binding,
                true,
                b"latest complete capture",
                Ok(space(u64::MAX))
            ),
            Err(CheckpointError::StorageUnavailable(_))
        ));
        assert_eq!(fs::read(&pending).unwrap(), b"interrupted new write");
        fs::write(&complete, bytes).unwrap();
        assert_eq!(
            publish_captured(
                &root,
                &binding,
                true,
                b"latest complete capture",
                Ok(space(0))
            )
            .unwrap(),
            next
        );
        assert_eq!(
            load(&root, &next, &binding).unwrap().payload,
            b"latest complete capture"
        );
        drop(writer);
        drop(lock);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn failed_capture_or_unverified_ordinary_data_never_becomes_a_storage_warning() {
        use crate::lifecycle::CheckpointError;
        let _serial = storage::STORE_TEST_LOCK.lock().unwrap();
        let (root, lock, writer, binding) = setup("captured-fatal");
        assert!(matches!(
            publish_captured(&root, &binding, true, b"", Ok(space(0))),
            Err(CheckpointError::RecoveryRequired(_))
        ));
        let mut invalid = binding.clone();
        invalid.state_abi = 2;
        assert!(matches!(
            publish_captured(&root, &invalid, true, b"complete", Ok(space(0))),
            Err(CheckpointError::RecoveryRequired(_))
        ));
        fs::write(root.join("GC/SRAM.raw"), b"unacknowledged external change").unwrap();
        assert!(matches!(
            publish_captured(&root, &binding, true, b"complete", Ok(space(0))),
            Err(CheckpointError::RecoveryRequired(_))
        ));
        assert_eq!(
            fs::read(root.join("GC/SRAM.raw")).unwrap(),
            b"unacknowledged external change"
        );
        drop(writer);
        drop(lock);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn native_failure_or_acknowledgement_mismatch_overrides_storage_classification() {
        use crate::lifecycle::CheckpointError::{RecoveryRequired, StorageUnavailable};
        for acknowledged in [0, 1, 2] {
            assert!(matches!(
                finish_capture(acknowledged, false, None),
                Err(RecoveryRequired(_))
            ));
            assert!(matches!(
                finish_capture(
                    acknowledged,
                    true,
                    Some(Err(StorageUnavailable("disk".into())))
                ),
                Err(RecoveryRequired(_))
            ));
        }
        assert!(matches!(
            finish_capture(0, false, Some(Err(StorageUnavailable("disk".into())))),
            Err(StorageUnavailable(_))
        ));
        assert!(matches!(
            finish_capture(1, false, Some(Err(StorageUnavailable("disk".into())))),
            Err(RecoveryRequired(_))
        ));
        assert!(matches!(
            finish_capture(0, false, Some(Ok("scene".into()))),
            Err(RecoveryRequired(_))
        ));
        assert_eq!(
            finish_capture(1, false, Some(Ok("scene".into()))).unwrap(),
            "scene"
        );
    }
    #[test]
    fn incompatible_inputs_and_corruption_retain_the_payload_and_current_saves() {
        let _serial = storage::STORE_TEST_LOCK.lock().unwrap();
        let (root, lock, writer, binding) = setup("rejection");
        let id = publish(&root, &binding, false, b"machine fixture", space(u64::MAX)).unwrap();
        let original = fs::read(path(&root, &id).unwrap()).unwrap();
        let mut alternatives = vec![binding.clone(); 6];
        alternatives[0].engine = "e".repeat(64);
        alternatives[1].resources.push_str("changed");
        alternatives[2].media_sha256 = "f".repeat(64);
        alternatives[3].disc = 1;
        alternatives[4].profile_id = "c".repeat(32);
        alternatives[5].state_abi = 2;
        for other in alternatives {
            assert!(load(&root, &id, &other).is_err());
        }
        assert_eq!(fs::read(path(&root, &id).unwrap()).unwrap(), original);
        let mut damaged = original.clone();
        *damaged.last_mut().unwrap() ^= 1;
        fs::write(path(&root, &id).unwrap(), &damaged).unwrap();
        assert!(load(&root, &id, &binding).is_err());
        assert_eq!(fs::read(path(&root, &id).unwrap()).unwrap(), damaged);
        damaged[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        fs::write(path(&root, &id).unwrap(), &damaged).unwrap();
        assert!(load(&root, &id, &binding).is_err());
        assert_eq!(
            fs::read(root.join("GC/MemoryCardA.fixture.raw")).unwrap(),
            b"original card"
        );
        assert!(load(&root, "../escape", &binding).is_err());
        drop(writer);
        drop(lock);
        fs::remove_dir_all(root).unwrap();
    }
}
