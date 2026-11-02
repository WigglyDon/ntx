// SPDX-License-Identifier: MIT
use crate::platform::{self, Command};
use android_activity::{
    AndroidApp, InputStatus, MainEvent, PollEvent,
    input::{InputEvent, MotionAction},
};
use ndk::native_window::NativeWindow;
use ntx::checkpoint::{self, Binding, Checkpoint};
use ntx::library::{Collection, GameId, GameOpen, SaveProfileId, SavedSession, valid_id};
use ntx::lifecycle::{StopResult, StopRuntime, stop_game};
use ntx::{
    Pad, ProfileLock,
    backend::dolphin::*,
    touch::{DownReport, Target},
    viewport::{PlayerView, WindowGeometry},
};
use std::{
    ffi::{CStr, CString, c_char},
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
static OWNER: AtomicBool = AtomicBool::new(false);
static NEXT_RUNTIME: AtomicU64 = AtomicU64::new(1);
static UI: Mutex<PlayerView> = Mutex::new(PlayerView::new());
static MISSING_PRESENT_REPORTED: AtomicBool = AtomicBool::new(false);
struct MediaConfirmation {
    game_id: String,
    uri: String,
    digest: String,
    system: ntx::backend::System,
    backend: ntx::backend::Backend,
}
pub(crate) fn haptics_enabled() -> bool {
    UI.lock().is_ok_and(|ui| ui.haptics())
}
pub(crate) fn controls_opacity() -> f32 {
    UI.lock()
        .map_or(0.85, |ui| ui.controls_opacity() as f32 / 100.)
}
// The core asks for one complete input packet at its actual pad poll. Rendering
// and the Android owner loop never consume quick-press pulses.
unsafe extern "C" fn sample_input(controls: *mut f64) {
    if controls.is_null() {
        return;
    }
    let pad = UI.lock().map(|mut ui| ui.sample_pad()).unwrap_or_default();
    // SAFETY: ABI8 lends exactly18 writable doubles for this synchronous call.
    unsafe {
        std::ptr::copy_nonoverlapping(pad.values().as_ptr(), controls, 18);
    }
}
pub(crate) fn measure_window(geometry: Option<WindowGeometry>) {
    if let Ok(mut view) = crate::n64_host::view().lock() {
        view.measure(geometry);
    }
    let observed = UI.lock().ok().map(|mut ui| {
        ui.measure(geometry);
        ui.has_presented_geometry()
    });
    if let Some(ready) = observed {
        platform::trace_geometry(&format!(
            "measurement={} presented={ready}",
            geometry.is_some()
        ));
    }
}

struct SaveProfile {
    path: PathBuf,
    _writer: ProfileLock,
}
#[path = "n64_runtime.rs"]
mod n64_runtime;
struct RuntimeInfo {
    events: Box<RuntimeEvents>,
    media_sha256: String,
}
enum LiveRuntime {
    Dolphin(Box<DolphinRuntime>),
    N64(Box<n64_runtime::Runtime>),
}
impl std::ops::Deref for DolphinRuntime {
    type Target = RuntimeInfo;
    fn deref(&self) -> &RuntimeInfo {
        &self.info
    }
}
impl std::ops::DerefMut for DolphinRuntime {
    fn deref_mut(&mut self) -> &mut RuntimeInfo {
        &mut self.info
    }
}
impl std::ops::Deref for LiveRuntime {
    type Target = RuntimeInfo;
    fn deref(&self) -> &RuntimeInfo {
        match self {
            Self::Dolphin(r) => &r.info,
            Self::N64(r) => &r.info,
        }
    }
}
impl LiveRuntime {
    fn poll_timeout(&self) -> Duration {
        match self {
            Self::N64(r) => r.poll_timeout(),
            Self::Dolphin(_) => Duration::from_millis(8),
        }
    }
    fn pending(&self) -> bool {
        match self {
            Self::Dolphin(r) => r.pending_restore.is_some(),
            Self::N64(r) => r.pending.is_some(),
        }
    }
    fn failed(&self) -> bool {
        match self {
            Self::Dolphin(r) => r.failed_restore || unsafe { ntx_engine_recovery_required() != 0 },
            Self::N64(r) => r.failed,
        }
    }
    fn pause(&mut self, paused: bool) {
        match self {
            Self::Dolphin(_) => unsafe {
                ntx_engine_pause(i32::from(paused));
                if paused {
                    ntx_engine_input(Pad::default().values().as_ptr());
                }
            },
            Self::N64(r) => r.pause(paused),
        }
    }
    fn state(&self) -> i32 {
        match self {
            Self::Dolphin(_) => unsafe { ntx_engine_state() },
            Self::N64(r) => {
                if r.core.is_some() {
                    1
                } else {
                    0
                }
            }
        }
    }
    fn dispatch(&mut self) {
        match self {
            Self::Dolphin(_) => unsafe { ntx_engine_dispatch() },
            Self::N64(r) => r.dispatch(),
        }
    }
    fn surface(&mut self, window: &NativeWindow) -> bool {
        match self {
            Self::Dolphin(_) => unsafe { ntx_engine_surface(window.ptr().as_ptr().cast()) != 0 },
            Self::N64(r) => r.surface(Some(window)),
        }
    }
    fn detach_surface(&mut self) {
        if let Self::N64(r) = self {
            r.surface(None);
        }
    }
    fn resize(&mut self) {
        if let Self::Dolphin(_) = self {
            unsafe {
                ntx_engine_resize();
            }
        }
    }
    fn commit_for_departure(&mut self) -> bool {
        match self {
            Self::Dolphin(r) => r.commit_for_departure(),
            Self::N64(r) => r.commit().is_ok(),
        }
    }
    fn verify_departure(&self) -> std::result::Result<(), String> {
        match self {
            Self::Dolphin(r) => r.verify_departure(),
            Self::N64(r) => r.verify_departure(),
        }
    }
    fn verify_pending(&self) -> Result<()> {
        match self {
            Self::Dolphin(r) => r
                .pending_restore
                .as_ref()
                .ok_or("No pending scene")?
                .verify_current_saves(&r._profile.path)
                .map_err(Into::into),
            Self::N64(r) => r.verify_pending().map_err(Into::into),
        }
    }
    fn restore(&mut self) -> Result<Option<()>> {
        match self {
            Self::Dolphin(r) => r.restore(),
            Self::N64(r) => r.restore().map_err(Into::into),
        }
    }
    fn accept_checkpoint(&mut self, id: Option<String>) {
        match self {
            Self::Dolphin(r) => r.accepted_checkpoint = id,
            Self::N64(_) => {}
        }
    }
}
impl StopRuntime for LiveRuntime {
    fn epoch(&self) -> u64 {
        self.events.epoch
    }
    fn pause_commit(&mut self) -> std::result::Result<(), String> {
        match self {
            Self::Dolphin(r) => r.pause_commit(),
            Self::N64(r) => r.commit(),
        }
    }
    fn discard_checkpoint(&mut self) {
        if let Self::Dolphin(r) = self {
            r.discard_checkpoint();
        }
    }
    fn join(&mut self) -> std::result::Result<(), String> {
        match self {
            Self::Dolphin(r) => r.join(),
            Self::N64(r) => r.join(),
        }
    }
}
fn save_checkpoint(
    active: &mut LiveRuntime,
    root: &Path,
    collection: &mut Collection,
    index: usize,
) -> Result<()> {
    match active {
        LiveRuntime::Dolphin(r) => save_dolphin_checkpoint(r, root, collection, index),
        LiveRuntime::N64(r) => {
            let id = r.capture()?;
            let earlier = collection.entries[index].checkpoint.replace(id);
            if let Err(e) = save_collection(root, collection) {
                collection.entries[index].checkpoint = earlier;
                return Err(e);
            }
            r.accept_departure();
            Ok(())
        }
    }
}
fn pause_for_background(
    root: &Path,
    runtime: &mut Option<LiveRuntime>,
    collection: &mut Collection,
    selected: Option<usize>,
    native_paused: &mut Option<bool>,
) {
    if let Ok(mut ui) = UI.lock() {
        ui.cancel();
    }
    let Some(active) = runtime.as_mut() else {
        return;
    };
    active.pause(true);
    if !matches!(active.state(), 1 | 2) {
        *native_paused = None;
        return;
    }
    if *native_paused != Some(true)
        && !active.pending()
        && let Some(index) = selected
        && !collection.entries[index].restart_required
        && let Err(error) = save_checkpoint(active, root, collection, index)
    {
        platform::push(Command::Error(format!(
            "Could not checkpoint the game: {error}. Current saves and paused runtime retained."
        )));
    }
    *native_paused = Some(true);
}
fn boot(
    root: &Path,
    sys: &Path,
    entry: &SavedSession,
    window: &NativeWindow,
) -> Result<LiveRuntime> {
    match (entry.system, entry.backend) {
        (ntx::backend::System::GameCube, ntx::backend::Backend::Dolphin) => Ok(
            LiveRuntime::Dolphin(Box::new(boot_dolphin(root, sys, entry, window)?)),
        ),
        (ntx::backend::System::Nintendo64, ntx::backend::Backend::Mupen64PlusNext) => Ok(
            LiveRuntime::N64(Box::new(n64_runtime::Runtime::boot(root, entry, window)?)),
        ),
        _ => Err("Unsupported system/backend route".into()),
    }
}
struct DolphinRuntime {
    native_alive: bool,
    binding: Binding,
    info: RuntimeInfo,
    _cards: ntx::storage::CardStore,
    _profile: SaveProfile,
    _boot_window: NativeWindow,
    pending_restore: Option<Checkpoint>,
    accepted_checkpoint: Option<String>,
    accepted_generation: Option<ntx::storage::SaveGeneration>,
    failed_restore: bool,
}
struct RuntimeEvents {
    game_id: String,
    epoch: u64,
    stopped: AtomicBool,
    teardown_error: Mutex<Option<String>>,
}
impl DolphinRuntime {
    fn commit_for_departure(&self) -> bool {
        if let Ok(mut touch) = UI.lock() {
            touch.cancel();
        }
        // SAFETY: this live owner retains the native runtime and profile lock.
        // Keep execution paused after the barrier until stop/join completes.
        unsafe {
            platform::trace_current("native.pause.begin", "pause");
            ntx_engine_pause(1);
            platform::trace_current("native.pause.end", "paused");
            ntx_engine_input(Pad::default().values().as_ptr());
            // A partially loaded machine is quarantined through teardown.
            ntx_engine_recovery_required() != 0 || ntx_engine_flush_cards() != 0
        }
    }
}
/// Publish the immutable complete generation before changing its durable session
/// reference. The profile lock is still owned, including across metadata sync.
fn save_dolphin_checkpoint(
    active: &mut DolphinRuntime,
    root: &Path,
    collection: &mut Collection,
    index: usize,
) -> Result<()> {
    if active.pending_restore.is_some() {
        return Err("Checkpoint restore is still starting.".into());
    }
    if !active.native_alive {
        return Err("The native runtime is already unloaded.".into());
    }
    let timing = UI.lock().ok().map(|mut touch| {
        touch.cancel();
        touch.delivery()
    });
    if let Some(timing) = timing {
        platform::diagnostic(&format!(
            "Input delivery: {} updates, {} native sampled batches, {} canceled batches, max dispatch-to-pad {}us; <=1/2/4/8/16/33/>33ms {:?}. Software path only.",
            timing.updates,
            timing.sampled_batches,
            timing.canceled_batches,
            timing.max_dispatch_to_poll_micros,
            timing.histogram
        ));
        platform::diagnostic(&format!(
            "Input source age: {} foreground motion events, max event-to-dispatch {}us; <=1/2/4/8/16/33/>33ms {:?}; {} invalid timestamps. Software timestamps, not touch-to-photon.",
            timing.source_events,
            timing.max_event_to_dispatch_micros,
            timing.event_histogram,
            timing.invalid_source_times
        ));
        platform::diagnostic(&format!(
            "Input native poll cadence: {} complete pad polls; largest active gap {}us. Deliberate pause/departure gaps excluded; not game response or display FPS.",
            timing.native_polls, timing.max_poll_gap_micros
        ));
    }
    unsafe {
        platform::trace_current("native.pause.begin", "pause");
        ntx_engine_pause(1);
        platform::trace_current("native.pause.end", "paused");
        ntx_engine_input(Pad::default().values().as_ptr());
    }
    platform::trace_current("checkpoint.capture.begin", "");
    let captured = unsafe {
        checkpoint::capture(
            &active._profile.path,
            &active.binding,
            collection.entries[index].paused,
        )
    };
    platform::trace_current(
        "checkpoint.capture.end",
        if captured.is_ok() { "ok" } else { "error" },
    );
    let id = captured?;
    let generation = ntx::storage::save_generation(&active._profile.path)?;
    let earlier = collection.entries[index].checkpoint.replace(id.clone());
    platform::trace_current("checkpoint.publish.begin", "");
    if let Err(error) = save_collection(root, collection) {
        platform::trace_current("checkpoint.publish.end", "error");
        // Even if rename preceded the sync error, both possible references still
        // reach complete files. Never retire either on unacknowledged metadata.
        collection.entries[index].checkpoint = earlier;
        return Err(error);
    }
    platform::trace_current("checkpoint.publish.end", "durable");
    active.accepted_generation = Some(generation);
    if let Some(old) = active.accepted_checkpoint.replace(id.clone())
        && let Err(error) = checkpoint::retire_previous(&active._profile.path, &old, &id)
    {
        platform::diagnostic(&format!("Prior successful checkpoint retained: {error}"));
    }
    platform::diagnostic("Checkpoint complete: native paused, saves and session reference durable");
    Ok(())
}
impl DolphinRuntime {
    fn verify_departure(&self) -> std::result::Result<(), String> {
        self.accepted_generation
            .as_ref()
            .ok_or_else(|| "No acknowledged checkpoint generation.".to_owned())?
            .verify(&self._profile.path)
            .map_err(|e| e.to_string())
    }
    fn restore(&mut self) -> Result<Option<()>> {
        if unsafe { ntx_engine_state() } != 1 {
            return Ok(None);
        }
        let Some(mut state) = self.pending_restore.take() else {
            return Ok(None);
        };
        platform::trace(self.events.epoch, 0, "restore.validate.begin", "");
        state.verify_current_saves(&self._profile.path)?;
        platform::trace(self.events.epoch, 0, "restore.load.begin", "");
        if unsafe { ntx_engine_checkpoint_load(state.payload.as_mut_ptr(), state.payload.len()) }
            == 0
        {
            platform::trace(self.events.epoch, 0, "restore.load.end", "error");
            self.failed_restore = true;
            return Err("Checkpoint restore failed. Its file and current saves are retained; restart explicitly with current saves.".into());
        }
        platform::trace(self.events.epoch, 0, "restore.load.end", "ok_paused");
        self.accepted_generation = Some(ntx::storage::save_generation(&self._profile.path)?);
        platform::diagnostic(
            "Checkpoint restored into paused native runtime; current saves validated",
        );
        Ok(Some(()))
    }
}
impl StopRuntime for DolphinRuntime {
    fn epoch(&self) -> u64 {
        self.events.epoch
    }
    fn pause_commit(&mut self) -> std::result::Result<(), String> {
        if self.commit_for_departure() {
            Ok(())
        } else {
            Err("Could not durably commit the card. The paused game is retained; Stop game can be retried.".into())
        }
    }
    fn discard_checkpoint(&mut self) {
        if let Some(id) = &self.accepted_checkpoint
            && let Err(error) = checkpoint::discard_successful(&self._profile.path, id)
        {
            platform::diagnostic(&format!(
                "Stopped checkpoint retained after cleanup error: {error}"
            ));
        }
    }
    fn join(&mut self) -> std::result::Result<(), String> {
        if self.native_alive {
            if let Ok(mut error) = self.events.teardown_error.lock() {
                *error = None;
            }
            platform::trace(self.events.epoch, 0, "stop.join.begin", "writer_owned");
            platform::diagnostic("Native shutdown begin; profile writer still owned");
            // SAFETY: same serialized native owner. Keep all Rust fields/profile
            // alive until the stop caller has durably published its result.
            unsafe {
                self.failed_restore |= ntx_engine_recovery_required() != 0;
                ntx_engine_stop();
                ntx_engine_shutdown();
            }
            self.native_alive = false;
            platform::trace(self.events.epoch, 0, "stop.join.end", "writer_owned");
            platform::diagnostic("Native shutdown joined; profile writer still owned");
            // SAFETY: shutdown has joined native workers; this only reads core state.
            if unsafe { ntx_engine_state() } != 0 {
                return Err("Native shutdown did not reach the unloaded state.".into());
            }
        }
        if self.failed_restore {
            return Err(format!(
                "Failed saved-data initialization or checkpoint restore discarded without persistence; current saves retained. {}",
                ntx::storage::last_card_error().unwrap_or_default()
            ));
        }
        if let Some(error) = ntx::storage::last_card_error().or_else(|| {
            self.events
                .teardown_error
                .lock()
                .ok()
                .and_then(|e| e.clone())
        }) {
            return Err(error);
        }
        Ok(())
    }
}
impl Drop for DolphinRuntime {
    fn drop(&mut self) {
        if let Ok(mut touch) = UI.lock() {
            touch.cancel();
        }
        // SAFETY: the one android_main thread owns this runtime; callbacks are
        // static and the profile lock outlives completed native teardown.
        unsafe {
            if self.native_alive {
                ntx_engine_input(Pad::default().values().as_ptr());
            }
        }
        let _ = self.join();
    }
}
unsafe extern "C" fn event(context: *mut std::ffi::c_void, id: u32, message: *const c_char) {
    // SAFETY: this box is retained in DolphinRuntime until every native worker joins.
    let context = unsafe { &*context.cast::<RuntimeEvents>() };
    if id == 10 {
        context.stopped.store(true, Ordering::Relaxed);
    }
    if id == 2 || id == 20 {
        // SAFETY: native lends a NUL-terminated message during this callback.
        let message = unsafe { CStr::from_ptr(message) }
            .to_string_lossy()
            .into_owned();
        let message = if id == 20 || unsafe { ntx_engine_recovery_required() } != 0 {
            format!(
                "{message} {}",
                ntx::storage::last_card_error().unwrap_or_default()
            )
        } else {
            message
        };
        if id == 2
            && let Ok(mut error) = context.teardown_error.lock()
        {
            *error = Some(message.clone());
        }
        platform::push(Command::NativeError {
            game_id: context.game_id.clone(),
            epoch: context.epoch,
            message,
        });
    }
    if id == 3 {
        // SAFETY: same borrowed native message contract as critical alerts.
        let message = unsafe { CStr::from_ptr(message) }
            .to_string_lossy()
            .into_owned();
        platform::push(Command::Notice(message));
    }
}
extern "C" fn draw(phase: u32, width: f32, height: f32, aspect: f32) -> Rect {
    let observed = {
        let Ok(mut ui) = UI.lock() else {
            return Rect::default();
        };
        let fresh = !ui.has_presented_geometry();
        ui.present(width, height).cloned().map(|geometry| {
            let pad = if phase == 1 { ui.pad() } else { Pad::default() };
            let centers = [
                ui.stick_center(ntx::Control::StickX),
                ui.stick_center(ntx::Control::CX),
            ];
            (geometry, pad, centers, fresh, ui.controls_opacity())
        })
    };
    let Some((geometry, pad, centers, fresh, opacity)) = observed else {
        if !MISSING_PRESENT_REPORTED.swap(true, Ordering::Relaxed) {
            platform::trace_presentation(&format!("present.unavailable buffer={width}x{height}"));
        }
        return Rect::default();
    };
    MISSING_PRESENT_REPORTED.store(false, Ordering::Relaxed);
    if fresh {
        platform::trace_presentation(&format!(
            "present.ready buffer={width}x{height} pause_screen={:?}",
            geometry.pause_screen_area()
        ));
    }
    if phase == 0 {
        let a = geometry.picture(aspect);
        return Rect {
            left: a.left as i32,
            top: a.top as i32,
            right: a.right as i32,
            bottom: a.bottom as i32,
        };
    }
    let layout = &geometry.layout;
    if phase == 1 {
        let value = |control: ntx::Control| pad.values()[control as usize] as f32;
        // SAFETY: called on the video thread with the donor ImGui frame active.
        unsafe {
            let circle =
                |x, y, r, color| ntx_ui_circle(x, y, r, ntx::presentation::opacity(color, opacity));
            let rectangle = |l, t, r, b, color| {
                ntx_ui_rect(l, t, r, b, ntx::presentation::opacity(color, opacity))
            };
            let text = |x, y, font, label, color| {
                ntx_ui_text(
                    x,
                    y,
                    font,
                    label,
                    ntx::presentation::opacity(color, opacity),
                )
            };
            for zone in &layout.zones {
                let a = zone.area;
                let (x, y) = a.center();
                let radius = (a.right - a.left).min(a.bottom - a.top) * 0.5;
                match zone.target {
                    Target::Stick(cx, cy) => {
                        let (x, y, radius) = ntx::presentation::stick_visual(
                            a,
                            centers[usize::from(cx == ntx::Control::CX)],
                        );
                        circle(x, y, radius, 0x708f9298);
                        circle(x, y, radius - 1.5 * geometry.density, 0xc026292e);
                        let reach = radius * 0.64;
                        let knob_x = x + value(cx) * reach;
                        let knob_y = y - value(cy) * reach;
                        circle(
                            knob_x,
                            knob_y,
                            radius * 0.36,
                            if cx == ntx::Control::CX {
                                0xff51cfe6
                            } else {
                                0xffb5b3ae
                            },
                        );
                        if cx == ntx::Control::CX {
                            let font = radius * 0.4;
                            text(
                                knob_x - font * 0.3,
                                knob_y - font * 0.5,
                                font,
                                c"C".as_ptr(),
                                0xff252629,
                            );
                        }
                        continue;
                    }
                    Target::Button(control) => {
                        let color = zone.target.color();
                        if matches!(control, ntx::Control::X | ntx::Control::Y) {
                            // Original GameCube auxiliary-cap silhouettes,
                            // inside the same generous rectangular hit target.
                            let (w, h) = if control == ntx::Control::X {
                                ((a.right - a.left) * 0.72, a.bottom - a.top)
                            } else {
                                (a.right - a.left, (a.bottom - a.top) * 0.72)
                            };
                            let pill = |w: f32, h: f32, color| {
                                let r = w.min(h) * 0.5;
                                if w > h {
                                    rectangle(
                                        x - w / 2.0 + r,
                                        y - h / 2.0,
                                        x + w / 2.0 - r,
                                        y + h / 2.0,
                                        color,
                                    );
                                    circle(x - w / 2.0 + r, y, r, color);
                                    circle(x + w / 2.0 - r, y, r, color);
                                } else {
                                    rectangle(
                                        x - w / 2.0,
                                        y - h / 2.0 + r,
                                        x + w / 2.0,
                                        y + h / 2.0 - r,
                                        color,
                                    );
                                    circle(x, y - h / 2.0 + r, r, color);
                                    circle(x, y + h / 2.0 - r, r, color);
                                }
                            };
                            pill(
                                w,
                                h,
                                if value(control) > 0.0 {
                                    0xffeeeeee
                                } else {
                                    0xa0d1d4db
                                },
                            );
                            pill(
                                w - 2.4 * geometry.density,
                                h - 2.4 * geometry.density,
                                color,
                            );
                        } else if (a.right - a.left - (a.bottom - a.top)).abs() < 1.0 {
                            circle(
                                x,
                                y,
                                radius,
                                if value(control) > 0.0 {
                                    0xffeeeeee
                                } else {
                                    0xa0d1d4db
                                },
                            );
                            circle(x, y, radius - 1.2 * geometry.density, color);
                        } else {
                            rectangle(a.left, a.top, a.right, a.bottom, color);
                        }
                    }
                    Target::Travel(control) => {
                        rectangle(a.left, a.top, a.right, a.bottom, 0xe0475057);
                        rectangle(
                            a.left,
                            a.bottom - (a.bottom - a.top) * 0.25,
                            a.left + (a.right - a.left) * value(control),
                            a.bottom,
                            0xffb5b3ae,
                        );
                    }
                    Target::Dpad => {
                        let step = (a.right - a.left) / 3.0;
                        rectangle(
                            a.left + step,
                            a.top + step,
                            a.right - step,
                            a.bottom - step,
                            0xff848b95,
                        );
                        for (control, left, top) in [
                            (ntx::Control::Up, a.left + step, a.top),
                            (ntx::Control::Down, a.left + step, a.bottom - step),
                            (ntx::Control::Left, a.left, a.top + step),
                            (ntx::Control::Right, a.right - step, a.top + step),
                        ] {
                            rectangle(
                                left,
                                top,
                                left + step,
                                top + step,
                                if value(control) > 0.0 {
                                    0xffeeeeee
                                } else {
                                    0xffaaa6a2
                                },
                            );
                        }
                        continue;
                    }
                    Target::Pause => rectangle(a.left, a.top, a.right, a.bottom, 0xb03c3b3a),
                }
                let count = zone.label.to_bytes().len() as f32;
                let density = geometry.density;
                let font = ((a.bottom - a.top) * 0.4)
                    .min((a.right - a.left) / (count * 0.65 + 1.0))
                    .clamp(10.0 * density, 32.0 * density);
                text(
                    x - count * font * 0.3,
                    y - font * 0.5,
                    font,
                    zone.label.as_ptr(),
                    if matches!(zone.target, Target::Button(_) | Target::Dpad) {
                        0xff252629
                    } else {
                        0xffeeeeee
                    },
                );
            }
        }
    }
    Rect::default()
}

fn id() -> Result<String> {
    let mut bytes = [0_u8; 16];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn load_collection(root: &Path) -> Result<Collection> {
    let path = root.join("collection.json");
    if !path.exists() {
        return Ok(Collection::default());
    }
    if path.metadata()?.len() > 4 * 1024 * 1024 {
        return Err("collection metadata is too large; original retained".into());
    }
    let collection = Collection::decode(&std::fs::read(path)?)?;
    collection.validate()?;
    Ok(collection)
}
fn save_collection(root: &Path, collection: &Collection) -> Result<()> {
    ntx::storage::atomic_replace(
        &root.join("collection.json"),
        &serde_json::to_vec(collection)?,
    )?;
    Ok(())
}
fn resources(app: &AndroidApp, root: &Path) -> Result<PathBuf> {
    let manager = app.asset_manager();
    let mut index = String::new();
    let mut asset = manager
        .open(c"resource-index.txt")
        .ok_or("missing resource inventory")?;
    if asset.length() > 4 * 1024 * 1024 {
        return Err("resource inventory exceeds its size limit".into());
    }
    asset.read_to_string(&mut index)?;
    let sys = root.join("Sys");
    for relative in index.lines() {
        if Path::new(relative)
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err("invalid resource inventory path".into());
        }
        let name = CString::new(format!("Sys/{relative}"))?;
        let mut bytes = Vec::new();
        let mut asset = manager.open(&name).ok_or("missing engine resource")?;
        if asset.length() > 4 * 1024 * 1024 {
            return Err("engine resource exceeds its size limit".into());
        }
        asset.read_to_end(&mut bytes)?;
        let destination = sys.join(relative);
        std::fs::create_dir_all(destination.parent().ok_or("resource parent unavailable")?)?;
        if std::fs::read(&destination).ok().as_deref() != Some(bytes.as_slice()) {
            ntx::storage::atomic_replace(&destination, &bytes)?;
        }
    }
    Ok(sys)
}
fn media_digest(system: ntx::backend::System, file: File) -> Result<String> {
    match system {
        ntx::backend::System::GameCube => Ok(checkpoint::media_digest(file)?),
        ntx::backend::System::Nintendo64 => Ok(ntx::n64::Cartridge::read(file)?.sha256),
    }
}
fn inspect_media(
    uri: &str,
    live: Option<&LiveRuntime>,
) -> Result<(ntx::backend::System, ntx::backend::Backend)> {
    use ntx::backend::{Backend, System};
    let mut file = platform::open_document(uri)?;
    let mut magic = [0; 4];
    file.read_exact(&mut magic)?;
    if matches!(
        magic,
        [0x80, 0x37, 0x12, 0x40] | [0x37, 0x80, 0x40, 0x12] | [0x40, 0x12, 0x37, 0x80]
    ) {
        ntx::n64::Cartridge::read(platform::open_document(uri)?)?;
        return Ok((System::Nintendo64, Backend::Mupen64PlusNext));
    }
    let candidate = CString::new(uri)?;
    let mut info = MediaInfo::default();
    // Inspection also runs before the first Dolphin boot or after its shutdown.
    // Bind scoped host reads without creating an execution/profile owner.
    if !matches!(live,Some(LiveRuntime::Dolphin(d)) if d.native_alive) {
        unsafe {
            ntx_engine_platform(platform::dispatch);
        }
    }
    if unsafe { ntx_engine_inspect(candidate.as_ptr(), &mut info) } != 0 {
        return Ok((System::GameCube, Backend::Dolphin));
    }
    Err("Choose a readable GameCube disc or N64 cartridge. Existing games and progress are retained.".into())
}
fn boot_dolphin(
    root: &Path,
    sys: &Path,
    entry: &SavedSession,
    window: &NativeWindow,
) -> Result<DolphinRuntime> {
    if entry.system != ntx::backend::System::GameCube
        || entry.backend != ntx::backend::Backend::Dolphin
    {
        return Err("This backend is not yet available in this build.".into());
    }
    if entry.media_relink_required {
        return Err("Relink this game to grant this installation media access. Its identity and progress are retained.".into());
    }
    let build_engine =
        BUILD_ENGINE_SHA256.ok_or("this build lacks its native engine fingerprint")?;
    let build_resources = BUILD_RESOURCE_ID.ok_or("this build lacks its resource fingerprint")?;
    if !valid_id(&entry.profile_id.0) {
        return Err("invalid profile identity".into());
    }
    platform::trace_current("boot.media_hash.begin", "");
    let media_sha256 = checkpoint::media_digest(platform::open_document(&entry.uri)?)?;
    if entry
        .media_sha256
        .as_ref()
        .is_some_and(|expected| *expected != media_sha256)
    {
        return Err("The document bytes changed. Relink verified original media; existing game identity and saves are retained.".into());
    }
    platform::trace_current("boot.media_hash.end", "ok");
    let path = root.join("profiles").join(&entry.profile_id.0);
    std::fs::create_dir_all(&path)?;
    let profile = SaveProfile {
        _writer: ProfileLock::acquire(&path)?,
        path,
    };
    let user = CString::new(profile.path.to_str().ok_or("invalid user path")?)?;
    let sys = CString::new(sys.to_str().ok_or("invalid resource path")?)?;
    let uri = CString::new(entry.uri.as_str())?;
    let mut events = Box::new(RuntimeEvents {
        game_id: entry.game_id.0.clone(),
        epoch: NEXT_RUNTIME.fetch_add(1, Ordering::Relaxed),
        stopped: AtomicBool::new(false),
        teardown_error: Mutex::new(None),
    });
    platform::trace_runtime(events.epoch);
    platform::trace_current("runtime.boot.begin", "");
    // SAFETY: serialized owner, valid strings and stable process-long callbacks.
    unsafe {
        if ntx_engine_abi() != 8 {
            return Err("engine ABI mismatch".into());
        }
        let cards = ntx::storage::CardStore::bind(&profile.path)?;
        ntx_engine_card_writer(ntx::storage::write_card);
        ntx_engine_saved_reader(ntx::storage::read_image);
        ntx_engine_platform(platform::dispatch);
        ntx_engine_ui(draw);
        if let Ok(mut ui) = UI.lock() {
            ui.begin_runtime();
        }
        ntx_engine_input_sampler(Some(sample_input));
        platform::trace_current("boot.engine_init.begin", "");
        if ntx_engine_init(
            user.as_ptr(),
            sys.as_ptr(),
            event,
            std::ptr::from_mut(events.as_mut()).cast(),
            c"Vulkan".as_ptr(),
            c"Cubeb".as_ptr(),
        ) == 0
        {
            return Err("engine initialization rejected".into());
        }
        platform::trace_current("boot.engine_init.end", "ok");
        let mut runtime = DolphinRuntime {
            native_alive: true,
            _cards: cards,
            _profile: profile,
            _boot_window: window.clone(),
            info: RuntimeInfo {
                events,
                media_sha256: media_sha256.clone(),
            },
            binding: Binding {
                game_id: entry.game_id.0.clone(),
                profile_id: entry.profile_id.0.clone(),
                engine: build_engine.into(),
                state_abi: ntx_engine_checkpoint_abi(),
                resources: build_resources.into(),
                media_sha256: String::new(),
                disc: 0,
            },
            pending_restore: None,
            accepted_checkpoint: None,
            accepted_generation: None,
            failed_restore: false,
        };
        let mut info = MediaInfo::default();
        if ntx_engine_inspect(uri.as_ptr(), &mut info) == 0 {
            return Err("Choose a supported GameCube disc image (ISO/GCM or RVZ).".into());
        }
        runtime.binding.disc = info.disc;
        runtime.binding.media_sha256 = media_sha256;
        if let Some(id) = &entry.checkpoint {
            // No guest has started. Reject mismatches before paused native boot.
            platform::trace_current("boot.checkpoint_validate.begin", "");
            runtime.pending_restore = Some(checkpoint::load(
                &runtime._profile.path,
                id,
                &runtime.binding,
            )?);
            platform::trace_current("boot.checkpoint_validate.end", "ok");
        }
        // Initialize without guest execution. The owner loop resumes only a
        // healthy ready runtime after checked image reads/optional state load.
        platform::trace_current("boot.native_paused.begin", "");
        let admitted = ntx_engine_boot_paused(uri.as_ptr(), window.ptr().as_ptr().cast());
        if admitted == 0 {
            return Err("The native engine could not boot this GameCube game.".into());
        }
        platform::trace_current("boot.native_paused.end", "ok");
        platform::trace_current("runtime.boot.end", "ready_for_restore_or_play");
        Ok(runtime)
    }
}
pub fn run(app: AndroidApp) {
    if OWNER
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let result = run_owned(&app);
    if let Err(error) = result {
        platform::diagnostic(&format!("Player startup/host failure: {error}"));
        platform::notice(&app, format!("ntx: {error}"));
    }
    platform::shutdown();
    OWNER.store(false, Ordering::Release);
}
fn run_owned(app: &AndroidApp) -> Result<()> {
    platform::init(app)?;
    let root = app
        .internal_data_path()
        .ok_or("private storage unavailable")?
        .join("ntx");
    // A development migration transfers only a protected export into this
    // installation's private incoming directory. Rust validates and atomically
    // publishes it before any resource/runtime initialization. Existing target
    // data always wins; leaving the incoming export in place is intentional.
    let private_parent = root.parent().ok_or("missing private parent")?;
    let incoming = private_parent.join("ntx-import");
    if !root.try_exists()? && incoming.try_exists()? {
        ntx::migration::import_export(&incoming, private_parent)?;
    }
    let _store_writer = ProfileLock::acquire_file(
        &root
            .parent()
            .ok_or("missing private parent")?
            .join("ntx-store.lock"),
    )?;
    std::fs::create_dir_all(&root)?;
    match ntx::controls::Settings::load(&root) {
        Ok(settings) => {
            if let Ok(mut view) = crate::n64_host::view().lock() {
                view.configure_dead_zone(settings.dead_zone);
            }
            if let Ok(mut ui) = UI.lock() {
                ui.configure_controls(settings);
            }
        }
        Err(_) => {
            if let Ok(mut view) = crate::n64_host::view().lock() {
                view.configure_dead_zone(ntx::controls::Settings::default().dead_zone);
            }
            if let Ok(mut ui) = UI.lock() {
                ui.configure_controls(ntx::controls::Settings::default());
            }
            platform::push(Command::Notice("Could not load touch controls. Defaults are in use; the original settings file is retained.".into()));
        }
    }
    let sys = resources(app, &root)?;
    platform::diagnostic("Resources ready; loading collection");
    let mut collection = load_collection(&root)?;
    let mut interrupted = false;
    for entry in &mut collection.entries {
        interrupted |= entry.recover_interrupted();
    }
    if interrupted && save_collection(&root, &collection).is_err() {
        platform::diagnostic(
            "Could not persist interrupted-runtime recovery; the original intent marker remains retained.",
        );
    }
    let mut selected = collection
        .selected
        .as_ref()
        .and_then(|id| collection.entries.iter().position(|e| e.game_id.0 == *id));
    // A supplied link must resolve explicitly. Invalid/unknown cold links must
    // never fall through into the previously selected game's automatic boot.
    let launch = platform::initial_game_request();
    let explicit_launch = !matches!(launch, Ok(None));
    match launch {
        Ok(Some(id)) => platform::push(Command::Game(id)),
        Ok(None) => {}
        Err(_) => platform::notice(
            app,
            "Invalid ntx game link. Open a game from the library.".into(),
        ),
    }
    let mut runtime: Option<LiveRuntime> = None;
    let mut window: Option<NativeWindow> = None;
    let mut foreground = false;
    let mut destroyed = false;
    let mut hidden = false;
    let mut picker_pending = false;
    let mut media_confirmation: Option<MediaConfirmation> = None;
    let mut native_paused = None;
    let mut surface_pending = false;
    let mut surface_traced = false;
    let mut restart_requested = false;
    let mut refresh_pins = true;
    if explicit_launch {
        platform::menu(app, true, None);
    } else if let Some(index) = selected.filter(|i| collection.entries[*i].media_relink_required) {
        platform::relink_menu(app, &collection.entries[index].title);
    } else if let Some(index) = selected.filter(|i| collection.entries[*i].restart_required) {
        platform::menu(
            app,
            true,
            Some(format!(
                "ntx paused\n\n{}",
                collection.entries[index]
                    .last_error
                    .as_deref()
                    .unwrap_or("The previous runtime needs an explicit restart.")
            )),
        );
    } else if let Some(index) = selected.filter(|i| collection.entries[*i].stopped) {
        platform::unloaded_menu(app, &collection.entries[index].title, None);
    } else if selected.is_some() {
        platform::menu(app, false, None);
    } else {
        platform::menu(app, true, None);
    }
    while !destroyed {
        let poll_timeout = runtime
            .as_ref()
            .map_or(Duration::from_millis(8), LiveRuntime::poll_timeout);
        app.poll_events(Some(poll_timeout), |event| {
            if let PollEvent::Main(event) = event {
                match event {
                    MainEvent::InitWindow { .. } => {
                        platform::trace_current("surface.init", "");
                        surface_traced = false;
                        window = app.native_window();
                        measure_window(None);
                        platform::refresh_window(app);
                        surface_pending = runtime.is_some();
                    }
                    MainEvent::TerminateWindow { .. } => {
                        platform::trace_current("surface.terminate", "");
                        // Complete pause/checkpoint while the old drawable is still
                        // retained; surface loss never requests a fresh boot.
                        pause_for_background(
                            &root,
                            &mut runtime,
                            &mut collection,
                            selected,
                            &mut native_paused,
                        );
                        if let Some(active) = runtime.as_mut() {
                            active.detach_surface();
                        }
                        window = None;
                        measure_window(None);
                        if let Ok(mut touch) = UI.lock() {
                            touch.cancel();
                        }
                    }
                    MainEvent::GainedFocus => {
                        platform::trace_focus();
                        foreground = true;
                        platform::refresh_window(app);
                    }
                    MainEvent::LostFocus | MainEvent::Pause | MainEvent::Stop => {
                        platform::trace_current("activity.departure", "lost_focus_pause_or_stop");
                        foreground = false;
                        pause_for_background(
                            &root,
                            &mut runtime,
                            &mut collection,
                            selected,
                            &mut native_paused,
                        );
                        if let Ok(mut touch) = UI.lock() {
                            touch.cancel();
                        }
                    }
                    MainEvent::Destroy => destroyed = true,
                    MainEvent::ContentRectChanged { .. } => {
                        // Insets alone do not resize the native drawable. The
                        // shared geometry cancels only if its effective map changes.
                        platform::refresh_window(app);
                    }
                    MainEvent::WindowResized { .. } | MainEvent::ConfigChanged { .. } => {
                        measure_window(None);
                        platform::refresh_window(app);
                        if let Ok(mut touch) = UI.lock() {
                            touch.cancel();
                        }
                        if let Some(active) = runtime.as_mut() {
                            active.resize();
                        }
                    }
                    MainEvent::InputAvailable => {
                        if let Ok(mut input) = app.input_events_iter() {
                            while input.next(|event| {
                                if let InputEvent::MotionEvent(motion) = event {
                                    let down = matches!(
                                        motion.action(),
                                        MotionAction::Down | MotionAction::PointerDown
                                    );
                                    let mut pause_menu = None;
                                    let mut report = None;
                                    if down {
                                        platform::trace_input(&format!(
                                            "dispatch hidden={hidden} runtime={} window={}",
                                            runtime.is_some(),
                                            window.is_some()
                                        ));
                                    }
                                    if hidden
                                        && window.is_some()
                                        && matches!(runtime, Some(LiveRuntime::N64(_)))
                                    {
                                        let mut pause = false;
                                        if let Ok(mut touch) = crate::n64_host::view().lock() {
                                            match motion.action() {
                                                MotionAction::Down | MotionAction::PointerDown => {
                                                    let p = motion
                                                        .pointer_at_index(motion.pointer_index());
                                                    let hit =
                                                        touch.down(p.pointer_id(), p.x(), p.y());
                                                    pause =
                                                        hit == Some(ntx::n64_touch::Target::Pause);
                                                    if hit.is_some() && haptics_enabled() {
                                                        platform::haptic(app);
                                                    }
                                                }
                                                MotionAction::Move => {
                                                    for p in motion.pointers() {
                                                        touch.move_pointer(
                                                            p.pointer_id(),
                                                            p.x(),
                                                            p.y(),
                                                        );
                                                    }
                                                }
                                                MotionAction::Up | MotionAction::PointerUp => {
                                                    let p = motion
                                                        .pointer_at_index(motion.pointer_index());
                                                    touch.up(p.pointer_id());
                                                }
                                                MotionAction::Cancel => touch.cancel(),
                                                _ => {}
                                            }
                                        }
                                        if pause && let Some(index) = selected {
                                            collection.entries[index].paused = true;
                                            hidden = false;
                                            pause_for_background(
                                                &root,
                                                &mut runtime,
                                                &mut collection,
                                                selected,
                                                &mut native_paused,
                                            );
                                            if let Some(active) = runtime.as_ref() {
                                                platform::player_menu(
                                                    app,
                                                    active.events.epoch,
                                                    !collection.entries[index].restart_required,
                                                    None,
                                                );
                                            }
                                        }
                                        return InputStatus::Handled;
                                    }
                                    if hidden && runtime.is_some() && window.is_some() {
                                        if let Ok(mut touch) = UI.lock() {
                                            touch.record_source_age(platform::motion_age(
                                                motion.event_time(),
                                            ));
                                            match motion.action() {
                                                MotionAction::Down | MotionAction::PointerDown => {
                                                    let pointer = motion
                                                        .pointer_at_index(motion.pointer_index());
                                                    let hit = touch.down_report(
                                                        pointer.pointer_id(),
                                                        pointer.x(),
                                                        pointer.y(),
                                                    );
                                                    report = Some(hit);
                                                    if hit == DownReport::Pause
                                                        && let Some(index) = selected
                                                    {
                                                        collection.entries[index].paused = true;
                                                        touch.cancel();
                                                        hidden = false;
                                                        if let Some(active) = &runtime {
                                                            pause_menu = Some((
                                                                active.events.epoch,
                                                                !collection.entries[index]
                                                                    .restart_required,
                                                            ));
                                                        }
                                                    }
                                                }
                                                MotionAction::Move => {
                                                    for pointer in motion.pointers() {
                                                        touch.moved(
                                                            pointer.pointer_id(),
                                                            pointer.x(),
                                                            pointer.y(),
                                                        );
                                                    }
                                                }
                                                MotionAction::Up | MotionAction::PointerUp => {
                                                    let pointer = motion
                                                        .pointer_at_index(motion.pointer_index());
                                                    touch.up(pointer.pointer_id());
                                                }
                                                MotionAction::Cancel => touch.cancel(),
                                                _ => {}
                                            }
                                        } else if down {
                                            platform::trace_input_problem(
                                                "rejected ui_lock_poisoned",
                                            );
                                        }
                                    } else if down {
                                        platform::trace_input_problem(&format!(
                                            "rejected hidden={hidden} runtime={} window={}",
                                            runtime.is_some(),
                                            window.is_some()
                                        ));
                                    }
                                    // No UI lock crosses JNI menu posting or native quiescence.
                                    if let Some(hit) = report {
                                        if matches!(hit, DownReport::Captured | DownReport::Pause)
                                            && haptics_enabled()
                                        {
                                            platform::haptic(app);
                                        }
                                        let detail = format!("admitted result={hit:?}");
                                        if hit == DownReport::Captured {
                                            platform::trace_input(&detail);
                                        } else {
                                            platform::trace_input_problem(&detail);
                                        }
                                    }
                                    if let Some((epoch, can_resume)) = pause_menu {
                                        platform::trace(
                                            epoch,
                                            0,
                                            "pause.accepted",
                                            "logical_intent=true",
                                        );
                                        platform::player_menu(app, epoch, can_resume, None);
                                    }
                                    InputStatus::Handled
                                } else {
                                    InputStatus::Unhandled
                                }
                            }) {}
                        }
                    }
                    _ => {}
                }
            }
        });
        for command in platform::commands() {
            if !matches!(
                &command,
                Command::ConfirmLegacyMedia(_) | Command::Notice(_)
            ) {
                media_confirmation = None;
            }
            let command = match command {
                Command::NativeError {
                    game_id,
                    epoch,
                    message,
                } => {
                    if runtime
                        .as_ref()
                        .is_some_and(|active| active.events.epoch == epoch)
                    {
                        Command::Error(message)
                    } else {
                        if let Some(index) = collection
                            .entries
                            .iter()
                            .position(|entry| entry.game_id.0 == game_id)
                        {
                            collection.entries[index].last_error =
                                Some(message.chars().take(8192).collect());
                            if save_collection(&root, &collection).is_err() {
                                platform::notice(
                                    app,
                                    "Could not preserve an earlier runtime error on disk.".into(),
                                );
                            }
                            if runtime.is_none()
                                && selected == Some(index)
                                && collection.entries[index].restart_required
                            {
                                hidden = false;
                                platform::menu(app, true, Some(message));
                            } else {
                                platform::notice(app, format!("Earlier runtime: {message}"));
                            }
                        }
                        continue;
                    }
                }
                Command::ConfirmLegacyMedia(menu) => {
                    let Some(pending) = media_confirmation.take() else {
                        continue;
                    };
                    if !platform::menu_current(menu)
                        || runtime.is_some()
                        || selected
                            .is_none_or(|i| collection.entries[i].game_id.0 != pending.game_id)
                    {
                        continue;
                    }
                    let index = collection.resolve_game(&pending.game_id)?;
                    let observed = platform::open_document(&pending.uri)
                        .and_then(|file| media_digest(pending.system, file));
                    if !observed.is_ok_and(|digest| digest == pending.digest) {
                        platform::notice(app, "The chosen file changed or is unavailable. Choose the original again; your saves are retained.".into());
                        continue;
                    }
                    let earlier = collection.entries[index].clone();
                    if let Err(error) = collection.confirm_legacy_media(
                        &pending.game_id,
                        &pending.uri,
                        &pending.digest,
                        pending.system,
                        pending.backend,
                    ) {
                        platform::notice(app, error.into());
                        continue;
                    }
                    if let Err(error) = save_collection(&root, &collection) {
                        collection.entries[index] = earlier;
                        platform::notice(app, format!("Could not preserve media access: {error}"));
                        continue;
                    }
                    Command::Selected(pending.uri)
                }
                Command::DocumentSelected(uri) => {
                    picker_pending = false;
                    let relink = collection
                        .entries
                        .iter()
                        .position(|e| e.uri == uri && e.media_relink_required)
                        .or_else(|| {
                            selected.filter(|i| collection.entries[*i].media_relink_required)
                        });
                    if let Some(index) = relink {
                        let route = match inspect_media(&uri, runtime.as_ref()) {
                            Ok(route) => route,
                            Err(e) => {
                                platform::notice(app, e.to_string());
                                continue;
                            }
                        };
                        let observed = match platform::open_document(&uri)
                            .and_then(|file| media_digest(route.0, file))
                        {
                            Ok(digest) => digest,
                            Err(error) => {
                                platform::notice(
                                    app,
                                    format!("Could not read selected media: {error}"),
                                );
                                continue;
                            }
                        };
                        let earlier = collection.entries[index].clone();
                        if earlier.media_sha256.is_none() && earlier.uri != uri {
                            let mut proposed = collection.clone();
                            if runtime.is_some() {
                                platform::notice(
                                    app,
                                    "Stop the current game before confirming its original file."
                                        .into(),
                                );
                                continue;
                            }
                            if let Err(error) = proposed.confirm_legacy_media(
                                &earlier.game_id.0,
                                &uri,
                                &observed,
                                route.0,
                                route.1,
                            ) {
                                platform::notice(app, error.into());
                                continue;
                            }
                            let filename = platform::display_name(&uri)
                                .unwrap_or_else(|_| "the selected file".into());
                            media_confirmation = Some(MediaConfirmation {
                                game_id: earlier.game_id.0.clone(),
                                uri,
                                digest: observed,
                                system: route.0,
                                backend: route.1,
                            });
                            platform::confirm_legacy_media_menu(app, &earlier.title, &filename);
                            continue;
                        }
                        if let Err(error) = collection.relink_game_routed(
                            &earlier.game_id.0,
                            &uri,
                            &observed,
                            route.0,
                            route.1,
                        ) {
                            platform::notice(app, error.into());
                            continue;
                        }
                        if let Err(error) = save_collection(&root, &collection) {
                            collection.entries[index] = earlier;
                            platform::notice(
                                app,
                                format!("Could not preserve media access: {error}"),
                            );
                            continue;
                        }
                    }
                    Command::Selected(uri)
                }
                Command::Game(id) => {
                    if runtime.is_none()
                        && let Ok(index) = collection.resolve_game(&id)
                        && collection.entries[index].media_relink_required
                    {
                        selected = Some(index);
                        collection.selected = Some(id);
                        save_collection(&root, &collection)?;
                        platform::relink_menu(app, &collection.entries[index].title);
                        continue;
                    }
                    match collection
                        .open_game(&id, runtime.as_ref().map(|r| r.events.game_id.as_str()))
                    {
                        Ok(GameOpen::ResumeCurrent) => {
                            if let Some(index) = selected {
                                collection.entries[index].paused = false;
                            }
                            platform::menu(app, false, None);
                            continue;
                        }
                        Ok(GameOpen::Select(index)) => {
                            Command::Selected(collection.entries[index].uri.clone())
                        }
                        Err(error) => {
                            platform::notice(app, error.into());
                            continue;
                        }
                    }
                }
                other => other,
            };
            match command {
                Command::NativeError { .. }
                | Command::Game(_)
                | Command::DocumentSelected(_)
                | Command::ConfirmLegacyMedia(_) => {
                    continue;
                }
                Command::Controls(menu, width, height) => {
                    if !platform::menu_current(menu) {
                        continue;
                    }
                    hidden = false;
                    if let Some(index) = selected.filter(|_| runtime.is_some()) {
                        collection.entries[index].paused = true;
                    }
                    pause_for_background(
                        &root,
                        &mut runtime,
                        &mut collection,
                        selected,
                        &mut native_paused,
                    );
                    let settings = UI
                        .lock()
                        .map(|ui| ui.controls())
                        .map_err(|_| "controls lock poisoned")?;
                    if selected.is_some_and(|i| {
                        collection.entries[i].system == ntx::backend::System::Nintendo64
                    }) {
                        platform::n64_controls_menu(
                            app,
                            runtime.as_ref().map_or(0, |r| r.events.epoch),
                            &settings,
                        );
                        continue;
                    }
                    platform::controls_menu(
                        app,
                        runtime.as_ref().map_or(0, |r| r.events.epoch),
                        width,
                        height,
                        settings,
                    );
                }
                Command::ApplyControls(menu, settings) => {
                    if !platform::menu_current(menu) {
                        continue;
                    }
                    if settings.save(&root).is_ok() {
                        if let Ok(mut view) = crate::n64_host::view().lock() {
                            view.configure_dead_zone(settings.dead_zone);
                        }
                        if let Ok(mut ui) = UI.lock() {
                            ui.configure_controls(settings);
                        }
                        platform::push(Command::Library);
                    } else {
                        platform::notice(app,"Could not confirm saved controls. Your game is paused and your draft is kept so you can retry.".into());
                    }
                }
                Command::ControlsBack(menu) => {
                    if platform::menu_current(menu) {
                        platform::push(Command::Library);
                    }
                }
                Command::N64Comfort(menu, action) => {
                    if !platform::menu_current(menu)
                        || !selected.is_some_and(|i| {
                            collection.entries[i].system == ntx::backend::System::Nintendo64
                        })
                    {
                        continue;
                    }
                    let mut settings = UI.lock().map_err(|_| "controls lock poisoned")?.controls();
                    match action {
                        13 => settings.opacity = settings.opacity.saturating_sub(10).max(30),
                        14 => settings.opacity = (settings.opacity + 10).min(100),
                        15 => settings.haptics = !settings.haptics,
                        16 => {
                            settings.dead_zone = match settings.dead_zone {
                                0..=4 => 12,
                                5..=15 => 20,
                                _ => 0,
                            }
                        }
                        _ => continue,
                    }
                    if let Err(e) = settings.save(&root) {
                        platform::notice(app, format!("Could not preserve controls: {e}"));
                        continue;
                    }
                    if let Ok(mut view) = crate::n64_host::view().lock() {
                        view.configure_dead_zone(settings.dead_zone);
                    }
                    if let Ok(mut ui) = UI.lock() {
                        ui.configure_controls(settings.clone());
                    }
                    platform::n64_controls_menu(
                        app,
                        runtime.as_ref().map_or(0, |r| r.events.epoch),
                        &settings,
                    );
                }
                Command::About(menu) => {
                    if platform::menu_current(menu) {
                        platform::about_menu(app);
                    }
                }
                Command::Library => {
                    hidden = false;
                    if let Some(index) = selected.filter(|_| runtime.is_some()) {
                        collection.entries[index].paused = true;
                    }
                    pause_for_background(
                        &root,
                        &mut runtime,
                        &mut collection,
                        selected,
                        &mut native_paused,
                    );
                    platform::library_menu(
                        app,
                        collection
                            .entries
                            .iter()
                            .map(|e| {
                                let status = if e.restart_required {
                                    "Needs attention"
                                } else if e.checkpoint.is_some() && !e.stopped {
                                    "Resume saved scene"
                                } else {
                                    "Play · in-game saves kept"
                                };
                                (
                                    e.game_id.0.clone(),
                                    format!(
                                        "{}\n{status}",
                                        ntx::presentation::game_title(&e.title)
                                    ),
                                )
                            })
                            .collect(),
                        runtime.as_ref().map(|r| r.events.epoch),
                        selected.is_some_and(|i| !collection.entries[i].restart_required),
                    );
                }
                Command::PinCurrent => {
                    if let Some(index) = selected {
                        let entry = &collection.entries[index];
                        match platform::pin_game(&entry.game_id.0, &entry.title) {
                            Ok(true) => {
                                collection.entries[index].shortcut_requested = true;
                                let message = if save_collection(&root, &collection).is_ok() {
                                    "Home screen request sent. Confirm it in your launcher."
                                } else { "Home screen request sent, but its status could not be recorded." };
                                platform::notice(app, message.into());
                                refresh_pins = true;
                            }
                            Ok(false) => platform::notice(app, "This launcher cannot add a game shortcut right now.".into()),
                            Err(_) => platform::notice(app, "Could not request the game shortcut. Your game and saves are unchanged.".into()),
                        }
                    }
                }
                Command::Choose => {
                    if !picker_pending {
                        platform::picker(app, selected.map(|i| collection.entries[i].uri.as_str()));
                        picker_pending = true;
                    }
                }
                Command::Restart => restart_requested = true,
                Command::Resume(epoch) => {
                    platform::trace(epoch, 0, "resume.command", "");
                    if runtime.as_ref().is_some_and(|r| r.events.epoch == epoch)
                        && let Some(index) = selected
                        && !collection.entries[index].restart_required
                    {
                        collection.entries[index].paused = false;
                        platform::menu(app, false, None);
                    } else {
                        platform::trace(epoch, 0, "resume.reject", "stale_or_restart_required");
                    }
                }
                Command::Stop(epoch) => {
                    platform::trace(epoch, 0, "stop.command", "");
                    if let Some(index) = selected {
                        match stop_game(&mut runtime, epoch, &mut collection, index, |c| {
                            save_collection(&root, c).map_err(|e| e.to_string())
                        }) {
                            StopResult::Ignored => {
                                platform::trace(epoch, 0, "stop.reject", "stale_runtime");
                            }
                            StopResult::Retained(error) => {
                                platform::trace(
                                    epoch,
                                    0,
                                    "stop.retained",
                                    "barrier_or_storage_error",
                                );
                                hidden = false;
                                native_paused = Some(true);
                                platform::player_menu(app, epoch, false, Some(error));
                            }
                            StopResult::Unloaded { error } => {
                                platform::trace(
                                    epoch,
                                    0,
                                    "stop.complete",
                                    if error.is_none() {
                                        "joined_durable_profile_released"
                                    } else {
                                        "unloaded_with_error"
                                    },
                                );
                                platform::trace_runtime(0);
                                hidden = false;
                                native_paused = None;
                                surface_pending = false;
                                restart_requested = false;
                                platform::diagnostic(if error.is_none() {
                                    "Stop complete: native workers joined, clean metadata durable, profile released; game unloaded"
                                } else {
                                    "Stop unloaded the native runtime; recovery metadata/error retained"
                                });
                                platform::unloaded_menu(
                                    app,
                                    &collection.entries[index].title,
                                    error,
                                );
                            }
                        }
                    }
                }
                Command::Open => {
                    if runtime.is_none()
                        && let Some(index) =
                            selected.filter(|i| collection.entries[*i].media_relink_required)
                    {
                        platform::relink_menu(app, &collection.entries[index].title);
                        continue;
                    }
                    if runtime.is_none()
                        && let Some(index) = selected
                        && collection.entries[index].stopped
                        && !collection.entries[index].restart_required
                    {
                        collection.entries[index].request_restart();
                        if save_collection(&root, &collection).is_ok() {
                            platform::menu(app, false, None);
                        } else {
                            collection.entries[index].stopped = true;
                            platform::notice(
                                app,
                                "Could not preserve launch intent; the game remains unloaded."
                                    .into(),
                            );
                        }
                    }
                }
                Command::Selected(uri) => {
                    picker_pending = false;
                    let route = match inspect_media(&uri, runtime.as_ref()) {
                        Ok(route) => route,
                        Err(e) => {
                            platform::notice(app, e.to_string());
                            continue;
                        }
                    };
                    if runtime.is_some() {
                        if selected.is_some_and(|i| collection.entries[i].restart_required) {
                            platform::notice(app, "The current game needs recovery. Use Stop or Restart with current saves before opening another game.".into());
                            continue;
                        }
                        if selected.is_some_and(|i| {
                            collection.entries[i].uri == uri
                                && !collection.entries[i].restart_required
                        }) {
                            if let Some(index) = selected {
                                collection.entries[index].paused = false;
                            }
                            platform::menu(app, false, None);
                            continue;
                        }
                    }
                    // Save the departing scene and join while its profile remains
                    // exclusively owned. A failed checkpoint keeps its live RAM.
                    if let (Some(active), Some(index)) = (runtime.as_mut(), selected) {
                        let saved = save_checkpoint(active, &root, &mut collection, index);
                        if let Err(error) = saved {
                            native_paused = Some(true);
                            platform::push(Command::Error(format!(
                                "Could not leave the current game: {error}"
                            )));
                            continue;
                        }
                        let joined = active.join().and_then(|()| active.verify_departure());
                        collection.entries[index].runtime_incomplete = joined.is_err();
                        if let Err(error) = joined {
                            collection.entries[index].fail(error);
                        }
                        if let Err(error) = save_collection(&root, &collection) {
                            collection.entries[index].runtime_incomplete = true;
                            collection.entries[index]
                                .fail(format!("Could not preserve the departing session: {error}"));
                            let _ = save_collection(&root, &collection);
                            hidden = false;
                            platform::menu(app, true, collection.entries[index].last_error.clone());
                            drop(runtime.take());
                            native_paused = None;
                            continue;
                        }
                    }
                    drop(runtime.take());
                    native_paused = None;
                    let index =
                        if let Some(index) = collection.entries.iter().position(|e| e.uri == uri) {
                            index
                        } else {
                            let title = platform::display_name(&uri)
                                .unwrap_or_else(|_| "Nintendo game".into());
                            collection.entries.push(SavedSession {
                                system: route.0,
                                backend: route.1,
                                media_relink_required: false,
                                media_sha256: None,
                                game_id: GameId(id()?),
                                profile_id: SaveProfileId(id()?),
                                uri,
                                title,
                                paused: false,
                                last_error: None,
                                restart_required: false,
                                runtime_incomplete: false,
                                stopped: false,
                                checkpoint: None,
                                shortcut_requested: false,
                                shortcut_pinned: None,
                            });
                            collection.entries.len() - 1
                        };
                    collection.selected = Some(collection.entries[index].game_id.0.clone());
                    selected = Some(index);
                    if collection.entries[index].stopped
                        && !collection.entries[index].restart_required
                    {
                        collection.entries[index].request_restart();
                    }
                    if !collection.entries[index].restart_required {
                        collection.entries[index].paused = false;
                    }
                    if save_collection(&root, &collection).is_err() {
                        platform::notice(app,"Could not durably save the collection. Files are retained; choose the document again to retry.".into());
                        continue;
                    }
                    if collection.entries[index].restart_required {
                        platform::menu(app, true, collection.entries[index].last_error.clone());
                    } else {
                        platform::menu(app, false, None);
                    }
                }
                Command::MenuHidden(generation) => {
                    platform::trace(
                        0,
                        generation,
                        "menu.hidden.consume",
                        if platform::menu_current(generation) {
                            "current"
                        } else {
                            "stale"
                        },
                    );
                    if platform::menu_current(generation) {
                        hidden = true;
                    }
                }
                Command::Canceled => picker_pending = false,
                Command::Error(error) => {
                    picker_pending = false;
                    if runtime.is_some()
                        && let Some(index) = selected
                    {
                        collection.entries[index].fail(error.clone());
                        if let Ok(mut touch) = UI.lock() {
                            touch.cancel();
                        }
                        hidden = false;
                        {
                            platform::trace_current("native.pause.begin", "pause");
                            runtime.as_mut().unwrap().pause(true);
                            platform::trace_current("native.pause.end", "paused");
                        }
                        native_paused = Some(true);
                        let persistence = if save_collection(&root, &collection).is_err() {
                            "\nCould not preserve this error on disk; it remains on this screen."
                        } else {
                            ""
                        };
                        if let Some(active) = &runtime {
                            platform::player_menu(
                                app,
                                active.events.epoch,
                                false,
                                Some(format!("{error}{persistence}")),
                            );
                        }
                    } else {
                        platform::notice(app, error);
                    }
                }
                Command::Notice(message) => platform::notice(app, message),
            }
        }
        // Android documents this Binder query as potentially slow. Run it on
        // Rust's owner worker while the guest is paused/unloaded, never Java UI.
        if refresh_pins && foreground && (runtime.is_none() || native_paused == Some(true)) {
            refresh_pins = false;
            match platform::pinned_games() {
                Ok(ids) => {
                    if collection.observe_shortcuts(&ids)
                        && save_collection(&root, &collection).is_err()
                    {
                        platform::notice(
                            app,
                            "Could not record Home screen status; the game is retained.".into(),
                        );
                    }
                }
                Err(_) => platform::diagnostic(
                    "Own-app pinned shortcut observation unavailable; prior status retained",
                ),
            }
        }
        if restart_requested {
            restart_requested = false;
            if let Some(index) = selected {
                if collection.entries[index].media_relink_required {
                    hidden = false;
                    platform::relink_menu(app, &collection.entries[index].title);
                    continue;
                }
                if runtime
                    .as_mut()
                    .is_some_and(|active| !active.commit_for_departure())
                {
                    native_paused = Some(true);
                    platform::push(Command::Error("The memory card could not be committed. The current runtime is retained; restart has not occurred.".into()));
                    continue;
                }
                let mut teardown_error = None;
                if let Some(active) = runtime.as_mut() {
                    let quarantined = active.failed();
                    let joined = active.join();
                    if !quarantined {
                        teardown_error = joined.err();
                    }
                    // A failed-load quarantine has protected durable ordinary
                    // files through shutdown; only this explicit action cold boots.
                }
                if let Some(error) = teardown_error {
                    collection.entries[index].fail(format!(
                        "Persistence failed during restart teardown: {error}"
                    ));
                    hidden = false;
                    platform::menu(app, true, collection.entries[index].last_error.clone());
                    let _ = save_collection(&root, &collection);
                    drop(runtime.take());
                    continue;
                }
                // Preserve the lock until restart intent is durably recorded.
                let earlier = collection.entries[index].clone();
                collection.entries[index].request_restart();
                if let Err(error) = save_collection(&root, &collection) {
                    collection.entries[index] = earlier;
                    collection.entries[index]
                        .fail(format!("Could not preserve restart intent: {error}"));
                    let _ = save_collection(&root, &collection);
                    hidden = false;
                    platform::menu(app, true, collection.entries[index].last_error.clone());
                } else {
                    platform::menu(app, false, None);
                }
                drop(runtime.take());
                native_paused = None;
            }
        }
        if runtime.is_none()
            && hidden
            && foreground
            && let (Some(index), Some(window)) = (selected, &window)
            && collection.entries[index].boot_allowed()
            && window.width() > 32
            && window.height() > 32
        {
            collection.entries[index].runtime_incomplete = true;
            if save_collection(&root, &collection).is_err() {
                collection.entries[index]
                    .fail("Could not durably record the runtime start. No game was booted.".into());
                hidden = false;
                platform::menu(app, true, collection.entries[index].last_error.clone());
                continue;
            }
            match boot(&root, &sys, &collection.entries[index], window) {
                Ok(active) => {
                    collection.entries[index].media_sha256 = Some(active.media_sha256.clone());
                    runtime = Some(active);
                    if let Err(error) = save_collection(&root, &collection) {
                        collection.entries[index]
                            .fail(format!("Could not preserve media identity: {error}"));
                        hidden = false;
                        platform::menu(app, true, collection.entries[index].last_error.clone());
                    }
                    surface_pending = false;
                    native_paused = None;
                }
                Err(error) => {
                    collection.entries[index].fail(error.to_string());
                    let retained = save_collection(&root, &collection).is_ok();
                    hidden = false;
                    platform::menu(
                        app,
                        true,
                        Some(format!(
                            "Could not start this game.\n\n{error}{}",
                            if retained {
                                ""
                            } else {
                                "\nCould not preserve this error on disk."
                            }
                        )),
                    );
                }
            }
        }
        if runtime.is_some() {
            // SAFETY: all lifecycle work runs on this owner thread; surface strong
            // references are held by Rust and the native queue/backend owners.
            {
                runtime.as_mut().unwrap().dispatch();
                if surface_pending && let Some(window) = &window {
                    if !surface_traced {
                        platform::trace_current("surface.apply.begin", "");
                    }
                    surface_pending = !runtime.as_mut().unwrap().surface(window);
                    if !surface_traced || !surface_pending {
                        platform::trace_current(
                            "surface.apply.end",
                            if surface_pending {
                                "pending"
                            } else {
                                "applied"
                            },
                        );
                    }
                    surface_traced = true;
                }
                if foreground
                    && window.is_some()
                    && !surface_pending
                    && let (Some(active), Some(index)) = (runtime.as_mut(), selected)
                    && active.pending()
                    && !collection.entries[index].restart_required
                {
                    match active.restore() {
                        Ok(Some(())) => {
                            // SavedSession owns the latest durable UI intent;
                            // Resume after the capture may have changed it.
                            let paused = collection.entries[index].paused;
                            active.accept_checkpoint(collection.entries[index].checkpoint.clone());
                            native_paused = Some(true);
                            if paused {
                                hidden = false;
                                platform::player_menu(app, active.events.epoch, true, None);
                            }
                        }
                        Ok(None) => {}
                        Err(error) => {
                            collection.entries[index].fail(error.to_string());
                            platform::push(Command::Error(error.to_string()));
                            native_paused = Some(true);
                        }
                    }
                }
                if matches!(runtime.as_ref().unwrap().state(), 1 | 2) {
                    let restoring = runtime.as_ref().is_some_and(|r| r.pending());
                    let active = runtime.as_ref().unwrap();
                    let paused = ntx::scheduling::should_pause(
                        &active.events.game_id,
                        selected.map(|i| &collection.entries[i]),
                        ntx::scheduling::HostAvailability {
                            foreground,
                            surface_ready: window.is_some(),
                            player_visible: hidden,
                        },
                        restoring,
                        active.failed(),
                    );
                    if native_paused != Some(paused) {
                        platform::trace_current(
                            "native.pause.begin",
                            if paused { "pause" } else { "play" },
                        );
                        runtime.as_mut().unwrap().pause(paused);
                        platform::trace_current(
                            "native.pause.end",
                            if paused { "paused" } else { "playing" },
                        );
                        native_paused = Some(paused);
                        if paused
                            && !restoring
                            && let (Some(active), Some(index)) = (runtime.as_mut(), selected)
                            && !collection.entries[index].restart_required
                            && let Err(error) =
                                save_checkpoint(active, &root, &mut collection, index)
                        {
                            platform::push(Command::Error(format!(
                                "Could not checkpoint the paused game: {error}"
                            )));
                        }
                        if save_collection(&root, &collection).is_err() {
                            if let Some(index) = selected {
                                collection.entries[index].paused = true;
                            }
                            platform::trace_current("native.pause.begin", "pause");
                            runtime.as_mut().unwrap().pause(true);
                            platform::trace_current("native.pause.end", "paused");
                            native_paused = Some(true);
                            platform::push(Command::Error("Could not preserve session settings. The paused game and previous checkpoint are retained.".into()));
                        }
                    }
                    // ABI8 samples Rust input at the actual native pad poll.
                }
            }
            if runtime
                .as_ref()
                .is_some_and(|active| active.events.stopped.load(Ordering::Relaxed))
            {
                let joined = runtime.as_mut().expect("checked live runtime").join();
                hidden = false;
                if let Some(index) = selected {
                    collection.entries[index].runtime_incomplete = joined.is_err();
                    collection.entries[index].stopped = true;
                    if let Err(error) = joined {
                        collection.entries[index].fail(format!(
                            "Card persistence failed during native stop: {error}"
                        ));
                    }
                    if collection.entries[index].restart_required {
                        let mut message = collection.entries[index]
                            .last_error
                            .clone()
                            .unwrap_or_else(|| "The runtime needs an explicit restart.".into());
                        if save_collection(&root, &collection).is_err() {
                            message.push_str("\nCould not preserve this error on disk; it remains on this screen.");
                        }
                        platform::menu(app, true, Some(message));
                    } else {
                        if save_collection(&root, &collection).is_err() {
                            collection.entries[index].runtime_incomplete = true;
                            collection.entries[index]
                                .fail("Could not preserve native stop metadata.".into());
                            let _ = save_collection(&root, &collection);
                        }
                        platform::unloaded_menu(
                            app,
                            &collection.entries[index].title,
                            collection.entries[index]
                                .restart_required
                                .then(|| collection.entries[index].last_error.clone())
                                .flatten(),
                        );
                    }
                } else {
                    platform::menu(app, true, None);
                }
                drop(runtime.take());
            }
        }
    }
    if let (Some(active), Some(index)) = (runtime.as_mut(), selected) {
        let checkpoint = if collection.entries[index].restart_required {
            Err(collection.entries[index]
                .last_error
                .clone()
                .unwrap_or_else(|| "The runtime already requires explicit recovery.".into())
                .into())
        } else if active.pending() {
            active.verify_pending()
        } else {
            save_checkpoint(active, &root, &mut collection, index)
        };
        let barrier = active.pause_commit();
        let joined = active.join().and_then(|()| {
            if active.pending() {
                active.verify_pending().map_err(|e| e.to_string())
            } else {
                active.verify_departure()
            }
        });
        collection.entries[index].runtime_incomplete =
            checkpoint.is_err() || barrier.is_err() || joined.is_err();
        // Activity destruction is a resumable departure, never explicit Stop.
        if let Some(error) = checkpoint
            .err()
            .or_else(|| barrier.err().map(Into::into))
            .or_else(|| joined.err().map(Into::into))
        {
            collection.entries[index].fail(format!("Interrupted departure: {error}"));
        }
    }
    let saved = save_collection(&root, &collection);
    drop(runtime.take());
    saved?;
    Ok(())
}
