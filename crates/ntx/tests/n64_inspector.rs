// SPDX-License-Identifier: MIT
use ntx::{n64_checkpoint, n64_save::SaveStore};
use std::{fs, process::Command};

#[test]
fn protected_copy_inspection_preserves_files_and_rejects_wrong_binding_or_save_bytes() {
    let root = std::env::temp_dir().join(format!("ntx-n64-inspector-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let profile = root.join("profile");
    let mut store = SaveStore::open(&profile).unwrap();
    let generation = store.commit(&vec![0; ntx::n64::SAVE_IMAGE_BYTES]).unwrap();
    let binding = n64_checkpoint::Binding {
        game_id: "a".repeat(32),
        profile_id: "b".repeat(32),
        media_sha256: "c".repeat(64),
        core_sha256: "d".repeat(64),
        options_sha256: "e".repeat(64),
        architecture: "aarch64-android-gles3-mupen-state1".into(),
    };
    let id = n64_checkpoint::publish(&profile, &binding, &store, b"retained fixture").unwrap();
    drop(store);
    let paths = [
        profile.join("writer.lock"),
        profile.join("N64/ordinary.json"),
        profile
            .join("N64")
            .join(format!("{}.m64ram", generation.image_sha256)),
        profile.join("n64-checkpoints").join(format!("{id}.ntx")),
    ];
    let before: Vec<_> = paths.iter().map(|p| fs::read(p).unwrap()).collect();
    let receipt = root.join("binding.json");
    fs::write(&receipt, serde_json::to_vec(&binding).unwrap()).unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_ntx-n64-checkpoint-inspect"))
            .args([profile.as_os_str(), id.as_ref(), receipt.as_os_str()])
            .output()
            .unwrap()
    };
    assert!(run().status.success());
    let mut wrong = binding.clone();
    wrong.core_sha256 = "f".repeat(64);
    fs::write(&receipt, serde_json::to_vec(&wrong).unwrap()).unwrap();
    assert!(!run().status.success());
    assert_eq!(
        before,
        paths
            .iter()
            .map(|p| fs::read(p).unwrap())
            .collect::<Vec<_>>()
    );
    fs::write(&receipt, serde_json::to_vec(&binding).unwrap()).unwrap();
    let mut changed = before[2].clone();
    changed[0] = 1;
    fs::write(&paths[2], changed).unwrap();
    assert!(!run().status.success());
    fs::remove_dir_all(root).unwrap();
}
