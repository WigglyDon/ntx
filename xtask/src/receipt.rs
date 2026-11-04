// SPDX-License-Identifier: MIT
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn output(command: &mut Command) -> Result<String> {
    let result = command.output()?;
    if !result.status.success() {
        return Err("source/tool identity command failed".into());
    }
    Ok(String::from_utf8(result.stdout)?)
}
pub(crate) fn digest(path: &Path) -> Result<String> {
    let text = output(Command::new("sha256sum").arg("--").arg(path))?;
    let hash = text
        .split_whitespace()
        .next()
        .ok_or("missing source digest")?;
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid source digest".into());
    }
    Ok(hash.into())
}
pub fn write(root: &Path, source: &Path) -> Result<()> {
    let listing = Command::new("git")
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ])
        .current_dir(root)
        .output()?;
    if !listing.status.success() {
        return Err("cannot enumerate public source inputs".into());
    }
    let mut paths = listing
        .stdout
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8(s.to_vec()))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    paths.sort();
    paths.dedup();
    let mut inventory = String::new();
    for path in &paths {
        let relative = Path::new(path);
        if relative
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err("invalid source input path".into());
        }
        let input = root.join(relative);
        if input.symlink_metadata()?.file_type().is_symlink() || !input.is_file() {
            return Err("public source inventory requires ordinary files".into());
        }
        inventory.push_str(&format!("{}  {path}\n", digest(&input)?));
    }
    let out = root.join("build/source-receipt");
    fs::create_dir_all(&out)?;
    let manifest = out.join("source-files.sha256");
    fs::write(&manifest, inventory)?;
    let mut tools = serde_json::Map::new();
    tools.insert(
        "rustc".into(),
        output(Command::new("rustc").arg("-vV"))?.into(),
    );
    tools.insert(
        "cmake".into(),
        output(Command::new("cmake").arg("--version"))?.into(),
    );
    tools.insert(
        "ninja".into(),
        output(Command::new("ninja").arg("--version"))?.into(),
    );
    if let Some(home) =
        std::env::var_os("ANDROID_HOME").or_else(|| std::env::var_os("ANDROID_SDK_ROOT"))
    {
        let sdk = PathBuf::from(home);
        for relative in [
            "platforms/android-37.0/android.jar",
            "build-tools/37.0.0/aapt2",
            "build-tools/37.0.0/zipalign",
            "build-tools/37.0.0/lib/d8.jar",
            "build-tools/37.0.0/lib/apksigner.jar",
        ] {
            tools.insert(relative.into(), digest(&sdk.join(relative))?.into());
        }
    }
    if let Some(home) = std::env::var_os("ANDROID_NDK_HOME") {
        let ndk = PathBuf::from(home);
        tools.insert(
            "NDK/source.properties".into(),
            fs::read_to_string(ndk.join("source.properties"))?.into(),
        );
        for relative in [
            "toolchains/llvm/prebuilt/linux-x86_64/bin/clang",
            "toolchains/llvm/prebuilt/linux-x86_64/bin/ld.lld",
            "toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-strip",
            "toolchains/llvm/prebuilt/linux-x86_64/sysroot/usr/lib/aarch64-linux-android/libc++_shared.so",
        ] {
            tools.insert(
                format!("NDK/{relative}"),
                digest(&ndk.join(relative))?.into(),
            );
        }
    }
    let mupen = crate::mupen::source_check(root)?;
    let binding = serde_json::json!({
        "schema":1,"native_abi":8,"checkpoint_abi":1,"first_party_head":output(Command::new("git").args(["rev-parse","HEAD"]).current_dir(root))?.trim(),
        "source_inventory_sha256":digest(&manifest)?,"source_files":paths.len(),
        "dolphin_commit":output(Command::new("git").arg("-C").arg(source).args(["rev-parse","HEAD"]))?.trim(),
        "dolphin_sys_tree":output(Command::new("git").arg("-C").arg(source).args(["rev-parse","HEAD:Data/Sys"]))?.trim(),
        "engine_patch_sha256":digest(&root.join("native/patches/0001-direct-adapter.patch"))?,
        "gitlink_graph_sha256":digest(&root.join("native/dolphin-gitlinks.tsv"))?,
        "mupen_commit":"15bada85906593376e8fd697f1b0b3af3a20f512",
        "mupen_sources_sha256":digest(&root.join("native/mupen/source-files.sha256"))?,
        "mupen_patched_sources_sha256":digest(&root.join("native/mupen/patched-files.sha256"))?,
        "mupen_selected_sources_sha256":digest(&root.join("native/mupen/selected-sources.txt"))?,
        "mupen_core_sha256":digest(&root.join("build/ntx-mupen/lib/arm64-v8a/libntx_mupen64plus_next.so"))?,
        "mupen_original_archive_sha256":digest(&root.join(".local/source-archives/mupen64plus-next-15bada85.tar.gz"))?,
        "mupen_patches":[digest(&root.join("native/mupen/0001-checked-state-result.patch"))?,digest(&root.join("native/mupen/0002-coherent-state-boundary.patch"))?,digest(&root.join("native/mupen/0003-host-shader-dispatch.patch"))?,digest(&root.join("native/mupen/0004-profile-shader-cache.patch"))?,digest(&root.join("native/mupen/0005-compile-observer.patch"))?,digest(&root.join("native/mupen/0006-compiler-phase-observer.patch"))?,digest(&root.join("native/mupen/0007-complete-cache-batch.patch"))?,digest(&root.join("native/mupen/0008-cull-register-membership.patch"))?,digest(&root.join("native/mupen/0009-cull-query-counts.patch"))?,digest(&root.join("native/mupen/0010-incoming-branch-index.patch"))?,digest(&root.join("native/mupen/0011-literal-log-payload.patch"))?,digest(&root.join("native/mupen/0012-assembly-child-scopes.patch"))?,digest(&root.join("native/mupen/0013-upper-constant-mask.patch"))?],
        "mupen_makefile_sha256":digest(&mupen.join("libretro/jni/Android.mk"))?,
        "mupen_build":"NDK30/API28/arm64-v8a/GLES3/HLE/GLideN64; LLE=0 HAVE_PARALLEL_RSP=0 HAVE_PARALLEL_RDP=0 HAVE_THR_AL=0; -j1; 16KiB ELF",
        "cargo_lock_sha256":digest(&root.join("Cargo.lock"))?,"tools":tools
    });
    fs::write(
        out.join("binding.json"),
        serde_json::to_vec_pretty(&binding)?,
    )?;
    println!(
        "Recorded {} public source inputs and observed tool hashes; binary correspondence still requires package checks",
        paths.len()
    );
    Ok(())
}
