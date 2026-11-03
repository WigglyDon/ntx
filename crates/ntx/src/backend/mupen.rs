// SPDX-License-Identifier: MIT
//! Small host for the pinned Mupen64Plus-Next libretro ABI. The host owns
//! presentation/audio resources; this adapter owns console calls and quiescence.
//! Native globals permit one instance per process until isolation is qualified.
use crate::n64::{Cartridge, SaveImage};
use std::{
    cell::Cell,
    collections::BTreeMap,
    ffi::{CStr, CString, c_char, c_void},
    marker::PhantomData,
    path::Path,
    ptr,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};
type Result<T> = std::result::Result<T, String>;
type Void = unsafe extern "C" fn();
static LIVE: AtomicBool = AtomicBool::new(false);
/// Includes a failed native join: the process slot stays occupied until all
/// callbacks, executable code and host resources have actually retired.
pub fn native_owner_present() -> bool {
    LIVE.load(Ordering::Acquire)
}
const HW_CONTEXT_OPENGLES3: u32 = 4;
const HW_CONTEXT_OPENGLES_VERSION: u32 = 5;
thread_local! { static CONTEXT: Cell<*mut Context> = const { Cell::new(ptr::null_mut()) }; }

/// Implementations stay on the owner thread and must not unwind from callbacks.
pub trait Host {
    fn jit_capable(&self) -> bool {
        false
    }
    fn framebuffer(&self) -> usize;
    fn proc_address(&self, name: &CStr) -> *const c_void;
    fn present(&mut self, width: u32, height: u32) -> Result<()>;
    /// May run on the core's alternate coroutine stack. Buffer PCM here;
    /// managed-runtime delivery must wait until the core returns to its host.
    fn audio(&mut self, interleaved_stereo: &[i16]) -> Result<usize>;
    fn poll_input(&mut self);
    fn input(&self, port: u32, device: u32, index: u32, id: u32) -> i16;
    fn silence(&mut self);
}
/// Instance-owned callback handoff, bounded to one second of 44.1 kHz stereo.
/// It owns copied samples, never a borrowed core buffer or Android reference.
#[derive(Default)]
pub struct AudioQueue {
    samples: Vec<i16>,
}
impl AudioQueue {
    const LIMIT: usize = 44100 * 2;
    pub fn push(&mut self, samples: &[i16]) -> Result<usize> {
        if !samples.len().is_multiple_of(2) {
            return Err("Invalid stereo audio callback length".into());
        }
        if samples.len() > Self::LIMIT - self.samples.len() {
            return Err("N64 audio handoff exceeded its bounded capacity".into());
        }
        self.samples.extend_from_slice(samples);
        Ok(samples.len() / 2)
    }
    pub fn clear(&mut self) {
        self.samples.clear();
    }
    /// Invoke only after returning to the ordinary host stack. A delivery error
    /// never replays an already-partly-written batch on a later frame.
    pub fn drain(&mut self, write: impl FnOnce(&[i16]) -> Result<usize>) -> Result<usize> {
        if self.samples.is_empty() {
            return Ok(0);
        }
        let frames = self.samples.len() / 2;
        let result = write(&self.samples);
        self.samples.clear();
        match result {
            Ok(written) if written == frames => Ok(written),
            Ok(_) => Err("Incomplete stereo audio delivery".into()),
            Err(error) => Err(error),
        }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Geometry {
    pub width: u32,
    pub height: u32,
    pub max_width: u32,
    pub max_height: u32,
    pub aspect: f32,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Timing {
    pub fps: f64,
    pub sample_rate: f64,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct AvInfo {
    pub geometry: Geometry,
    pub timing: Timing,
}
fn supported_timing(timing: Timing) -> bool {
    timing.fps.is_finite() && (40.0..=65.0).contains(&timing.fps) && timing.sample_rate == 44100.0
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Hardware {
    context_type: u32,
    reset: Option<Void>,
    framebuffer: Option<unsafe extern "C" fn() -> usize>,
    proc_address: Option<unsafe extern "C" fn(*const c_char) -> *const c_void>,
    depth: bool,
    stencil: bool,
    bottom_left: bool,
    major: u32,
    minor: u32,
    cache: bool,
    destroy: Option<Void>,
    debug: bool,
}
#[repr(C)]
struct Variable {
    key: *const c_char,
    value: *const c_char,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct OptionValue {
    value: *const c_char,
    label: *const c_char,
}
#[repr(C)]
struct Definition {
    key: *const c_char,
    description: *const c_char,
    description_category: *const c_char,
    info: *const c_char,
    info_category: *const c_char,
    category: *const c_char,
    values: [OptionValue; 128],
    default_value: *const c_char,
}
#[repr(C)]
struct Options {
    categories: *const c_void,
    definitions: *const Definition,
}
#[repr(C)]
struct GameInfo {
    path: *const c_char,
    data: *const c_void,
    size: usize,
    meta: *const c_char,
}
#[repr(C)]
struct SystemInfo {
    name: *const c_char,
    version: *const c_char,
    extensions: *const c_char,
    full_path: bool,
    block_extract: bool,
}
struct Context {
    host: Box<dyn Host>,
    system: CString,
    cache: CString,
    options: BTreeMap<Vec<u8>, CString>,
    hardware: Option<Hardware>,
    av: AvInfo,
    error: Option<String>,
    frames: u64,
    audio_frames: u64,
}
impl Context {
    fn fail(&mut self, error: String) {
        if self.error.is_none() {
            self.error = Some(error);
        }
    }
}
fn with_context<T>(default: T, f: impl FnOnce(&mut Context) -> T) -> T {
    CONTEXT.with(|slot| {
        let p = slot.get();
        if p.is_null() {
            default
        } else {
            // SAFETY: the pinned core calls on its owner thread; Box remains stable
            // through deinit and no adapter borrow spans a call into the core.
            f(unsafe { &mut *p })
        }
    })
}
unsafe extern "C" fn framebuffer() -> usize {
    with_context(0, |c| c.host.framebuffer())
}
unsafe extern "C" fn proc_address(name: *const c_char) -> *const c_void {
    if name.is_null() {
        return ptr::null();
    }
    // SAFETY: selected libretro ABI supplies a NUL-terminated symbol name.
    with_context(ptr::null(), |c| {
        c.host.proc_address(unsafe { CStr::from_ptr(name) })
    })
}
unsafe extern "C" fn video(data: *const c_void, width: u32, height: u32, _pitch: usize) {
    with_context((), |c| {
        if data.is_null() {
            return;
        } // A duplicate frame does not claim a new presentation.
        if data as usize != usize::MAX || width == 0 || height == 0 || width > 2048 || height > 2048
        {
            c.fail("Unsupported N64 video delivery".into());
            return;
        }
        match c.host.present(width, height) {
            Ok(()) => c.frames += 1,
            Err(e) => c.fail(e),
        }
    });
}
unsafe extern "C" fn audio(data: *const i16, frames: usize) -> usize {
    if data.is_null() || frames > 65536 {
        return 0;
    }
    with_context(0, |c| {
        // SAFETY: the synchronous pinned ABI lends frames*2 stereo samples.
        let samples = unsafe { std::slice::from_raw_parts(data, frames * 2) };
        match c.host.audio(samples) {
            Ok(n) if n <= frames => {
                c.audio_frames += n as u64;
                n
            }
            Ok(_) => {
                c.fail("Invalid N64 audio acknowledgement".into());
                0
            }
            Err(e) => {
                c.fail(e);
                0
            }
        }
    })
}
unsafe extern "C" fn audio_one(left: i16, right: i16) {
    let pair = [left, right];
    unsafe { audio(pair.as_ptr(), 1) };
}
unsafe extern "C" fn poll() {
    with_context((), |c| c.host.poll_input());
}
unsafe extern "C" fn input(port: u32, device: u32, index: u32, id: u32) -> i16 {
    with_context(0, |c| c.host.input(port, device, index, id))
}
unsafe extern "C" fn environment(command: u32, data: *mut c_void) -> bool {
    with_context(false, |c| {
        if command == (51 | 0x10000) {
            return true;
        }
        if data.is_null() {
            return false;
        }
        // SAFETY: payload layouts below are the selected version-1 libretro ABI.
        // Optional/unknown interfaces fail without dereferencing their payload.
        unsafe {
            match command {
                3 => {
                    *data.cast::<bool>() = true;
                    true
                }
                9 => {
                    *data.cast::<*const c_char>() = c.system.as_ptr();
                    true
                }
                10 => *data.cast::<u32>() == 1,
                11 | 18 | 35 | 55 => true,
                14 => {
                    let hw = &mut *data.cast::<Hardware>();
                    if hw.context_type != HW_CONTEXT_OPENGLES3
                        && !(hw.context_type == HW_CONTEXT_OPENGLES_VERSION
                            && hw.major == 3
                            && hw.minor == 0)
                    {
                        return false;
                    }
                    hw.framebuffer = Some(framebuffer);
                    hw.proc_address = Some(proc_address);
                    c.hardware = Some(*hw);
                    true
                }
                15 => {
                    let var = &mut *data.cast::<Variable>();
                    if var.key.is_null() {
                        return false;
                    }
                    let key = CStr::from_ptr(var.key).to_bytes();
                    var.value = c
                        .options
                        .get(key)
                        .map_or(ptr::null(), |value| value.as_ptr());
                    !var.value.is_null()
                }
                17 => {
                    *data.cast::<bool>() = false;
                    true
                }
                31 => {
                    *data.cast::<*const c_char>() = c.cache.as_ptr();
                    true
                }
                32 => {
                    let av = *data.cast::<AvInfo>();
                    if !supported_timing(av.timing) {
                        c.fail("Unsupported N64 timing/audio update".into());
                        return false;
                    }
                    c.av = av;
                    true
                }
                37 => {
                    c.av.geometry = *data.cast::<Geometry>();
                    true
                }
                39 => {
                    *data.cast::<u32>() = 0;
                    true
                }
                52 => {
                    *data.cast::<u32>() = 2;
                    true
                }
                59 => {
                    *data.cast::<u32>() = 0;
                    true
                }
                67 | 68 => {
                    let options = if command == 67 {
                        data.cast::<Options>()
                    } else {
                        *data.cast::<*const Options>()
                    };
                    if options.is_null() || (*options).definitions.is_null() {
                        return false;
                    }
                    for i in 0..1024 {
                        let item = &*(*options).definitions.add(i);
                        if item.key.is_null() {
                            break;
                        }
                        let value = if item.default_value.is_null() {
                            item.values[0].value
                        } else {
                            item.default_value
                        };
                        if value.is_null() {
                            continue;
                        }
                        c.options
                            .entry(CStr::from_ptr(item.key).to_bytes().to_vec())
                            .or_insert_with(|| CStr::from_ptr(value).to_owned());
                    }
                    false // Defaults accepted; a user-facing option-category UI is not advertised.
                }
                74 => {
                    *data.cast::<bool>() = c.host.jit_capable();
                    true
                } // Ordinary host executable-memory capability; no security modification.
                n if n == (47 | 0x10000) => {
                    *data.cast::<i32>() = 3;
                    true
                }
                n if n == (50 | 0x10000) => {
                    *data.cast::<f32>() = 60.0;
                    true
                }
                n if n == (51 | 0x10000) => true,
                _ => false,
            }
        }
    })
}

#[link(name = "dl")]
unsafe extern "C" {
    fn dlopen(name: *const c_char, flags: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> i32;
}
struct Library(*mut c_void);
impl Library {
    unsafe fn symbol<T: Copy>(&self, name: &CStr) -> Result<T> {
        let p = unsafe { dlsym(self.0, name.as_ptr()) };
        if p.is_null() {
            return Err(format!(
                "N64 core ABI is missing {}",
                name.to_string_lossy()
            ));
        }
        if std::mem::size_of::<T>() != std::mem::size_of_val(&p) {
            return Err("Invalid native function pointer layout".into());
        }
        // SAFETY: only exact known ABI function pointer types are requested.
        Ok(unsafe { std::mem::transmute_copy(&p) })
    }
}
impl Drop for Library {
    fn drop(&mut self) {
        unsafe { dlclose(self.0) };
    }
}
struct Api {
    set_environment: unsafe extern "C" fn(unsafe extern "C" fn(u32, *mut c_void) -> bool),
    set_video: unsafe extern "C" fn(unsafe extern "C" fn(*const c_void, u32, u32, usize)),
    set_audio: unsafe extern "C" fn(unsafe extern "C" fn(i16, i16)),
    set_audio_batch: unsafe extern "C" fn(unsafe extern "C" fn(*const i16, usize) -> usize),
    set_poll: unsafe extern "C" fn(unsafe extern "C" fn()),
    set_input: unsafe extern "C" fn(unsafe extern "C" fn(u32, u32, u32, u32) -> i16),
    api_version: unsafe extern "C" fn() -> u32,
    init: Void,
    deinit: Void,
    shutdown_complete: unsafe extern "C" fn() -> bool,
    unload: Void,
    run: Void,
    info: unsafe extern "C" fn(*mut SystemInfo),
    av: unsafe extern "C" fn(*mut AvInfo),
    load: unsafe extern "C" fn(*const GameInfo) -> bool,
    controller: unsafe extern "C" fn(u32, u32),
    memory: unsafe extern "C" fn(u32) -> *mut c_void,
    memory_size: unsafe extern "C" fn(u32) -> usize,
    serialize_size: unsafe extern "C" fn() -> usize,
    serialize: unsafe extern "C" fn(*mut c_void, usize) -> bool,
    unserialize: unsafe extern "C" fn(*const c_void, usize) -> bool,
}
impl Api {
    unsafe fn load(l: &Library) -> Result<Self> {
        macro_rules! sym {
            ($s:literal) => {
                unsafe {
                    l.symbol(CStr::from_bytes_with_nul(concat!($s, "\0").as_bytes()).unwrap())?
                }
            };
        }
        Ok(Self {
            set_environment: sym!("retro_set_environment"),
            set_video: sym!("retro_set_video_refresh"),
            set_audio: sym!("retro_set_audio_sample"),
            set_audio_batch: sym!("retro_set_audio_sample_batch"),
            set_poll: sym!("retro_set_input_poll"),
            set_input: sym!("retro_set_input_state"),
            api_version: sym!("retro_api_version"),
            init: sym!("retro_init"),
            deinit: sym!("retro_deinit"),
            shutdown_complete: sym!("retro_ntx_shutdown_complete"),
            unload: sym!("retro_unload_game"),
            run: sym!("retro_run"),
            info: sym!("retro_get_system_info"),
            av: sym!("retro_get_system_av_info"),
            load: sym!("retro_load_game"),
            controller: sym!("retro_set_controller_port_device"),
            memory: sym!("retro_get_memory_data"),
            memory_size: sym!("retro_get_memory_size"),
            serialize_size: sym!("retro_serialize_size"),
            serialize: sym!("retro_serialize"),
            unserialize: sym!("retro_unserialize"),
        })
    }
}
struct Slot {
    release: bool,
}
impl Drop for Slot {
    fn drop(&mut self) {
        if self.release {
            LIVE.store(false, Ordering::Release);
        }
    }
}
pub struct Core {
    api: Api,
    context: std::mem::ManuallyDrop<Box<Context>>,
    initialized: bool,
    loaded: bool,
    quarantined: bool,
    _library: Option<Library>,
    _slot: Slot,
    _owner: PhantomData<Rc<()>>,
}
impl Core {
    /// The host has a current GLES3 context and framebuffer before this call.
    /// It stays alive through all completion-bearing stop/deinit callbacks.
    pub fn load(
        host: Box<dyn Host>,
        cartridge: &Cartridge,
        system: &Path,
        cache: &Path,
        ordinary: Option<&[u8]>,
    ) -> Result<Self> {
        if LIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("A native N64 instance already owns this process".into());
        }
        let slot = Slot { release: true };
        for path in [system, cache] {
            std::fs::create_dir_all(path).map_err(|e| e.to_string())?;
        }
        let handle = unsafe { dlopen(c"libntx_mupen64plus_next.so".as_ptr(), 2) };
        if handle.is_null() {
            return Err("The packaged N64 core could not be loaded".into());
        }
        let library = Library(handle);
        let api = unsafe { Api::load(&library)? };
        if unsafe { (api.api_version)() } != 1 {
            return Err("Unsupported N64 core ABI".into());
        }
        let mut options = BTreeMap::new();
        for (key, value) in [
            ("rdp-plugin", "gliden64"),
            ("rsp-plugin", "hle"),
            ("cpucore", "dynamic_recompiler"),
            ("ThreadedRenderer", "False"),
            ("EnableShadersStorage", "True"),
            ("alt-map", "False"),
            ("astick-deadzone", "0"),
            ("astick-sensitivity", "100"),
            ("pak1", "memory"),
            ("pak2", "none"),
            ("pak3", "none"),
            ("pak4", "none"),
            ("43screensize", "640x480"),
        ] {
            options.insert(
                format!("mupen64plus-{key}").into_bytes(),
                CString::new(value).unwrap(),
            );
        }
        let mut context = Box::new(Context {
            host,
            system: CString::new(system.to_str().ok_or("Invalid N64 system path")?)
                .map_err(|e| e.to_string())?,
            cache: CString::new(cache.to_str().ok_or("Invalid N64 backend path")?)
                .map_err(|e| e.to_string())?,
            options,
            hardware: None,
            av: AvInfo::default(),
            error: None,
            frames: 0,
            audio_frames: 0,
        });
        CONTEXT.with(|p| p.set(&mut *context));
        let mut core = Self {
            api,
            context: std::mem::ManuallyDrop::new(context),
            initialized: false,
            loaded: false,
            quarantined: false,
            _library: Some(library),
            _slot: slot,
            _owner: PhantomData,
        };
        // SAFETY: callbacks/context are stable and the exact selected ABI is loaded.
        unsafe {
            (core.api.set_environment)(environment);
            (core.api.set_video)(video);
            (core.api.set_audio)(audio_one);
            (core.api.set_audio_batch)(audio);
            (core.api.set_poll)(poll);
            (core.api.set_input)(input);
            let mut info = SystemInfo {
                name: ptr::null(),
                version: ptr::null(),
                extensions: ptr::null(),
                full_path: false,
                block_extract: false,
            };
            (core.api.info)(&mut info);
            if info.full_path
                || info.name.is_null()
                || !CStr::from_ptr(info.name)
                    .to_bytes()
                    .starts_with(b"Mupen64Plus")
            {
                return Err("Unexpected N64 content interface".into());
            }
            (core.api.init)();
            core.initialized = true;
            let game = GameInfo {
                path: ptr::null(),
                data: cartridge.bytes.as_ptr().cast(),
                size: cartridge.bytes.len(),
                meta: ptr::null(),
            };
            if !(core.api.load)(&game) {
                return Err("N64 core rejected the cartridge or GLES context".into());
            }
            core.loaded = true;
            if let Some(bytes) = ordinary {
                SaveImage::parse(bytes).map_err(|e| e.to_string())?;
                core.restore_ordinary(bytes)?;
            }
            let reset = core
                .context
                .hardware
                .and_then(|h| h.reset)
                .ok_or("N64 core did not request a supported GLES context")?;
            reset();
            for port in 0..4 {
                (core.api.controller)(port, if port == 0 { 1 } else { 0 });
            }
            (core.api.av)(&mut core.context.av);
        }
        core.check()?;
        let timing = core.context.av.timing;
        if !supported_timing(timing) {
            return Err("Unsupported N64 timing/audio contract".into());
        }
        Ok(core)
    }
    fn check(&mut self) -> Result<()> {
        self.context.error.take().map_or(Ok(()), Err)
    }
    pub fn av_info(&self) -> AvInfo {
        self.context.av
    }
    /// Successful callback acknowledgments. A host may discard muted audio or
    /// accept video without a visible surface; these are not output evidence.
    pub fn observations(&self) -> (u64, u64) {
        (self.context.frames, self.context.audio_frames)
    }
    pub fn options_sha256(&self) -> String {
        let mut bytes = Vec::new();
        for (k, v) in &self.context.options {
            bytes.extend(k);
            bytes.push(0);
            bytes.extend(v.as_bytes());
            bytes.push(0);
        }
        crate::storage::digest(&bytes)
    }
    pub fn run_frame(&mut self) -> Result<()> {
        if !self.loaded || self.quarantined {
            return Err("N64 runtime is stopped or requires recovery".into());
        }
        unsafe { (self.api.run)() };
        self.check()
    }
    pub fn silence(&mut self) {
        self.context.host.silence();
    }
    pub fn ordinary_image(&self) -> Result<Vec<u8>> {
        if self.quarantined {
            return Err("Failed N64 restore is quarantined from ordinary saves".into());
        }
        let n = unsafe { (self.api.memory_size)(0) };
        let p = unsafe { (self.api.memory)(0) };
        if n != crate::n64::SAVE_IMAGE_BYTES || p.is_null() {
            return Err("N64 ordinary-save interface differs from pinned format".into());
        }
        // SAFETY: quiescent selected core lends its static complete save region.
        Ok(unsafe { std::slice::from_raw_parts(p.cast::<u8>(), n) }.to_vec())
    }
    fn restore_ordinary(&mut self, bytes: &[u8]) -> Result<()> {
        SaveImage::parse(bytes).map_err(|e| e.to_string())?;
        let n = unsafe { (self.api.memory_size)(0) };
        let p = unsafe { (self.api.memory)(0) };
        if n != bytes.len() || p.is_null() {
            return Err("N64 ordinary-save buffer mismatch".into());
        }
        // SAFETY: before context_reset/first execution, after core save formatting.
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), p.cast::<u8>(), n) };
        Ok(())
    }
    /// # Safety
    /// Only at a quiescent cold initialization boundary immediately before a
    /// validated machine restore. The caller owns the exact current generation;
    /// this is never permission to attach arbitrary newer data to an old scene.
    pub unsafe fn reseed_for_restore(&mut self, bytes: &[u8]) -> Result<()> {
        if !self.loaded || self.quarantined {
            return Err("N64 ordinary reseed unavailable".into());
        }
        self.restore_ordinary(bytes)
    }
    pub fn capture(&mut self) -> Result<Vec<u8>> {
        if self.quarantined || !self.loaded {
            return Err("N64 state capture is unavailable".into());
        }
        let n = unsafe { (self.api.serialize_size)() };
        if n == 0 || n > 64 * 1024 * 1024 {
            return Err("Invalid N64 state size".into());
        }
        let mut bytes = vec![0; n];
        if !unsafe { (self.api.serialize)(bytes.as_mut_ptr().cast(), n) } {
            return Err("N64 core did not complete state capture".into());
        }
        self.check()?;
        Ok(bytes)
    }
    /// # Safety
    /// Validate exact backend/build/options/architecture/media binding and current
    /// ordinary generation before calling. Never publish save bytes after failure.
    pub unsafe fn restore(&mut self, bytes: &[u8]) -> Result<()> {
        if self.quarantined || !self.loaded || bytes.len() != unsafe { (self.api.serialize_size)() }
        {
            return Err("N64 restore binding/size is unavailable".into());
        }
        let restored = unsafe { (self.api.unserialize)(bytes.as_ptr().cast(), bytes.len()) };
        let callback_result = self.check();
        if !restored || callback_result.is_err() {
            self.quarantined = true;
            return Err(
                "N64 restore failed; native memory is quarantined and ordinary progress retained"
                    .into(),
            );
        }
        Ok(())
    }
    /// Join native execution while the host/context/profile remain owned. The
    /// selected core's static save region remains valid until dlclose, so callers
    /// can commit its final post-join ordinary bytes before releasing the writer.
    pub fn stop(&mut self) -> Result<()> {
        self.silence();
        unsafe {
            if self.loaded {
                (self.api.unload)();
                self.loaded = false;
            }
            if self.initialized {
                (self.api.deinit)();
                if !(self.api.shutdown_complete)() {
                    return Err("N64 native shutdown did not acknowledge completion; host, writer and native memory must remain retained".into());
                }
                self.initialized = false;
            }
            if let Some(destroy) = self.context.hardware.take().and_then(|h| h.destroy) {
                destroy();
            }
        }
        self.check()
    }
}
impl Drop for Core {
    fn drop(&mut self) {
        let _ = self.stop();
        if self.initialized || self.loaded {
            // A failed join may not free the callback owner or executable code.
            // Preserve the actual process slot and host resources for recovery.
            self._slot.release = false;
            if let Some(library) = self._library.take() {
                std::mem::forget(library);
            }
            return;
        }
        // Shared-library destructors finish while host graphics/audio and the
        // callback context are still alive. No native owner outlives the slot.
        drop(self._library.take());
        CONTEXT.with(|c| c.set(ptr::null_mut()));
        // SAFETY: completed stop, DLL destructors and callback retirement precede
        // release of graphics/audio and their strong surface references.
        unsafe {
            std::mem::ManuallyDrop::drop(&mut self.context);
        }
    }
}

#[cfg(all(test, target_pointer_width = "64"))]
mod tests {
    use super::*;
    #[test]
    fn audio_callbacks_copy_in_order_and_deliver_only_on_explicit_host_drain() {
        let mut queue = AudioQueue::default();
        let mut source = [1, -1, 2, -2];
        assert_eq!(queue.push(&source), Ok(2));
        source.fill(99);
        assert_eq!(queue.push(&[3, -3]), Ok(1));
        let mut writes = 0;
        assert_eq!(
            queue.drain(|samples| {
                writes += 1;
                assert_eq!(samples, &[1, -1, 2, -2, 3, -3]);
                Ok(3)
            }),
            Ok(3)
        );
        assert_eq!(writes, 1);
        assert_eq!(
            queue.drain(|_| panic!("empty queue must not call the device")),
            Ok(0)
        );
        queue.push(&[4, -4]).unwrap();
        assert!(
            queue
                .drain(|_| Err("partly written device failure".into()))
                .is_err()
        );
        assert_eq!(
            queue.drain(|_| panic!("failed batch must not replay")),
            Ok(0)
        );
        queue.push(&[5, -5, 6, -6]).unwrap();
        assert!(queue.drain(|_| Ok(1)).is_err());
        assert_eq!(
            queue.drain(|_| panic!("partly delivered batch must not replay")),
            Ok(0)
        );
    }
    #[test]
    fn audio_handoff_bounds_and_silence_preserve_stereo_without_old_batch_replay() {
        let mut queue = AudioQueue::default();
        queue.push(&[5, -5]).unwrap();
        assert!(queue.push(&[1]).is_err());
        assert!(queue.push(&vec![0; AudioQueue::LIMIT]).is_err());
        assert_eq!(
            queue.drain(|samples| {
                assert_eq!(samples, &[5, -5]);
                Ok(1)
            }),
            Ok(1)
        );
        queue.push(&[6, -6]).unwrap();
        queue.clear();
        assert_eq!(
            queue.drain(|_| panic!("silenced PCM must not reach the device")),
            Ok(0)
        );
    }
    struct ContextHost;
    impl Host for ContextHost {
        fn framebuffer(&self) -> usize {
            7
        }
        fn proc_address(&self, _: &CStr) -> *const c_void {
            ptr::null()
        }
        fn present(&mut self, _: u32, _: u32) -> Result<()> {
            Ok(())
        }
        fn audio(&mut self, _: &[i16]) -> Result<usize> {
            Ok(0)
        }
        fn poll_input(&mut self) {}
        fn input(&self, _: u32, _: u32, _: u32, _: u32) -> i16 {
            0
        }
        fn silence(&mut self) {}
    }
    #[test]
    fn actual_hardware_callback_accepts_pinned_gles3_and_refuses_desktop_gl() {
        let mut context = Box::new(Context {
            host: Box::new(ContextHost),
            system: CString::new("system").unwrap(),
            cache: CString::new("cache").unwrap(),
            options: BTreeMap::new(),
            hardware: None,
            av: AvInfo::default(),
            error: None,
            frames: 0,
            audio_frames: 0,
        });
        CONTEXT.with(|slot| {
            assert!(slot.get().is_null());
            slot.set(&mut *context);
        });
        for (kind, major, minor, accepted) in [
            (4, 0, 0, true),
            (5, 3, 0, true),
            (3, 3, 0, false),
            (2, 2, 0, false),
            (5, 3, 1, false),
        ] {
            context.hardware = None;
            let mut hw = Hardware {
                context_type: kind,
                reset: None,
                framebuffer: None,
                proc_address: None,
                depth: true,
                stencil: false,
                bottom_left: true,
                major,
                minor,
                cache: false,
                destroy: None,
                debug: false,
            };
            // SAFETY: the actual selected ABI callback receives a valid Hardware
            // payload and thread-local host, without loading/executing a core.
            assert_eq!(
                unsafe { environment(14, (&mut hw as *mut Hardware).cast()) },
                accepted
            );
            assert_eq!(context.hardware.is_some(), accepted);
            if accepted {
                assert!(hw.framebuffer.is_some() && hw.proc_address.is_some());
            }
        }
        CONTEXT.with(|slot| slot.set(ptr::null_mut()));
    }
    #[test]
    fn selected_libretro_ffi_layout_matches_64_bit_header_contract() {
        assert_eq!(std::mem::size_of::<Hardware>(), 64);
        assert_eq!(std::mem::offset_of!(Hardware, major), 36);
        assert_eq!(std::mem::offset_of!(Hardware, destroy), 48);
        assert_eq!(std::mem::size_of::<Definition>(), 2104);
        assert_eq!(std::mem::size_of::<GameInfo>(), 32);
        assert_eq!(std::mem::size_of::<SystemInfo>(), 32);
        assert_eq!(std::mem::size_of::<AvInfo>(), 40);
    }
}
