// SPDX-License-Identifier: MIT
//! Durable identities and host session metadata; contains no native handles or machine snapshot.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameId(pub String);
#[derive(Clone, Serialize, Deserialize)]
pub struct SaveProfileId(pub String);
#[derive(Clone, Serialize, Deserialize)]
pub struct SavedSession {
    pub system: crate::backend::System,
    pub backend: crate::backend::Backend,
    /// A locator is not a grant in a new Android package.
    #[serde(default)]
    pub media_relink_required: bool,
    #[serde(default)]
    pub media_sha256: Option<String>,
    pub game_id: GameId,
    pub profile_id: SaveProfileId,
    pub uri: String,
    pub title: String,
    pub paused: bool,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub restart_required: bool,
    #[serde(default)]
    pub runtime_incomplete: bool,
    #[serde(default)]
    pub stopped: bool,
    /// Immutable complete generation; never a native handle or ordinary card image.
    #[serde(default)]
    pub checkpoint: Option<String>,
    #[serde(default)]
    pub shortcut_requested: bool,
    /// Last OS observation, separate from accepting a pin request.
    #[serde(default)]
    pub shortcut_pinned: Option<bool>,
}
impl SavedSession {
    pub fn fail(&mut self, error: String) {
        self.paused = true;
        self.restart_required = true;
        self.last_error = Some(error.chars().take(8192).collect());
    }
    /// Explicitly cold boot using the same profile; never selects an older save image.
    pub fn request_restart(&mut self) {
        self.paused = false;
        self.restart_required = false;
        self.runtime_incomplete = false;
        self.stopped = false;
        // The rejected file remains available; explicit restart uses current saves.
        self.checkpoint = None;
    }
    pub fn boot_allowed(&self) -> bool {
        !self.stopped && !self.restart_required && !self.media_relink_required
    }
    pub fn recover_interrupted(&mut self) -> bool {
        if !self.runtime_incomplete || self.restart_required || self.checkpoint.is_some() {
            return false;
        }
        let mut message = "The previous runtime ended without a completed card barrier and teardown. Restart explicitly with current saves.".to_owned();
        if let Some(earlier) = &self.last_error {
            message.push_str("\nEarlier retained error: ");
            message.push_str(earlier);
        }
        self.fail(message);
        true
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Collection {
    pub schema_version: u32,
    pub entries: Vec<SavedSession>,
    pub selected: Option<String>,
}
impl Default for Collection {
    fn default() -> Self {
        Self {
            schema_version: 2,
            entries: Vec::new(),
            selected: None,
        }
    }
}
pub fn valid_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
/// A persisted host locator is an opaque binding, never game identity or access
/// authority. Each host adapter enforces its own supported scoped access.
fn valid_media_locator(value: &str) -> bool {
    if value.len() > 65536 || value.chars().any(char::is_control) {
        return false;
    }
    let Some((scheme, location)) = value.split_once("://") else {
        return false;
    };
    !location.is_empty()
        && !scheme.is_empty()
        && scheme.len() <= 32
        && scheme.as_bytes()[0].is_ascii_lowercase()
        && scheme
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"+.-".contains(&c))
}
#[derive(Debug, PartialEq, Eq)]
pub enum GameOpen {
    ResumeCurrent,
    Select(usize),
}
const PLAY_URI_PREFIX: &str = "machinerhythm-ntx://play/";
pub fn play_uri(id: &str) -> Result<String, &'static str> {
    if !valid_id(id) {
        return Err("Invalid game identity.");
    }
    Ok(format!("{PLAY_URI_PREFIX}{id}"))
}
pub fn parse_play_uri(uri: &str) -> Result<String, &'static str> {
    let id = uri
        .strip_prefix(PLAY_URI_PREFIX)
        .or_else(|| uri.strip_prefix(crate::legacy::PLAY_URI_PREFIX))
        .filter(|id| valid_id(id))
        .ok_or("Invalid ntx game link. Open a game from the library.")?;
    Ok(id.to_owned())
}
impl Collection {
    /// Schema 1 executed exclusively through direct Dolphin GameCube, but may
    /// retain failed imports of other media. Start with that legacy route;
    /// a freshly verified relink can classify a never-bound failed import.
    /// IDs, selected game, ordinary data and checkpoint fingerprints stay intact.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let mut value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if value["schema_version"] == 1 {
            let entries = value["entries"]
                .as_array_mut()
                .ok_or("invalid legacy entries")?;
            for entry in entries {
                let object = entry.as_object_mut().ok_or("invalid legacy session")?;
                if object.contains_key("system") || object.contains_key("backend") {
                    return Err("ambiguous legacy routing; original retained".into());
                }
                object.insert("system".into(), serde_json::json!("GameCube"));
                object.insert("backend".into(), serde_json::json!("Dolphin"));
            }
            value["schema_version"] = serde_json::json!(2);
        }
        let collection: Self = serde_json::from_value(value).map_err(|e| e.to_string())?;
        collection.validate()?;
        Ok(collection)
    }

    /// A launch/UI request names an existing entry, never a document or profile
    /// path. Unknown, removed and malformed IDs leave the live owner untouched.
    pub fn resolve_game(&self, id: &str) -> Result<usize, &'static str> {
        if !valid_id(id) {
            return Err("Invalid game identity.");
        }
        self.entries
            .iter()
            .position(|entry| entry.game_id.0 == id)
            .ok_or("This game is no longer in the library. Open the library to choose a game.")
    }
    /// The host has freshly acquired read access and fingerprinted these bytes.
    /// A changed platform locator never allocates another game or save profile.
    pub fn relink_game(&mut self, id: &str, uri: &str, observed: &str) -> Result<(), &'static str> {
        let index = self.resolve_game(id)?;
        if !valid_media_locator(uri)
            || !crate::storage::valid_digest(observed)
            || self
                .entries
                .iter()
                .enumerate()
                .any(|(i, e)| i != index && e.uri == uri)
        {
            return Err("Invalid or already assigned media locator.");
        }
        let game = &mut self.entries[index];
        if !game.media_relink_required {
            return Err("This game does not need relinking.");
        }
        match &game.media_sha256 {
            Some(expected) if expected != observed => {
                return Err("Selected media differs; original identity and progress retained.");
            }
            None if game.uri != uri => {
                return Err(
                    "A verified original media fingerprint is required to change this locator.",
                );
            }
            _ => {}
        }
        game.uri = uri.into();
        game.media_sha256 = Some(observed.into());
        game.media_relink_required = false;
        Ok(())
    }
    /// Explicit owner confirmation for an older, stopped entry that never
    /// recorded its media digest. This establishes a first fingerprint; it
    /// cannot replace a known fingerprint or rebind a machine continuation.
    pub fn confirm_legacy_media(
        &mut self,
        id: &str,
        uri: &str,
        observed: &str,
        system: crate::backend::System,
        backend: crate::backend::Backend,
    ) -> Result<(), &'static str> {
        let index = self.resolve_game(id)?;
        let game = &self.entries[index];
        if !game.media_relink_required
            || game.media_sha256.is_some()
            || game.checkpoint.is_some()
            || game.runtime_incomplete
            || game.restart_required
            || !game.stopped
            || (game.system, game.backend) != (system, backend)
            || backend.system() != system
        {
            return Err(
                "This game needs verified original media; its progress and recovery are retained.",
            );
        }
        if !valid_media_locator(uri)
            || !crate::storage::valid_digest(observed)
            || self
                .entries
                .iter()
                .enumerate()
                .any(|(i, e)| i != index && e.uri == uri)
        {
            return Err("Invalid or already assigned media locator.");
        }
        let game = &mut self.entries[index];
        game.uri = uri.into();
        game.media_sha256 = Some(observed.into());
        game.media_relink_required = false;
        Ok(())
    }
    /// The host classified the actual freshly granted bytes, not a filename.
    /// Legacy failed imports with no bound scene/media may acquire their real
    /// route without allocating identities. Existing bound progress refuses a
    /// system change; profile files are never converted or deleted here.
    pub fn relink_game_routed(
        &mut self,
        id: &str,
        uri: &str,
        observed: &str,
        system: crate::backend::System,
        backend: crate::backend::Backend,
    ) -> Result<(), &'static str> {
        let index = self.resolve_game(id)?;
        if backend.system() != system {
            return Err("Invalid verified media route.");
        }
        let game = &self.entries[index];
        let changed = (game.system, game.backend) != (system, backend);
        if changed
            && !(game.media_relink_required
                && game.media_sha256.is_none()
                && game.checkpoint.is_none()
                && game.restart_required
                && game
                    .last_error
                    .as_ref()
                    .is_some_and(|e| e.contains(crate::legacy::UNSUPPORTED_DISC_IMPORT))
                && game.system == crate::backend::System::GameCube
                && game.backend == crate::backend::Backend::Dolphin
                && system == crate::backend::System::Nintendo64
                && backend == crate::backend::Backend::Mupen64PlusNext)
        {
            return Err(
                "Verified media system differs from bound progress; original identity and recovery retained.",
            );
        }
        self.relink_game(id, uri, observed)?;
        if changed {
            let game = &mut self.entries[index];
            game.system = system;
            game.backend = backend;
            let mut message="N64 cartridge verified for this retained import. Earlier profile files are kept. Restart explicitly with current N64 saves.".to_owned();
            if let Some(earlier) = &game.last_error {
                message.push_str("\nEarlier retained error: ");
                message.push_str(earlier);
            }
            game.fail(message);
        }
        Ok(())
    }
    pub fn open_game(&self, id: &str, live_game: Option<&str>) -> Result<GameOpen, &'static str> {
        let target = self.resolve_game(id)?;
        if self.entries[target].media_relink_required {
            return Err(
                "This installation needs media access. Use Add game to select the same original document; your game identity and progress are retained.",
            );
        }
        if let Some(live) = live_game {
            let current = self.resolve_game(live)?;
            if self.entries[current].restart_required {
                return Err(
                    "The current game needs recovery. Use Stop or Restart with current saves before opening another game.",
                );
            }
            if current == target {
                return Ok(GameOpen::ResumeCurrent);
            }
        }
        Ok(GameOpen::Select(target))
    }
    /// A ntx-only OS shortcut observation never removes a game or its saves.
    pub fn observe_shortcuts(&mut self, pinned_games: &[String]) -> bool {
        let pinned: BTreeSet<_> = pinned_games.iter().map(String::as_str).collect();
        let mut changed = false;
        for entry in &mut self.entries {
            let value = Some(pinned.contains(entry.game_id.0.as_str()));
            changed |= entry.shortcut_pinned != value;
            entry.shortcut_pinned = value;
        }
        changed
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != 2 {
            return Err("unsupported collection schema; original retained");
        }
        if self.entries.len() > 4096 {
            return Err("collection has too many entries; original retained");
        }
        let mut games = BTreeSet::new();
        let mut documents = BTreeSet::new();
        let mut profiles = BTreeSet::new();
        for e in &self.entries {
            if e.backend.system() != e.system
                || e.media_sha256
                    .as_ref()
                    .is_some_and(|s| !crate::storage::valid_digest(s))
                || !valid_id(&e.game_id.0)
                || !valid_id(&e.profile_id.0)
                || !valid_media_locator(&e.uri)
                || e.title.len() > 16384
                || e.last_error.as_ref().is_some_and(|s| s.len() > 32768)
                || e.checkpoint
                    .as_ref()
                    .is_some_and(|s| !crate::storage::valid_digest(s))
            {
                return Err("invalid collection metadata; original retained");
            }
            if !games.insert(e.game_id.0.as_str())
                || !documents.insert(e.uri.as_str())
                || !profiles.insert(e.profile_id.0.as_str())
            {
                return Err("ambiguous collection identity; original retained");
            }
        }
        if self
            .selected
            .as_ref()
            .is_some_and(|id| !games.contains(id.as_str()))
        {
            return Err("selected game is absent; original retained");
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_file_confirmation_preserves_identity_and_current_save_intent() {
        use crate::backend::{Backend, System};
        let mut game = entry();
        game.media_relink_required = true;
        game.stopped = true;
        game.paused = true;
        game.last_error = Some("Retained earlier diagnostic".into());
        let mut collection = Collection {
            schema_version: 2,
            selected: Some(game.game_id.0.clone()),
            entries: vec![game.clone()],
        };
        let chosen = "content://new-provider/original-game";
        let digest = "e".repeat(64);
        assert!(
            collection
                .relink_game(&game.game_id.0, chosen, &digest)
                .is_err()
        );
        collection
            .confirm_legacy_media(
                &game.game_id.0,
                chosen,
                &digest,
                System::GameCube,
                Backend::Dolphin,
            )
            .unwrap();
        let result = &collection.entries[0];
        assert_eq!(result.game_id, game.game_id);
        assert_eq!(result.profile_id.0, game.profile_id.0);
        assert_eq!(result.last_error, game.last_error);
        assert_eq!(collection.selected, Some(game.game_id.0));
        assert!(result.stopped && result.paused && !result.media_relink_required);
        assert_eq!(result.media_sha256.as_deref(), Some(digest.as_str()));
        assert!(!result.boot_allowed());
    }
    #[test]
    fn legacy_confirmation_cannot_override_fingerprint_scene_live_owner_or_route() {
        use crate::backend::{Backend, System};
        let mut game = entry();
        game.media_relink_required = true;
        game.stopped = true;
        for change in 0..8 {
            let mut candidate = game.clone();
            match change {
                0 => candidate.media_sha256 = Some("f".repeat(64)),
                1 => candidate.checkpoint = Some("f".repeat(64)),
                2 => candidate.runtime_incomplete = true,
                3 => candidate.stopped = false,
                4 => candidate.restart_required = true,
                5 => candidate.media_relink_required = false,
                _ => {}
            }
            let mut collection = Collection {
                schema_version: 2,
                entries: vec![candidate],
                selected: None,
            };
            let before = serde_json::to_vec(&collection).unwrap();
            let route = if change == 6 {
                (System::Nintendo64, Backend::Mupen64PlusNext)
            } else {
                (System::GameCube, Backend::Dolphin)
            };
            let digest = if change == 7 {
                "invalid".into()
            } else {
                "e".repeat(64)
            };
            assert!(
                collection
                    .confirm_legacy_media(
                        &game.game_id.0,
                        "content://new-provider/original-game",
                        &digest,
                        route.0,
                        route.1
                    )
                    .is_err()
            );
            assert_eq!(serde_json::to_vec(&collection).unwrap(), before);
        }
    }
    #[test]
    fn host_locator_changes_do_not_reidentify_a_game_profile_or_scene() {
        let mut game = entry();
        game.uri = "file:///owned-games/cartridge.z64".into();
        game.media_sha256 = Some("e".repeat(64));
        game.checkpoint = Some("f".repeat(64));
        game.media_relink_required = true;
        let mut collection = Collection {
            schema_version: 2,
            selected: Some(game.game_id.0.clone()),
            entries: vec![game.clone()],
        };
        collection.validate().unwrap();
        collection
            .relink_game(
                &game.game_id.0,
                "content://scoped-provider/verified-cartridge",
                &"e".repeat(64),
            )
            .unwrap();
        assert_eq!(collection.entries[0].game_id, game.game_id);
        assert_eq!(collection.entries[0].profile_id.0, game.profile_id.0);
        assert_eq!(collection.entries[0].checkpoint, game.checkpoint);
        assert_eq!(collection.entries[0].system, game.system);
        assert_eq!(collection.entries[0].backend, game.backend);
        for bad in [
            "/unscoped/path",
            "content://",
            "1invalid://location",
            "content://bad\0location",
        ] {
            collection.entries[0].uri = bad.into();
            assert!(collection.validate().is_err());
        }
    }
    #[test]
    fn verified_legacy_failed_cartridge_relinks_without_new_ids_but_bound_scene_refuses_system_change()
     {
        use crate::backend::{Backend, System};
        let mut game = entry();
        game.media_relink_required = true;
        game.fail(crate::legacy::UNSUPPORTED_DISC_IMPORT.into());
        let mut collection = Collection {
            schema_version: 2,
            selected: Some(game.game_id.0.clone()),
            entries: vec![game],
        };
        let original = collection.entries[0].clone();
        collection
            .relink_game_routed(
                &original.game_id.0,
                &original.uri,
                &"e".repeat(64),
                System::Nintendo64,
                Backend::Mupen64PlusNext,
            )
            .unwrap();
        let game = &collection.entries[0];
        assert_eq!(game.game_id, original.game_id);
        assert_eq!(game.profile_id.0, original.profile_id.0);
        assert_eq!(game.system, System::Nintendo64);
        assert!(!game.boot_allowed());
        assert!(
            game.last_error
                .as_ref()
                .unwrap()
                .contains(crate::legacy::UNSUPPORTED_DISC_IMPORT)
        );
        collection.entries[0] = original.clone();
        collection.entries[0].checkpoint = Some("f".repeat(64));
        assert!(
            collection
                .relink_game_routed(
                    &original.game_id.0,
                    &original.uri,
                    &"e".repeat(64),
                    System::Nintendo64,
                    Backend::Mupen64PlusNext
                )
                .is_err()
        );
        assert_eq!(collection.entries[0].system, System::GameCube);
        assert!(collection.entries[0].media_relink_required);
        collection.entries[0] = original.clone();
        collection.entries[0].media_sha256 = Some("e".repeat(64));
        assert!(
            collection
                .relink_game_routed(
                    &original.game_id.0,
                    &original.uri,
                    &"e".repeat(64),
                    System::Nintendo64,
                    Backend::Mupen64PlusNext
                )
                .is_err()
        );
        collection.entries[0] = original.clone();
        collection.entries[0].last_error = Some("Earlier save barrier failure".into());
        assert!(
            collection
                .relink_game_routed(
                    &original.game_id.0,
                    &original.uri,
                    &"e".repeat(64),
                    System::Nintendo64,
                    Backend::Mupen64PlusNext
                )
                .is_err()
        );
    }
    fn entry() -> SavedSession {
        SavedSession {
            system: crate::backend::System::GameCube,
            backend: crate::backend::Backend::Dolphin,
            media_relink_required: false,
            media_sha256: None,
            game_id: GameId("a".repeat(32)),
            profile_id: SaveProfileId("b".repeat(32)),
            uri: "content://fixture/game".into(),
            title: "Original fixture".into(),
            paused: false,
            last_error: None,
            restart_required: false,
            runtime_incomplete: false,
            stopped: false,
            checkpoint: None,
            shortcut_requested: false,
            shortcut_pinned: None,
        }
    }
    #[test]
    fn schema_upgrade_keeps_games_independent_and_never_infers_new_media_grants() {
        let mut old = serde_json::to_value(Collection {
            entries: vec![entry()],
            ..Default::default()
        })
        .unwrap();
        old["schema_version"] = serde_json::json!(1);
        let object = old["entries"][0].as_object_mut().unwrap();
        object.remove("system");
        object.remove("backend");
        object.remove("media_relink_required");
        let mut c = Collection::decode(&serde_json::to_vec(&old).unwrap()).unwrap();
        assert_eq!(c.entries[0].system, crate::backend::System::GameCube);
        let mut n64 = entry();
        n64.game_id.0 = "c".repeat(32);
        n64.profile_id.0 = "d".repeat(32);
        n64.uri = "content://fixture/cartridge".into();
        n64.system = crate::backend::System::Nintendo64;
        n64.backend = crate::backend::Backend::Mupen64PlusNext;
        c.entries.push(n64);
        assert!(c.validate().is_ok());
        c.entries[0].media_relink_required = true;
        assert!(c.open_game(&"a".repeat(32), None).is_err());
        assert_eq!(c.open_game(&"c".repeat(32), None), Ok(GameOpen::Select(1)));
        assert_eq!(
            parse_play_uri(&format!(
                "{}{id}",
                crate::legacy::PLAY_URI_PREFIX,
                id = "a".repeat(32)
            ))
            .unwrap(),
            "a".repeat(32)
        );
        assert!(
            play_uri(&"a".repeat(32))
                .unwrap()
                .starts_with("machinerhythm-ntx://")
        );
        c.entries[1].backend = crate::backend::Backend::Dolphin;
        assert!(c.validate().is_err());
        old["schema_version"] = serde_json::json!(2);
        assert!(
            Collection::decode(&serde_json::to_vec(&old).unwrap()).is_err(),
            "new schema cannot omit routing"
        );
    }
    #[test]
    fn relinking_a_new_host_locator_checks_media_without_reidentifying_progress() {
        let mut c = Collection {
            entries: vec![entry()],
            ..Default::default()
        };
        c.entries[0].media_relink_required = true;
        let id = c.entries[0].game_id.0.clone();
        assert!(
            c.relink_game(&id, "content://other-host/game", &"c".repeat(64))
                .is_err()
        );
        c.entries[0].media_sha256 = Some("c".repeat(64));
        c.entries[0].checkpoint = Some("e".repeat(64));
        c.entries[0].restart_required = true;
        assert!(
            c.relink_game(&id, "content://other-host/game", &"d".repeat(64))
                .is_err()
        );
        assert_eq!(c.entries[0].uri, "content://fixture/game");
        c.relink_game(&id, "content://other-host/game", &"c".repeat(64))
            .unwrap();
        assert_eq!(c.entries[0].profile_id.0, "b".repeat(32));
        assert_eq!(c.entries[0].game_id.0, id);
        assert_eq!(c.entries[0].checkpoint, Some("e".repeat(64)));
        assert!(c.entries[0].restart_required && !c.entries[0].media_relink_required);
    }
    #[test]
    fn failure_survives_relaunch_and_explicit_restart_keeps_identity_and_error() {
        let mut session = entry();
        session.checkpoint = Some("c".repeat(64));
        session.fail("write failed".into());
        let mut restored: SavedSession =
            serde_json::from_slice(&serde_json::to_vec(&session).unwrap()).unwrap();
        assert!(restored.paused && restored.restart_required);
        assert_eq!(restored.checkpoint, session.checkpoint);
        restored.request_restart();
        assert!(restored.checkpoint.is_none());
        assert!(!restored.paused && !restored.restart_required);
        assert_eq!(restored.last_error.as_deref(), Some("write failed"));
        assert_eq!(restored.profile_id.0, session.profile_id.0);
        assert_eq!(restored.game_id.0, session.game_id.0);
    }
    #[test]
    fn unsupported_schema_and_ambiguous_or_unsafe_identity_are_retained_as_errors() {
        let mut c = Collection {
            entries: vec![entry()],
            selected: Some("a".repeat(32)),
            ..Default::default()
        };
        assert!(c.validate().is_ok());
        c.schema_version = 3;
        assert!(c.validate().is_err());
        c.schema_version = 2;
        c.entries.push(entry());
        assert!(c.validate().is_err());
        c.entries.pop();
        c.entries[0].profile_id.0 = "../other-profile".into();
        assert!(c.validate().is_err());
        c.entries[0] = entry();
        c.selected = Some("c".repeat(32));
        assert!(c.validate().is_err());
    }
    #[test]
    fn checkpoint_defers_interruption_recovery_to_pre_native_generation_validation() {
        let mut original = entry();
        original.runtime_incomplete = true;
        original.checkpoint = Some("d".repeat(64));
        let mut loaded: SavedSession =
            serde_json::from_slice(&serde_json::to_vec(&original).unwrap()).unwrap();
        assert!(!loaded.recover_interrupted());
        assert!(loaded.boot_allowed());
        assert_eq!(loaded.checkpoint, original.checkpoint);
        loaded.fail("Current card generation differs".into());
        assert!(!loaded.boot_allowed());
        assert_eq!(loaded.checkpoint, original.checkpoint);
        loaded.request_restart();
        assert!(loaded.boot_allowed() && loaded.checkpoint.is_none());
        assert_eq!(loaded.profile_id.0, original.profile_id.0);
    }
    #[test]
    fn library_requests_resolve_stable_ids_and_refuse_paths_unknown_ids_or_shared_profiles() {
        let a = entry();
        let mut b = entry();
        b.game_id.0 = "c".repeat(32);
        b.profile_id.0 = "d".repeat(32);
        b.uri = "content://fixture/other".into();
        let mut c = Collection {
            entries: vec![a, b],
            selected: None,
            ..Default::default()
        };
        assert!(c.validate().is_ok());
        assert_eq!(c.resolve_game(&"c".repeat(32)), Ok(1));
        for _ in 0..3 {
            assert_eq!(
                c.open_game(&"a".repeat(32), Some(&"a".repeat(32))),
                Ok(GameOpen::ResumeCurrent)
            );
        }
        assert_eq!(
            c.open_game(&"c".repeat(32), Some(&"a".repeat(32))),
            Ok(GameOpen::Select(1))
        );
        c.entries[0].fail("checkpoint publication failed".into());
        assert!(c.open_game(&"c".repeat(32), Some(&"a".repeat(32))).is_err());
        assert!(c.open_game(&"a".repeat(32), Some(&"a".repeat(32))).is_err());
        c.entries[0].request_restart();
        assert!(c.open_game(&"e".repeat(32), None).is_err());
        assert!(c.open_game(&"e".repeat(32), Some(&"a".repeat(32))).is_err());
        for id in [
            "../profiles/card",
            "content://fixture/game",
            &"e".repeat(32),
            &"A".repeat(32),
        ] {
            assert!(c.resolve_game(id).is_err());
        }
        c.entries[1].profile_id = c.entries[0].profile_id.clone();
        assert!(c.validate().is_err(), "initial profiles must be isolated");
        c.entries.swap(0, 1);
        assert_eq!(
            c.resolve_game(&"c".repeat(32)),
            Ok(0),
            "identity survives row reordering"
        );
    }
    #[test]
    fn links_carry_only_canonical_game_ids_and_pin_requests_are_not_confirmations() {
        let id = "a".repeat(32);
        let uri = play_uri(&id).unwrap();
        assert_eq!(parse_play_uri(&uri).unwrap(), id);
        let mut legacy = serde_json::to_value(entry()).unwrap();
        for name in ["checkpoint", "shortcut_requested", "shortcut_pinned"] {
            legacy.as_object_mut().unwrap().remove(name);
        }
        let legacy: SavedSession = serde_json::from_value(legacy).unwrap();
        assert!(!legacy.shortcut_requested && legacy.shortcut_pinned.is_none());
        assert!(legacy.checkpoint.is_none());
        for value in [
            "content://fixture/game",
            "machinerhythm-ntx://play/../card",
            &format!("{uri}?path=card"),
            &format!("{uri}#other"),
            &format!("{uri}/extra"),
            "machinerhythm-ntx://play/%61",
        ] {
            assert!(parse_play_uri(value).is_err());
        }
        let mut c = Collection {
            entries: vec![entry()],
            ..Default::default()
        };
        c.entries[0].shortcut_requested = true;
        let mut restored: Collection =
            serde_json::from_slice(&serde_json::to_vec(&c).unwrap()).unwrap();
        assert_eq!(restored.entries[0].shortcut_pinned, None);
        assert!(restored.observe_shortcuts(std::slice::from_ref(&id)));
        assert_eq!(restored.entries[0].shortcut_pinned, Some(true));
        assert!(!restored.observe_shortcuts(std::slice::from_ref(&id)));
        assert!(restored.observe_shortcuts(&[]));
        assert_eq!(restored.entries.len(), 1);
        assert_eq!(restored.entries[0].profile_id.0, "b".repeat(32));
        assert!(restored.entries[0].shortcut_requested);
    }
    #[test]
    fn interrupted_runtime_requires_explicit_restart_without_changing_save_identity() {
        let mut original = entry();
        original.runtime_incomplete = true;
        original.last_error = Some("earlier diagnosed write failure".into());
        let mut loaded: SavedSession =
            serde_json::from_slice(&serde_json::to_vec(&original).unwrap()).unwrap();
        assert!(loaded.recover_interrupted());
        assert!(loaded.paused && loaded.restart_required && loaded.runtime_incomplete);
        assert_eq!(loaded.profile_id.0, original.profile_id.0);
        assert_eq!(loaded.game_id.0, original.game_id.0);
        let message = loaded.last_error.clone();
        assert!(
            message
                .as_ref()
                .unwrap()
                .contains("Earlier retained error: earlier diagnosed write failure")
        );
        assert!(!loaded.recover_interrupted());
        assert_eq!(loaded.last_error, message);
        loaded.request_restart();
        assert!(!loaded.recover_interrupted());
        assert!(loaded.last_error.is_some());
    }
}
