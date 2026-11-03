// SPDX-License-Identifier: MIT
//! Portable Nintendo collection, session, input and save policy.
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::path::Path;

pub mod backend;
pub mod checkpoint;
pub mod controls;
mod legacy;
pub mod lifecycle;
pub mod migration;
pub mod n64;
pub mod n64_checkpoint;
pub mod n64_controls;
pub mod n64_save;
pub mod n64_touch;
pub mod presentation;
pub mod scheduling;
pub mod touch;
pub mod viewport;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[repr(usize)]
pub enum Control {
    A,
    B,
    X,
    Y,
    Z,
    Start,
    Up,
    Down,
    Left,
    Right,
    LClick,
    RClick,
    LTravel,
    RTravel,
    StickX,
    StickY,
    CX,
    CY,
}

#[derive(Clone, Debug, Default)]
pub struct Pad([f64; 18]);

impl Pad {
    pub fn set(&mut self, control: Control, value: f64) -> Result<(), &'static str> {
        let minimum = if control as usize >= Control::StickX as usize {
            -1.0
        } else {
            0.0
        };
        if !value.is_finite() || !(minimum..=1.0).contains(&value) {
            return Err("control value outside calibrated range");
        }
        self.0[control as usize] = value;
        Ok(())
    }
    pub fn release_all(&mut self) {
        self.0.fill(0.0);
    }
    pub fn values(&self) -> &[f64; 18] {
        &self.0
    }
}

// Use the kernel primitive directly: the std file-lock API is not assumed to be
// implemented on Android. A real Android contention/teardown check is still required.
unsafe extern "C" {
    fn flock(fd: std::ffi::c_int, operation: std::ffi::c_int) -> std::ffi::c_int;
}

pub struct ProfileLock {
    _file: File,
}
impl ProfileLock {
    /// The lock file must remain at this path; never unlink it on release.
    /// Keep this guard alive until the native card flush threads have joined.
    pub fn acquire(profile: &Path) -> io::Result<Self> {
        Self::acquire_file(&profile.join("writer.lock"))
    }
    /// Shared by the app and the offline migration publisher. Never unlink it.
    pub fn acquire_file(path: &Path) -> io::Result<Self> {
        use std::os::unix::fs::OpenOptionsExt;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .custom_flags(0x20000) // O_NOFOLLOW on Linux/Android
            .mode(0o600)
            .open(path)?;
        if !file.metadata()?.is_file() {
            return Err(io::Error::other("writer lock is not an ordinary file"));
        }
        // SAFETY: file owns a valid fd; LOCK_EX | LOCK_NB has no pointer arguments.
        if unsafe { flock(file.as_raw_fd(), 2 | 4) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { _file: file })
    }
}

/// Original tiny PowerPC program. Writes the retained GCX1 fixture marker to RAM and loops forever.
/// No SDK, firmware, game or third-party executable is embedded.
pub fn cpu_probe_dol() -> Vec<u8> {
    let code: [u32; 5] = [
        0x3c60_8001,
        0x3c80_4743,
        0x6084_5831,
        0x9083_0000,
        0x4800_0000,
    ];
    let mut dol = vec![0; 0x100];
    for (offset, word) in [
        (0, 0x100_u32),
        (0x48, 0x8000_3100),
        (0x90, 32),
        (0xe0, 0x8000_3100),
    ] {
        dol[offset..offset + 4].copy_from_slice(&word.to_be_bytes());
    }
    for word in code {
        dol.extend_from_slice(&word.to_be_bytes());
    }
    // Dolphin loads DOL sections rounded to a 32-byte boundary.
    dol.resize(0x120, 0);
    dol
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn simultaneous_sticks_and_partial_triggers_release_together() {
        let mut pad = Pad::default();
        pad.set(Control::StickX, -0.8).unwrap();
        pad.set(Control::CY, 0.6).unwrap();
        pad.set(Control::LTravel, 0.4).unwrap();
        pad.set(Control::RClick, 1.0).unwrap();
        assert_eq!(pad.values()[Control::LClick as usize], 0.0);
        assert_eq!(pad.values()[Control::RTravel as usize], 0.0);
        assert_eq!(pad.values()[Control::LTravel as usize], 0.4);
        assert_eq!(pad.values()[Control::StickX as usize], -0.8);
        assert_eq!(pad.values()[Control::CY as usize], 0.6);
        for value in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
            assert!(pad.set(Control::LTravel, value).is_err());
        }
        pad.release_all();
        assert!(pad.values().iter().all(|v| *v == 0.0));
    }
    #[test]
    fn lock_contention_and_release_preserve_lock_file() {
        let dir = std::env::temp_dir().join(format!("ntx-lock-test-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let owner = ProfileLock::acquire(&dir).unwrap();
        assert!(
            matches!(ProfileLock::acquire(&dir), Err(e) if e.kind() == io::ErrorKind::WouldBlock)
        );
        drop(owner);
        let next = ProfileLock::acquire(&dir).unwrap();
        assert!(dir.join("writer.lock").exists());
        drop(next);
        std::fs::remove_file(dir.join("writer.lock")).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn profile_lock_excludes_another_process() {
        const CHILD_PATH: &str = "NTX_LOCK_TEST_CHILD_PATH";
        if let Some(path) = std::env::var_os(CHILD_PATH) {
            let result = ProfileLock::acquire(Path::new(&path));
            if std::env::var_os("NTX_LOCK_TEST_EXPECT_BUSY").is_some() {
                assert!(matches!(result, Err(e) if e.kind() == io::ErrorKind::WouldBlock));
            } else {
                assert!(result.is_ok());
            }
            return;
        }
        let dir =
            std::env::temp_dir().join(format!("ntx-process-lock-test-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let owner = ProfileLock::acquire(&dir).unwrap();
        let child = |busy: bool| {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args(["--exact", "tests::profile_lock_excludes_another_process"])
                .env(CHILD_PATH, &dir);
            if busy {
                command.env("NTX_LOCK_TEST_EXPECT_BUSY", "1");
            }
            assert!(command.status().unwrap().success());
        };
        child(true);
        drop(owner);
        child(false);
        std::fs::remove_file(dir.join("writer.lock")).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}

pub mod library;

pub mod storage;
