// SPDX-License-Identifier: MIT
//! Bounded publication/restore cost fixture; no native boot or private inputs.
use ntx::{ProfileLock, checkpoint, storage::CardStore};
use std::{ffi::CString, fs, os::unix::ffi::OsStrExt, path::Path, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 1 {
        return Err("usage: checkpoint-cost <new-empty-fixture-directory>".into());
    }
    // Run these on ARM64 too: publication/restore self-consistency alone would
    // not prove that an accelerated digest agrees with existing checkpoints.
    assert_eq!(
        checkpoint::media_digest(&b"abc"[..])?,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        checkpoint::media_digest(vec![b'a'; 1_000_000].as_slice())?,
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
    let root = Path::new(&args[0]);
    // Never reuse an owner profile or delete a preexisting directory.
    fs::create_dir(root)?;
    fs::create_dir(root.join("GC"))?;
    let lock = ProfileLock::acquire(root)?;
    let writer = CardStore::bind(root)?;
    for (name, bytes) in [
        ("GC/MemoryCardA.fixture.raw", vec![0x53; 16 * 1024 * 1024]),
        ("GC/SRAM.raw", vec![0x31; 68]),
    ] {
        let path = CString::new(root.join(name).as_os_str().as_bytes())?;
        // SAFETY: synchronous product write callback; fixture path/data live.
        if unsafe { ntx::storage::write_card(path.as_ptr(), bytes.as_ptr(), bytes.len()) } != 1 {
            return Err("fixture ordinary-data publication failed".into());
        }
    }
    let binding = checkpoint::Binding {
        game_id: "a".repeat(32),
        profile_id: "b".repeat(32),
        engine: "c".repeat(64),
        state_abi: 1,
        resources: "original synthetic publication fixture".into(),
        media_sha256: "d".repeat(64),
        disc: 0,
    };
    let mut payload = vec![0; 96 * 1024 * 1024];
    let mut state = 0x4d595df4d0f33173_u64;
    for chunk in payload.chunks_exact_mut(8) {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        chunk.copy_from_slice(&state.to_le_bytes());
    }
    let start = Instant::now();
    let id = checkpoint::publish(
        root,
        &binding,
        true,
        &payload,
        ntx::resume_storage::Capacity {
            available_bytes: u64::MAX,
            remaining_resume_bytes: u64::MAX,
        },
    )?;
    let micros = start.elapsed().as_micros();
    // Observe the publisher before a genuine restore allocates its payload.
    let high_water = fs::read_to_string("/proc/self/status")?
        .lines()
        .find(|line| line.starts_with("VmHWM:"))
        .unwrap_or("VmHWM: unavailable")
        .to_owned();
    println!(
        "publication_us={micros} payload_bytes={} {high_water}",
        payload.len()
    );
    println!(
        "payload_sha256={}",
        checkpoint::media_digest(payload.as_slice())?
    );
    let start = Instant::now();
    let restored = checkpoint::load(root, &id, &binding)?;
    assert!(restored.paused && restored.payload == payload);
    println!(
        "restore_validation_us={} roundtrip=PASS",
        start.elapsed().as_micros()
    );
    drop(restored);
    drop(writer);
    drop(lock);
    // Only this invocation's freshly created synthetic fixture is removed.
    fs::remove_dir_all(root)?;
    Ok(())
}
