// SPDX-License-Identifier: MIT
//! Validate a protected local N64 profile copy without loading an emulator.
use ntx::{n64_checkpoint, n64_save::SaveStore};
use std::{fs, io::Read, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 {
        return Err("usage: ntx-n64-checkpoint-inspect PROTECTED_PROFILE CHECKPOINT_ID TRUSTED_BINDING_JSON".into());
    }
    let profile = PathBuf::from(&args[0]);
    for directory in [&profile, &profile.join("N64")] {
        if !directory.symlink_metadata()?.is_dir() {
            return Err("expected an existing regular protected profile directory".into());
        }
    }
    if !profile.join("writer.lock").symlink_metadata()?.is_file() {
        return Err("protected copy must retain its existing writer.lock".into());
    }
    let binding_file = PathBuf::from(&args[2]);
    let info = binding_file.symlink_metadata()?;
    if !info.is_file() || info.len() > 8192 {
        return Err("trusted binding must be a bounded regular independent receipt".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(binding_file)?
        .take(8193)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 8192 {
        return Err("trusted binding grew beyond its limit".into());
    }
    let binding = serde_json::from_slice(&bytes)?;
    // Both directories and the lock already exist. Opening this protected-copy
    // store takes its local lock but never publishes ordinary or scene data.
    let store = SaveStore::open(&profile)?;
    let scene = n64_checkpoint::load(
        &profile,
        args[1].to_str().ok_or("checkpoint identity encoding")?,
        &binding,
        &store,
    )?;
    scene.verify_current(&store)?;
    println!(
        "PASS protected-copy N64 checkpoint inspection: {} payload bytes; exact binding and current ordinary generation verified. No core boot, restore or save publication.",
        scene.payload.len()
    );
    Ok(())
}
