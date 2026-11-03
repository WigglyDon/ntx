// SPDX-License-Identifier: MIT
use super::{NEXT_RUNTIME, RuntimeEvents, RuntimeInfo, platform};
use crate::n64_host::{Adapter, Resources};
use ndk::native_window::NativeWindow;
use ntx::{
    backend::mupen::Core,
    frame_timing::FrameTimings,
    library::SavedSession,
    n64::Cartridge,
    n64_checkpoint::{self, Binding, Checkpoint},
    n64_save::SaveStore,
    scheduling::FramePacer,
};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, String>;
pub(super) struct Runtime {
    pub info: RuntimeInfo,
    pub core: Option<Core>,
    pub pending: Option<Checkpoint>,
    pub failed: bool,
    pub resources: Rc<RefCell<Resources>>,
    store: std::mem::ManuallyDrop<SaveStore>,
    path: PathBuf,
    binding: Binding,
    paused: bool,
    pacer: FramePacer,
    timings: FrameTimings,
    accepted: Option<ntx::n64_save::Generation>,
}
impl Runtime {
    pub fn boot(root: &Path, entry: &SavedSession, window: &NativeWindow) -> Result<Self> {
        if entry.media_relink_required {
            return Err("Choose the original cartridge to grant this installation access; its game identity and progress are retained".into());
        }
        let core_sha =
            option_env!("NTX_MUPEN_SHA256").ok_or("This package lacks its N64 core fingerprint")?;
        if core_sha.len() != 64 || !core_sha.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err("Invalid packaged N64 fingerprint".into());
        }
        let cartridge =
            Cartridge::read(platform::open_document(&entry.uri).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        if entry
            .media_sha256
            .as_ref()
            .is_some_and(|h| *h != cartridge.sha256)
        {
            return Err("N64 cartridge bytes changed; original identity and saves retained".into());
        }
        let path = root.join("profiles").join(&entry.profile_id.0);
        let store = SaveStore::open(&path).map_err(|e| e.to_string())?;
        let ordinary = store.load().map_err(|e| e.to_string())?;
        let resources = Rc::new(RefCell::new(Resources::new(window)?));
        let core = Core::load(
            Box::new(Adapter(resources.clone())),
            &cartridge,
            &root.join("N64/System"),
            &path.join("N64/backend"),
            ordinary.as_deref(),
        )?;
        let binding = Binding {
            game_id: entry.game_id.0.clone(),
            profile_id: entry.profile_id.0.clone(),
            media_sha256: cartridge.sha256.clone(),
            core_sha256: core_sha.into(),
            options_sha256: core.options_sha256(),
            architecture: "aarch64-android-gles3-mupen-state1".into(),
        };
        let pending = entry
            .checkpoint
            .as_ref()
            .map(|id| n64_checkpoint::load(&path, id, &binding, &store))
            .transpose()
            .map_err(|e| e.to_string())?;
        let info = RuntimeInfo {
            events: Box::new(RuntimeEvents {
                game_id: entry.game_id.0.clone(),
                epoch: NEXT_RUNTIME.fetch_add(1, Ordering::Relaxed),
                stopped: AtomicBool::new(false),
                teardown_error: Mutex::new(None),
            }),
            media_sha256: cartridge.sha256,
        };
        platform::trace_runtime(info.events.epoch);
        Ok(Self {
            info,
            core: Some(core),
            pending,
            failed: false,
            resources,
            store: std::mem::ManuallyDrop::new(store),
            path,
            binding,
            paused: true,
            pacer: FramePacer::new(Instant::now()),
            timings: FrameTimings::default(),
            accepted: None,
        })
    }
    fn core(&mut self) -> Result<&mut Core> {
        self.core
            .as_mut()
            .ok_or("N64 runtime already unloaded".into())
    }
    pub fn pause(&mut self, paused: bool) {
        self.paused = paused;
        self.resources.borrow_mut().muted = paused;
        if paused {
            self.timings.suspend();
            if let Some(core) = self.core.as_mut() {
                core.silence();
            }
        } else {
            self.pacer.reset(Instant::now());
        }
    }
    pub fn surface(&mut self, window: Option<&NativeWindow>) -> bool {
        let result = self.resources.borrow_mut().graphics.set_window(window);
        if let Err(e) = result {
            self.report(e);
            false
        } else {
            true
        }
    }
    fn report(&mut self, error: String) {
        self.pause(true);
        platform::push(platform::Command::NativeError {
            game_id: self.info.events.game_id.clone(),
            epoch: self.info.events.epoch,
            message: error,
        });
    }
    pub fn dispatch(&mut self) {
        if self.paused || self.failed || self.pending.is_some() || !self.pacer.ready(Instant::now())
        {
            return;
        }
        let fps = self.core.as_ref().map_or(60., |c| c.av_info().timing.fps);
        let start = Instant::now();
        self.timings.start(start, self.pacer.lateness(start));
        if let Err(error) = self.pacer.schedule(start, fps) {
            self.report(error.into());
            return;
        }
        self.resources.borrow().shader_probe.begin_frame();
        let core_start = Instant::now();
        let run = self.core().and_then(Core::run_frame);
        let core_elapsed = core_start.elapsed();
        self.resources.borrow().shader_probe.end_frame(core_elapsed);
        self.timings.core.record(core_elapsed);
        if let Err(e) = run {
            self.report(e);
            return;
        }
        let audio_start = Instant::now();
        let delivered = self.resources.borrow_mut().deliver_audio();
        self.timings.audio.record(audio_start.elapsed());
        if let Err(error) = delivered {
            self.report(error);
        }
    }
    pub fn poll_timeout(&self) -> Duration {
        if self.paused || self.failed || self.pending.is_some() {
            return Duration::from_millis(8);
        }
        self.pacer.wait(Instant::now(), Duration::from_millis(8))
    }
    pub fn commit(&mut self) -> Result<()> {
        self.pause(true);
        if self.failed {
            return Ok(());
        }
        if self.pending.is_some() {
            return self.verify_pending();
        }
        self.store.verify_current().map_err(|e| e.to_string())?;
        let bytes = self.core()?.ordinary_image()?;
        self.store.commit(&bytes).map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn capture(&mut self) -> Result<String> {
        self.pause(true);
        if self.pending.is_some() {
            return Err("N64 restore is still pending".into());
        }
        if self.failed {
            return Err("N64 runtime requires recovery".into());
        }
        self.store.verify_current().map_err(|e| e.to_string())?;
        let payload = self.core()?.capture()?;
        let image = self.core()?.ordinary_image()?;
        self.store.commit(&image).map_err(|e| e.to_string())?;
        n64_checkpoint::publish(&self.path, &self.binding, &self.store, &payload)
            .map_err(|e| e.to_string())
    }
    pub fn accept_departure(&mut self) {
        self.accepted = self.store.generation().cloned();
    }
    pub fn verify_pending(&self) -> Result<()> {
        self.pending
            .as_ref()
            .ok_or("No pending N64 scene")?
            .verify_current(&self.store)
            .map_err(|e| e.to_string())
    }
    pub fn verify_departure(&self) -> Result<()> {
        self.store.verify_current().map_err(|e| e.to_string())?;
        if self.store.generation() != self.accepted.as_ref() {
            return Err("N64 ordinary generation differs from acknowledged scene".into());
        }
        Ok(())
    }
    pub fn restore(&mut self) -> Result<Option<()>> {
        if self.pending.is_none() {
            return Ok(None);
        }
        self.verify_pending()?;
        // Initialize the machine once before restoring, with current ordinary data
        // already injected. No initialization bytes are committed on restore failure.
        self.resources.borrow_mut().muted = true;
        self.core()?.run_frame()?;
        self.verify_pending()?;
        let ordinary = self
            .store
            .load()
            .map_err(|e| e.to_string())?
            .ok_or("N64 scene has no current ordinary data")?;
        // SAFETY: still quiescent, before native restore, and verified against
        // the pending scene's exact ordinary generation. Cold initialization
        // must not contribute an unrelated boot-time write to this scene.
        unsafe {
            self.core()?.reseed_for_restore(&ordinary)?;
        }
        let pending = self.pending.take().unwrap();
        // SAFETY: exact build/options/media/game/profile plus current actual ordinary
        // generation validated before initialization and again before native mutation.
        if let Err(e) = unsafe { self.core()?.restore(&pending.payload) } {
            self.failed = true;
            self.pending = Some(pending);
            return Err(e);
        }
        if self.core()?.ordinary_image()? != ordinary {
            self.failed = true;
            self.pending = Some(pending);
            return Err("N64 restore changed the bound ordinary image; runtime quarantined and current saves retained".into());
        }
        self.accept_departure();
        Ok(Some(()))
    }
    pub fn join(&mut self) -> Result<()> {
        self.pause(true);
        if let Some(core) = self.core.as_mut() {
            core.stop()?;
            let (video_callbacks, audio_acknowledged) = core.observations();
            let resources = self.resources.borrow();
            let audio_written = resources.written_audio_frames;
            let backend_gl_errors = resources.graphics.backend_gl_errors;
            if self.timings.core.samples() != 0 {
                platform::diagnostic(&format!(
                    "N64 timing fps={} (first4096; includes startup; intervals exclude pauses; core includes present; elapsed host durations, not display/audio proof): interval[{}] late[{}] core[{}] present[{}] swap[{}] audio[{}]",
                    core.av_info().timing.fps,
                    self.timings.intervals,
                    self.timings.lateness,
                    self.timings.core,
                    resources.graphics.present_timing,
                    resources.graphics.swap_timing,
                    self.timings.audio,
                ));
                platform::diagnostic(&format!(
                    "N64 core shader timing (first4096 core calls; elapsed, not GPU/CPU attribution): {}",
                    resources.shader_probe.summary()
                ));
            }
            platform::diagnostic(&format!(
                "N64 joined: {video_callbacks} accepted video callbacks, {audio_acknowledged} acknowledged stereo frames (including muted/background), {audio_written} stereo frames written to AudioTrack on host stack, {backend_gl_errors} GL flags observed before host presentation; not audible proof; profile writer retained"
            ));
            drop(resources);
            if !self.failed && self.pending.is_none() {
                let bytes = core.ordinary_image()?;
                self.store.commit(&bytes).map_err(|e| e.to_string())?;
            }
        }
        drop(self.core.take());
        if self.failed {
            return Err("N64 failed restore was discarded without publishing native save memory; current saves retained".into());
        }
        Ok(())
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        let _ = self.join();
        if let Some(core) = self.core.take() {
            // Failed join/publication: preserve native memory, callback resources
            // and the writer FD instead of allowing another profile owner.
            std::mem::forget(core);
        } else {
            // SAFETY: joined core has been destroyed; caller retained this writer
            // through its durable session publication before dropping Runtime.
            unsafe {
                std::mem::ManuallyDrop::drop(&mut self.store);
            }
        }
    }
}
