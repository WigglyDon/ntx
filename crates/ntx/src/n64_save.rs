// SPDX-License-Identifier: MIT
//! N64 ordinary progress belongs to a save profile, independently of a core
//! instance or machine checkpoint. Only the quiescent runtime may publish it.
use crate::{ProfileLock, n64::SaveImage, storage};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io,
    path::{Path, PathBuf},
};

/// Normally only the current image and scene-pinned history remain. Refuse
/// additional history if uncertain recovery candidates prevent safe retirement.
pub const MAX_ORDINARY_IMAGES: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Generation {
    pub schema: u32,
    pub number: u64,
    pub image_sha256: String,
}
pub struct SaveStore {
    directory: PathBuf,
    accepted: Option<Generation>,
    acknowledged_history: BTreeSet<String>,
    _writer: ProfileLock,
}
fn error(message: &str) -> io::Error {
    io::Error::other(message)
}
fn read_generation(directory: &Path) -> io::Result<Option<Generation>> {
    let path = directory.join("ordinary.json");
    match path.symlink_metadata() {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
        Ok(info) if !info.is_file() || info.len() > 4096 => {
            return Err(error("invalid N64 ordinary generation"));
        }
        _ => {}
    }
    let generation: Generation = serde_json::from_slice(&fs::read(path)?)?;
    if generation.schema != 1
        || generation.number == 0
        || !storage::valid_digest(&generation.image_sha256)
    {
        return Err(error("unsupported N64 ordinary generation"));
    }
    Ok(Some(generation))
}
fn image_path(directory: &Path, generation: &Generation) -> PathBuf {
    directory.join(format!("{}.m64ram", generation.image_sha256))
}
fn read_image(directory: &Path, generation: &Generation) -> io::Result<Vec<u8>> {
    let path = image_path(directory, generation);
    let info = path.symlink_metadata()?;
    if !info.is_file() || info.len() != crate::n64::SAVE_IMAGE_BYTES as u64 {
        return Err(error("invalid pinned N64 ordinary image"));
    }
    let bytes = fs::read(path)?;
    SaveImage::parse(&bytes)?;
    if storage::digest(&bytes) != generation.image_sha256 {
        return Err(error(
            "N64 ordinary data changed outside its writer; retain runtime and recovery",
        ));
    }
    Ok(bytes)
}
impl SaveStore {
    pub fn open(profile: &Path) -> io::Result<Self> {
        fs::create_dir_all(profile)?;
        if !profile.symlink_metadata()?.is_dir() {
            return Err(error("unsafe N64 profile"));
        }
        let writer = ProfileLock::acquire(profile)?;
        let directory = profile.join("N64");
        fs::create_dir_all(&directory)?;
        if !directory.symlink_metadata()?.is_dir() {
            return Err(error("unsafe N64 ordinary directory"));
        }
        let accepted = read_generation(&directory)?;
        if let Some(generation) = &accepted {
            read_image(&directory, generation)?;
        }
        Ok(Self {
            directory,
            accepted,
            acknowledged_history: BTreeSet::new(),
            _writer: writer,
        })
    }
    pub(crate) fn ensure_profile(&self, profile: &Path) -> io::Result<()> {
        if self.directory != profile.join("N64") {
            return Err(error(
                "N64 scene and ordinary writer belong to different profiles",
            ));
        }
        Ok(())
    }
    pub fn generation(&self) -> Option<&Generation> {
        self.accepted.as_ref()
    }
    /// Validate actual bytes again immediately before native restore or flush.
    pub fn verify_current(&self) -> io::Result<()> {
        if read_generation(&self.directory)? != self.accepted {
            return Err(error(
                "N64 ordinary generation advanced; stale runtime cannot publish",
            ));
        }
        if let Some(generation) = &self.accepted {
            read_image(&self.directory, generation)?;
        }
        Ok(())
    }
    pub fn load(&self) -> io::Result<Option<Vec<u8>>> {
        self.verify_current()?;
        self.accepted
            .as_ref()
            .map(|g| read_image(&self.directory, g))
            .transpose()
    }
    /// After a durable ordinary reference, keep the current ordinary image
    /// and every image pinned by a retained scene. The profile writer is held.
    pub fn retire_unreferenced_images(&mut self, retained: &BTreeSet<String>) -> io::Result<()> {
        self.verify_current()?;
        let mut protected = retained.clone();
        if let Some(current) = &self.accepted {
            protected.insert(current.image_sha256.clone());
        }
        for hash in &protected {
            if !storage::valid_digest(hash) {
                return Err(error("Invalid retained ordinary identity"));
            }
            let generation = Generation {
                schema: 1,
                number: 1,
                image_sha256: hash.clone(),
            };
            read_image(&self.directory, &generation)?;
        }
        let mut remove = Vec::new();
        // Only generations this writer actually saw acknowledged are eligible.
        // A file left by an uncertain commit or earlier process is not a cache.
        for hash in &self.acknowledged_history {
            if protected.contains(hash) {
                continue;
            }
            let generation = Generation {
                schema: 1,
                number: 1,
                image_sha256: hash.clone(),
            };
            let path = image_path(&self.directory, &generation);
            match path.symlink_metadata() {
                Ok(info) if info.is_file() => {
                    read_image(&self.directory, &generation)?;
                }
                Ok(_) => return Err(error("Unrecognized ordinary history retained")),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
            remove.push((hash.clone(), path));
        }
        self.verify_current()?;
        for (_, path) in &remove {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        File::open(&self.directory)?.sync_all()?;
        for (hash, _) in remove {
            self.acknowledged_history.remove(&hash);
        }
        self.verify_current()
    }
    /// Publish immutable image bytes before their durable generation reference.
    /// On any failure the caller retains its paused runtime and profile writer.
    pub fn commit(&mut self, bytes: &[u8]) -> io::Result<Generation> {
        SaveImage::parse(bytes)?;
        self.verify_current()?;
        let hash = storage::digest(bytes);
        if let Some(current) = &self.accepted
            && current.image_sha256 == hash
        {
            return Ok(current.clone());
        }
        let generation = Generation {
            schema: 1,
            number: self
                .accepted
                .as_ref()
                .map_or(Some(1), |g| g.number.checked_add(1))
                .ok_or_else(|| error("N64 generation exhausted"))?,
            image_sha256: hash,
        };
        let path = image_path(&self.directory, &generation);
        if path.try_exists()? {
            if read_image(&self.directory, &generation)? != bytes {
                return Err(error("retained N64 image collision"));
            }
        } else {
            let mut images = 0usize;
            for entry in fs::read_dir(&self.directory)? {
                let entry = entry?;
                let name = entry.file_name();
                if name
                    .to_str()
                    .is_some_and(|n| n.ends_with(".m64ram") || n.ends_with(".m64ram.ntx-pending"))
                {
                    if !entry.path().symlink_metadata()?.is_file() {
                        return Err(error("Unrecognized N64 ordinary recovery file retained"));
                    }
                    images += 1;
                }
            }
            if images >= MAX_ORDINARY_IMAGES {
                return Err(error(
                    "Save recovery storage is full. The current ordinary save and paused runtime are retained; free space before saving again.",
                ));
            }
            storage::publish_immutable(&path, bytes)?;
        }
        if read_image(&self.directory, &generation)? != bytes {
            return Err(error("N64 ordinary image readback failed"));
        }
        self.verify_current()?;
        storage::atomic_replace(
            &self.directory.join("ordinary.json"),
            &serde_json::to_vec(&generation)?,
        )?;
        if read_generation(&self.directory)?.as_ref() != Some(&generation) {
            return Err(error("N64 ordinary generation readback failed"));
        }
        if let Some(previous) = &self.accepted {
            self.acknowledged_history
                .insert(previous.image_sha256.clone());
        }
        self.accepted = Some(generation.clone());
        Ok(generation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("ntx-n64-{name}-{}", std::process::id()));
        fs::create_dir(&p).unwrap();
        p
    }
    #[test]
    fn unacknowledged_ordinary_candidate_is_not_swept_as_history() {
        let root = root("unknown-history");
        let mut store = SaveStore::open(&root).unwrap();
        let mut bytes = vec![0; crate::n64::SAVE_IMAGE_BYTES];
        store.commit(&bytes).unwrap();
        let old = store.generation().unwrap().clone();
        bytes[0x800] = 2;
        let unacknowledged = image_path(
            &root.join("N64"),
            &Generation {
                schema: 1,
                number: 2,
                image_sha256: storage::digest(&bytes),
            },
        );
        fs::write(&unacknowledged, &bytes).unwrap();
        bytes[0x800] = 3;
        store.commit(&bytes).unwrap();
        store.retire_unreferenced_images(&BTreeSet::new()).unwrap();
        assert!(!image_path(&root.join("N64"), &old).exists());
        assert!(unacknowledged.exists());
        assert_eq!(fs::read(unacknowledged).unwrap()[0x800], 2);
        assert_eq!(store.load().unwrap().unwrap(), bytes);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn retry_preserves_an_existing_incomplete_ordinary_image() {
        let root = root("same-pending");
        let mut store = SaveStore::open(&root).unwrap();
        let mut bytes = vec![0; crate::n64::SAVE_IMAGE_BYTES];
        let original = bytes.clone();
        let current = store.commit(&bytes).unwrap();
        bytes[0x800] = 1;
        let hash = storage::digest(&bytes);
        let pending = root.join("N64").join(format!("{hash}.m64ram.ntx-pending"));
        fs::write(&pending, b"uncertain ordinary bytes").unwrap();
        assert!(store.commit(&bytes).is_err());
        assert_eq!(fs::read(&pending).unwrap(), b"uncertain ordinary bytes");
        assert_eq!(store.generation(), Some(&current));
        assert_eq!(store.load().unwrap().unwrap(), original);
        assert!(!root.join("N64").join(format!("{hash}.m64ram")).exists());
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn uncertain_history_has_a_hard_budget_and_refusal_keeps_the_current_save() {
        let root = root("history-budget");
        let mut store = SaveStore::open(&root).unwrap();
        let mut bytes = vec![0; crate::n64::SAVE_IMAGE_BYTES];
        for n in 0..MAX_ORDINARY_IMAGES {
            bytes[0x800] = n as u8;
            store.commit(&bytes).unwrap();
        }
        let current = store.generation().unwrap().clone();
        let expected = bytes.clone();
        bytes[0x800] = MAX_ORDINARY_IMAGES as u8;
        assert!(store.commit(&bytes).is_err());
        assert_eq!(store.generation(), Some(&current));
        assert_eq!(store.load().unwrap().unwrap(), expected);
        store.retire_unreferenced_images(&BTreeSet::new()).unwrap();
        assert_eq!(
            fs::read_dir(root.join("N64"))
                .unwrap()
                .filter(|p| p
                    .as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|e| e == "m64ram"))
                .count(),
            1
        );
        store.commit(&bytes).unwrap();
        assert_eq!(store.load().unwrap().unwrap(), bytes);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn ordinary_progress_reopens_independently_and_a_writer_cannot_adopt_outside_changes() {
        let root = root("generation");
        let mut store = SaveStore::open(&root).unwrap();
        assert!(store.load().unwrap().is_none());
        assert!(SaveStore::open(&root).is_err());
        let mut bytes = vec![0; crate::n64::SAVE_IMAGE_BYTES];
        bytes[0x800] = 23;
        let generation = store.commit(&bytes).unwrap();
        assert_eq!(generation.number, 1);
        drop(store);
        let mut store = SaveStore::open(&root).unwrap();
        assert_eq!(store.load().unwrap().unwrap(), bytes);
        fs::write(
            image_path(&store.directory, &generation),
            vec![1; bytes.len()],
        )
        .unwrap();
        assert!(store.commit(&bytes).is_err());
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn failed_metadata_publication_keeps_prior_committed_generation_and_profile_lock() {
        let root = root("publication");
        let mut store = SaveStore::open(&root).unwrap();
        let mut bytes = vec![0; crate::n64::SAVE_IMAGE_BYTES];
        let old = store.commit(&bytes).unwrap();
        fs::create_dir(store.directory.join("ordinary.json.ntx-pending")).unwrap();
        bytes[0] = 9;
        assert!(store.commit(&bytes).is_err());
        assert_eq!(store.generation(), Some(&old));
        assert!(SaveStore::open(&root).is_err());
        assert_eq!(store.load().unwrap().unwrap()[0], 0);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}
