// SPDX-License-Identifier: MIT
use android_activity::AndroidApp;
use jni::{
    JNIEnv, JavaVM,
    objects::{GlobalRef, JClass, JObject, JString, JValue},
    sys::{jint, jlong},
};
use std::{
    collections::VecDeque,
    ffi::{CStr, c_char},
    fs::File,
    io::{Seek, SeekFrom},
    os::fd::{FromRawFd, IntoRawFd},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
#[path = "controls_ui.rs"]
mod controls_ui;
#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(priority: i32, tag: *const c_char, text: *const c_char) -> i32;
}
/// NDK motion timestamps use System.nanoTime's CLOCK_MONOTONIC time base.
/// This is foreground event age, not physical touch or display latency.
pub fn motion_age(event_nanos: i64) -> Option<std::time::Duration> {
    #[repr(C)]
    struct Timespec {
        seconds: std::ffi::c_long,
        nanos: std::ffi::c_long,
    }
    unsafe extern "C" {
        fn clock_gettime(clock: i32, time: *mut Timespec) -> i32;
    }
    let mut time = Timespec {
        seconds: 0,
        nanos: 0,
    };
    // SAFETY: initialized timespec output; CLOCK_MONOTONIC is1 on stock Android.
    if unsafe { clock_gettime(1, &mut time) } != 0 {
        return None;
    }
    let now = u64::try_from(time.seconds)
        .ok()?
        .checked_mul(1_000_000_000)?
        .checked_add(u64::try_from(time.nanos).ok()?)?;
    let event = u64::try_from(event_nanos).ok()?;
    if event == 0 {
        return None;
    }
    Some(std::time::Duration::from_nanos(now.checked_sub(event)?))
}
pub fn diagnostic(message: &str) {
    let message: String = message.chars().filter(|c| *c != '\0').take(4096).collect();
    if let Ok(message) = std::ffi::CString::new(message) {
        // SAFETY: Android copies these borrowed NUL-terminated strings.
        unsafe {
            __android_log_write(6, c"ntx".as_ptr(), message.as_ptr());
        }
    }
}
static VM: OnceLock<JavaVM> = OnceLock::new();
static ACTIVITY: Mutex<Option<GlobalRef>> = Mutex::new(None);
static MENU: Mutex<Option<GlobalRef>> = Mutex::new(None);
static COMMANDS: Mutex<VecDeque<Command>> = Mutex::new(VecDeque::new());
static EPOCH: AtomicU64 = AtomicU64::new(0);
static MENU_GENERATION: AtomicU64 = AtomicU64::new(0);
static PLAYING: AtomicBool = AtomicBool::new(false);
// Diagnostic-only: one bounded burst per Activity, never per native pad/frame.
static TRACE_START: OnceLock<std::time::Instant> = OnceLock::new();
// The same192-event cap reserves lifecycle/rejection evidence for Stop.
static TRACE_EVENTS: ntx::lifecycle::DiagnosticBudget = ntx::lifecycle::DiagnosticBudget::new(128);
static TRACE_SAMPLES: ntx::lifecycle::DiagnosticBudget = ntx::lifecycle::DiagnosticBudget::new(48);
static TRACE_INPUT: ntx::lifecycle::DiagnosticBudget = ntx::lifecycle::DiagnosticBudget::new(32);
static TRACE_INPUT_FAILURES: ntx::lifecycle::DiagnosticBudget =
    ntx::lifecycle::DiagnosticBudget::new(16);
static TRACE_GEOMETRY: ntx::lifecycle::DiagnosticBudget = ntx::lifecycle::DiagnosticBudget::new(24);
static TRACE_RUNTIME: AtomicU64 = AtomicU64::new(0);
static TRACE_PRESENT_CAP_REPORTED: AtomicBool = AtomicBool::new(false);
static TRACE_OPTIONAL_CAP_REPORTED: AtomicBool = AtomicBool::new(false);
static TRACE_ACK_CAP_REPORTED: AtomicBool = AtomicBool::new(false);

pub fn trace_runtime(runtime: u64) {
    TRACE_RUNTIME.store(runtime, Ordering::Relaxed);
}
pub fn trace(runtime: u64, menu: u64, stage: &str, detail: &str) {
    if TRACE_EVENTS.take_lifecycle_stage(stage) {
        emit_trace(runtime, menu, stage, detail);
    } else {
        let optional =
            ntx::lifecycle::lifecycle_trace_reserve(stage) == ntx::lifecycle::LIFECYCLE_ACK_RESERVE;
        let reported = if optional {
            &TRACE_OPTIONAL_CAP_REPORTED
        } else {
            &TRACE_ACK_CAP_REPORTED
        };
        // Each marker spends one of the existing128 slots; no cap expansion.
        if !reported.swap(true, Ordering::Relaxed) && TRACE_EVENTS.take() {
            emit_trace(
                runtime,
                menu,
                if optional {
                    "lifecycle.optional_limit"
                } else {
                    "lifecycle.ack_limit"
                },
                if optional {
                    "optional records omitted; native acknowledgment and Stop slots reserved"
                } else {
                    "trace cap: native state acknowledgments unavailable"
                },
            );
        }
    }
}
fn emit_trace(runtime: u64, menu: u64, stage: &str, detail: &str) {
    let elapsed = TRACE_START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_micros();
    diagnostic(&format!(
        "Lifecycle +{elapsed}us a={} r={runtime} m={menu} {stage} {detail}",
        EPOCH.load(Ordering::Acquire)
    ));
}
fn emit_current(stage: &str, detail: &str) {
    emit_trace(
        TRACE_RUNTIME.load(Ordering::Relaxed),
        MENU_GENERATION.load(Ordering::Acquire),
        stage,
        detail,
    );
}
pub fn trace_current(stage: &str, detail: &str) {
    trace(
        TRACE_RUNTIME.load(Ordering::Relaxed),
        MENU_GENERATION.load(Ordering::Acquire),
        stage,
        detail,
    );
}
pub fn trace_input(detail: &str) {
    if TRACE_INPUT.take()
        && TRACE_SAMPLES.take_preserving(ntx::lifecycle::PRESENTATION_TRACE_RESERVE)
    {
        emit_current("input.down", detail);
    }
}
pub fn trace_input_problem(detail: &str) {
    if TRACE_INPUT_FAILURES.take() {
        emit_current("input.down", detail);
    }
}
pub fn trace_focus() {
    // Per-focus samples share a fixed Activity budget; they cannot spend
    // lifecycle or rejection reserves.
    TRACE_INPUT.reset(32);
    TRACE_GEOMETRY.reset(24);
    trace_current("activity.focus", "gained");
}
pub fn trace_geometry(detail: &str) {
    if TRACE_GEOMETRY.take()
        && TRACE_SAMPLES.take_preserving(ntx::lifecycle::PRESENTATION_TRACE_RESERVE)
    {
        emit_current("geometry", detail);
    }
}

/// A successful presentation is input-admission evidence, independent of the
/// per-focus measurement budget. It still shares the fixed48-sample hard cap.
pub fn trace_presentation(detail: &str) {
    if TRACE_SAMPLES.take() {
        emit_current("geometry", detail);
    } else if !TRACE_PRESENT_CAP_REPORTED.swap(true, Ordering::Relaxed) {
        trace_current("geometry", "presentation trace cap reached");
    }
}

pub enum Command {
    NativeError {
        game_id: String,
        epoch: u64,
        message: String,
    },
    Choose,
    Restart,
    Resume(u64),
    Stop(u64),
    Open,
    Library,
    Controls(u64, f32, f32),
    ApplyControls(u64, ntx::controls::Settings),
    N64Comfort(u64, i32),
    ControlsBack(u64),
    About(u64),
    Game(String),
    PinCurrent,
    Selected(String),
    DocumentSelected(String),
    ConfirmLegacyMedia(u64),
    Canceled,
    MenuHidden(u64),
    Error(String),
    Notice(String),
}
pub fn push(command: Command) {
    if let Ok(mut queue) = COMMANDS.lock() {
        if matches!(command, Command::Game(_)) {
            queue.retain(|c| !matches!(c, Command::Game(_)));
        }
        if queue.len() >= 16 {
            // Preserve at least the first actionable failure through an alert burst.
            let redundant = match &command {
                Command::Error(_) => queue.iter().any(|c| matches!(c, Command::Error(_))),
                Command::NativeError { epoch, .. } => queue
                    .iter()
                    .any(|c| matches!(c,Command::NativeError {epoch:old,..} if old==epoch)),
                _ => true,
            };
            if redundant {
                return;
            }
            queue.pop_back();
        }
        queue.push_back(command);
    }
}
pub fn commands() -> Vec<Command> {
    COMMANDS
        .lock()
        .map(|mut q| q.drain(..).collect())
        .unwrap_or_default()
}
pub fn init(app: &AndroidApp) -> Result<()> {
    EPOCH.fetch_add(1, Ordering::AcqRel);
    TRACE_EVENTS.reset(128);
    TRACE_SAMPLES.reset(48);
    TRACE_PRESENT_CAP_REPORTED.store(false, Ordering::Relaxed);
    TRACE_OPTIONAL_CAP_REPORTED.store(false, Ordering::Relaxed);
    TRACE_ACK_CAP_REPORTED.store(false, Ordering::Relaxed);
    TRACE_INPUT.reset(32);
    TRACE_INPUT_FAILURES.reset(16);
    TRACE_GEOMETRY.reset(24);
    trace_runtime(0);
    HAPTIC_OUTCOMES.store(0, Ordering::Relaxed);
    trace_current("activity.init", "");
    PLAYING.store(false, Ordering::Release);
    COMMANDS
        .lock()
        .map_err(|_| "command lock poisoned")?
        .clear();
    // SAFETY: AndroidApp supplies the process VM and NativeActivity's global object.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast())? };
    let _ = VM.set(vm);
    // NativeWindow's final release can destroy Android framework JNI objects.
    // Keep android_main attached through all window drops, including shutdown.
    let env = VM
        .get()
        .ok_or("VM unavailable")?
        .attach_current_thread_permanently()?;
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    *ACTIVITY.lock().map_err(|_| "activity lock poisoned")? = Some(env.new_global_ref(activity)?);
    let epoch = EPOCH.load(Ordering::Acquire);
    app.run_on_java_main_thread(Box::new(move || {
        if EPOCH.load(Ordering::Acquire) != epoch {
            return;
        }
        let _ = with_env(|env, activity| {
            let window = env
                .call_method(activity, "getWindow", "()Landroid/view/Window;", &[])?
                .l()?;
            let decor = env
                .call_method(window, "getDecorView", "()Landroid/view/View;", &[])?
                .l()?;
            let class = activity_class(env, activity, "com.machinerhythm.ntx.NtxActivity$Insets")?;
            let listener = env.new_object(
                class,
                "(Lcom/machinerhythm/ntx/NtxActivity;)V",
                &[JValue::Object(activity)],
            )?;
            env.call_method(
                &decor,
                "setOnApplyWindowInsetsListener",
                "(Landroid/view/View$OnApplyWindowInsetsListener;)V",
                &[JValue::Object(&listener)],
            )?;
            update_window(env, activity);
            Ok(())
        });
    }));
    Ok(())
}
pub(crate) fn with_env<T>(f: impl FnOnce(&mut JNIEnv<'_>, &JObject<'_>) -> Result<T>) -> Result<T> {
    let activity = ACTIVITY
        .lock()
        .map_err(|_| "activity lock poisoned")?
        .clone()
        .ok_or("activity unavailable")?;
    let mut env = VM.get().ok_or("VM unavailable")?.attach_current_thread()?;
    // Callers return owned Rust values or global references. Android's Java
    // main thread stays attached, so temporary JNI references need a frame.
    let result = env.with_local_frame(64, |env| f(env, activity.as_obj()));
    if env.exception_check()? {
        // Preserve the actual app-local Java failure before clearing it. Raw
        // diagnostics stay private; UI errors still use the recovery policy.
        env.exception_describe()?;
        env.exception_clear()?;
    }
    if let Err(error) = &result {
        diagnostic(&format!("Android UI/provider failure: {error}"));
    }
    result
}
fn activity_class<'a>(
    env: &mut JNIEnv<'a>,
    activity: &JObject<'_>,
    name: &str,
) -> Result<JClass<'a>> {
    let class = env
        .call_method(activity, "getClass", "()Ljava/lang/Class;", &[])?
        .l()?;
    let loader = env
        .call_method(class, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?
        .l()?;
    let name = env.new_string(name)?;
    Ok(JClass::from(
        env.call_method(
            loader,
            "loadClass",
            "(Ljava/lang/String;)Ljava/lang/Class;",
            &[JValue::Object(&name)],
        )?
        .l()?,
    ))
}
fn sdk(env: &mut JNIEnv<'_>) -> Result<i32> {
    Ok(env
        .get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?
        .i()?)
}
/// App-window policy only. No gesture exclusion, global settings or navigation
/// mode changes. Stable inset clearance remains even during transient reveals.
fn playing_window(env: &mut JNIEnv<'_>, activity: &JObject<'_>, playing: bool) -> Result<()> {
    let window = env
        .call_method(activity, "getWindow", "()Landroid/view/Window;", &[])?
        .l()?;
    let decor = env
        .call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?
        .l()?;
    let api = sdk(env)?;
    if api >= 30 {
        env.call_method(
            &window,
            "setDecorFitsSystemWindows",
            "(Z)V",
            &[JValue::Bool((!playing).into())],
        )?;
        let controller = env
            .call_method(
                &window,
                "getInsetsController",
                "()Landroid/view/WindowInsetsController;",
                &[],
            )?
            .l()?;
        if !controller.is_null() {
            env.call_method(
                &controller,
                "setSystemBarsBehavior",
                "(I)V",
                // Match the reviewed public PSX path on API31+: DEFAULT keeps
                // system gestures active while bars are hidden. API30 retains
                // its transient-bars behavior. This changes only app policy;
                // the observed misplaced Home handle still needs a device test.
                &[JValue::Int(if api >= 31 { 1 } else { 2 })],
            )?;
            env.call_method(
                controller,
                if playing { "hide" } else { "show" },
                "(I)V",
                &[JValue::Int(7)],
            )?;
        }
    } else {
        // API28/29 supported legacy immersive-sticky path; layout flags keep
        // the taken surface stable while swipes temporarily reveal the bars.
        env.call_method(
            decor,
            "setSystemUiVisibility",
            "(I)V",
            &[JValue::Int(if playing { 5894 } else { 0 })],
        )?;
    }
    Ok(())
}
fn view_bounds(env: &mut JNIEnv<'_>, view: &JObject<'_>) -> Result<ntx::touch::Area> {
    let location = env.new_int_array(2)?;
    env.call_method(
        view,
        "getLocationOnScreen",
        "([I)V",
        &[JValue::Object(&location)],
    )?;
    let mut xy = [0; 2];
    env.get_int_array_region(&location, 0, &mut xy)?;
    let width = env.call_method(view, "getWidth", "()I", &[])?.i()?;
    let height = env.call_method(view, "getHeight", "()I", &[])?.i()?;
    Ok(ntx::touch::Area {
        left: xy[0] as f32,
        top: xy[1] as f32,
        right: (xy[0] + width) as f32,
        bottom: (xy[1] + height) as f32,
    })
}
fn measure(env: &mut JNIEnv<'_>, activity: &JObject<'_>) -> Result<ntx::viewport::WindowGeometry> {
    use ntx::{
        touch::Area,
        viewport::{Insets, WindowGeometry},
    };
    let version = sdk(env)?;
    let window = env
        .call_method(activity, "getWindow", "()Landroid/view/Window;", &[])?
        .l()?;
    let decor = env
        .call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?
        .l()?;
    let surface = view_bounds(env, &decor)?;
    let location = env.new_int_array(2)?;
    env.call_method(
        &decor,
        "getLocationInWindow",
        "([I)V",
        &[JValue::Object(&location)],
    )?;
    let mut xy = [0; 2];
    env.get_int_array_region(&location, 0, &mut xy)?;
    let content = env
        .call_method(
            activity,
            "findViewById",
            "(I)Landroid/view/View;",
            &[JValue::Int(16908290)],
        )?
        .l()?;
    if content.is_null() {
        return Err("NativeActivity content bounds unavailable".into());
    }
    let content = view_bounds(env, &content)?;
    let root_insets = env
        .call_method(
            &decor,
            "getRootWindowInsets",
            "()Landroid/view/WindowInsets;",
            &[],
        )?
        .l()?;
    if root_insets.is_null() {
        return Err("Window insets not ready".into());
    }
    let mut bounds = surface;
    let mut inset = Insets::default();
    if version >= 30 {
        let manager = env
            .call_method(
                activity,
                "getWindowManager",
                "()Landroid/view/WindowManager;",
                &[],
            )?
            .l()?;
        let metrics = env
            .call_method(
                manager,
                "getCurrentWindowMetrics",
                "()Landroid/view/WindowMetrics;",
                &[],
            )?
            .l()?;
        let rect = env
            .call_method(&metrics, "getBounds", "()Landroid/graphics/Rect;", &[])?
            .l()?;
        bounds = Area {
            left: env.get_field(&rect, "left", "I")?.i()? as f32,
            top: env.get_field(&rect, "top", "I")?.i()? as f32,
            right: env.get_field(&rect, "right", "I")?.i()? as f32,
            bottom: env.get_field(&rect, "bottom", "I")?.i()? as f32,
        };
        let insets = env
            .call_method(
                metrics,
                "getWindowInsets",
                "()Landroid/view/WindowInsets;",
                &[],
            )?
            .l()?;
        let stable = env
            .call_method(
                insets,
                "getInsetsIgnoringVisibility",
                "(I)Landroid/graphics/Insets;",
                &[JValue::Int(135)],
            )?
            .l()?;
        for (name, value) in [
            ("left", &mut inset.left),
            ("top", &mut inset.top),
            ("right", &mut inset.right),
            ("bottom", &mut inset.bottom),
        ] {
            *value = env.get_field(&stable, name, "I")?.i()? as f32;
        }
    } else {
        for (name, value) in [
            ("getStableInsetLeft", &mut inset.left),
            ("getStableInsetTop", &mut inset.top),
            ("getStableInsetRight", &mut inset.right),
            ("getStableInsetBottom", &mut inset.bottom),
        ] {
            *value = env.call_method(&root_insets, name, "()I", &[])?.i()? as f32;
        }
        let cutout = env
            .call_method(
                &root_insets,
                "getDisplayCutout",
                "()Landroid/view/DisplayCutout;",
                &[],
            )?
            .l()?;
        if !cutout.is_null() {
            for (name, value) in [
                ("getSafeInsetLeft", &mut inset.left),
                ("getSafeInsetTop", &mut inset.top),
                ("getSafeInsetRight", &mut inset.right),
                ("getSafeInsetBottom", &mut inset.bottom),
            ] {
                *value = value.max(env.call_method(&cutout, name, "()I", &[])?.i()? as f32);
            }
        }
    }
    if version >= 29 {
        let gestures = env
            .call_method(
                &root_insets,
                "getMandatorySystemGestureInsets",
                "()Landroid/graphics/Insets;",
                &[],
            )?
            .l()?;
        for (name, value) in [
            ("left", &mut inset.left),
            ("top", &mut inset.top),
            ("right", &mut inset.right),
            ("bottom", &mut inset.bottom),
        ] {
            *value = value.max(env.get_field(&gestures, name, "I")?.i()? as f32);
        }
    }
    let resources = env
        .call_method(
            activity,
            "getResources",
            "()Landroid/content/res/Resources;",
            &[],
        )?
        .l()?;
    let metrics = env
        .call_method(
            resources,
            "getDisplayMetrics",
            "()Landroid/util/DisplayMetrics;",
            &[],
        )?
        .l()?;
    let density = env.get_field(metrics, "density", "F")?.f()?;
    Ok(WindowGeometry {
        window: bounds,
        surface,
        content,
        input_origin: (surface.left - xy[0] as f32, surface.top - xy[1] as f32),
        clearance: inset,
        density,
    })
}
fn update_window(env: &mut JNIEnv<'_>, activity: &JObject<'_>) {
    trace_geometry("measure.begin");
    let result = env.with_local_frame(64, |env| measure(env, activity));
    trace_geometry(if result.is_ok() {
        "measure.end.ok"
    } else {
        "measure.end.error"
    });
    // Initial layout can legitimately lack insets. A later inset/layout callback
    // retries; until then no touch geometry is admitted.
    crate::player::measure_window(result.ok());
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
        diagnostic("Could not measure the current app window; input neutralized");
    }
}
pub fn refresh_window(app: &AndroidApp) {
    let epoch = EPOCH.load(Ordering::Acquire);
    app.run_on_java_main_thread(Box::new(move || {
        if EPOCH.load(Ordering::Acquire) != epoch {
            return;
        }
        let _ = with_env(|env, activity| {
            update_window(env, activity);
            Ok(())
        });
    }));
}
#[unsafe(no_mangle)]
extern "system" fn Java_com_machinerhythm_ntx_NtxActivity_windowChanged(
    mut env: JNIEnv<'_>,
    activity: JObject<'_>,
) {
    if current_activity(&env, &activity) {
        update_window(&mut env, &activity);
    }
}
#[unsafe(no_mangle)]
extern "system" fn Java_com_machinerhythm_ntx_NtxActivity_windowFocus(
    mut env: JNIEnv<'_>,
    activity: JObject<'_>,
    focused: jni::sys::jboolean,
) {
    if !current_activity(&env, &activity) {
        trace_current("focus.reject", "stale_activity");
        return;
    }
    trace_current("focus", if focused != 0 { "gained" } else { "lost" });
    if focused != 0 {
        if playing_window(&mut env, &activity, PLAYING.load(Ordering::Acquire)).is_err() {
            diagnostic("Could not apply app-local system-bar mode");
            let _ = env.exception_clear();
        }
        update_window(&mut env, &activity);
    } else {
        crate::player::measure_window(None);
    }
}
pub fn shutdown() {
    PLAYING.store(false, Ordering::Release);
    crate::player::measure_window(None);
    EPOCH.fetch_add(1, Ordering::AcqRel);
    if let Ok(mut menu) = MENU.lock() {
        *menu = None;
    }
    if let Ok(mut activity) = ACTIVITY.lock() {
        *activity = None;
    }
}
fn uri<'a>(env: &mut JNIEnv<'a>, text: &str) -> Result<JObject<'a>> {
    let value = env.new_string(text)?;
    Ok(env
        .call_static_method(
            "android/net/Uri",
            "parse",
            "(Ljava/lang/String;)Landroid/net/Uri;",
            &[JValue::Object(&value)],
        )?
        .l()?)
}
fn resolver<'a>(env: &mut JNIEnv<'a>, activity: &JObject<'_>) -> Result<JObject<'a>> {
    Ok(env
        .call_method(
            activity,
            "getContentResolver",
            "()Landroid/content/ContentResolver;",
            &[],
        )?
        .l()?)
}
pub fn open_document(text: &str) -> Result<File> {
    if !text.starts_with("content://") {
        return Err(
            "Choose the original game through this installation's document picker to grant access."
                .into(),
        );
    }
    with_env(|env, activity| {
        let uri = uri(env, text)?;
        let resolver = resolver(env, activity)?;
        let mode = env.new_string("r")?;
        let pfd = env
            .call_method(
                resolver,
                "openFileDescriptor",
                "(Landroid/net/Uri;Ljava/lang/String;)Landroid/os/ParcelFileDescriptor;",
                &[JValue::Object(&uri), JValue::Object(&mode)],
            )?
            .l()?;
        if pfd.is_null() {
            return Err("provider returned no file".into());
        }
        let fd = env.call_method(&pfd, "detachFd", "()I", &[])?.i()?;
        if fd < 0 {
            return Err("provider returned invalid descriptor".into());
        }
        // SAFETY: detachFd transfers exclusive ownership to us.
        let mut file = unsafe { File::from_raw_fd(fd) };
        file.seek(SeekFrom::Start(0))?;
        Ok(file)
    })
}
pub fn display_name(text: &str) -> Result<String> {
    with_env(|env, activity| {
        let uri = uri(env, text)?;
        let resolver = resolver(env, activity)?;
        let null = JObject::null();
        let cursor = env.call_method(resolver, "query", "(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;", &[JValue::Object(&uri), JValue::Object(&null), JValue::Object(&null), JValue::Object(&null), JValue::Object(&null)])?.l()?;
        if cursor.is_null() {
            return Err("provider metadata unavailable".into());
        }
        let result = (|| {
            if !env.call_method(&cursor, "moveToFirst", "()Z", &[])?.z()? {
                return Err("provider metadata empty".into());
            }
            let column = env.new_string("_display_name")?;
            let index = env
                .call_method(
                    &cursor,
                    "getColumnIndex",
                    "(Ljava/lang/String;)I",
                    &[JValue::Object(&column)],
                )?
                .i()?;
            if index < 0 {
                return Err("provider has no display name".into());
            }
            let value = JString::from(
                env.call_method(
                    &cursor,
                    "getString",
                    "(I)Ljava/lang/String;",
                    &[JValue::Int(index)],
                )?
                .l()?,
            );
            Ok(env.get_string(&value)?.into())
        })();
        let _ = env.call_method(cursor, "close", "()V", &[]);
        result
    })
}
pub fn picker(app: &AndroidApp, initial_uri: Option<&str>) {
    let initial_uri = initial_uri.map(str::to_owned);
    let epoch = EPOCH.load(Ordering::Acquire);
    app.run_on_java_main_thread(Box::new(move || {
        if EPOCH.load(Ordering::Acquire) != epoch {
            return;
        }
        let result = with_env(|env, activity| {
            PLAYING.store(false, Ordering::Release);
            playing_window(env, activity, false)?;
            let action = env.new_string("android.intent.action.OPEN_DOCUMENT")?;
            let intent = env.new_object(
                "android/content/Intent",
                "(Ljava/lang/String;)V",
                &[JValue::Object(&action)],
            )?;
            let category = env.new_string("android.intent.category.OPENABLE")?;
            env.call_method(
                &intent,
                "addCategory",
                "(Ljava/lang/String;)Landroid/content/Intent;",
                &[JValue::Object(&category)],
            )?;
            let mime = env.new_string("*/*")?;
            env.call_method(
                &intent,
                "setType",
                "(Ljava/lang/String;)Landroid/content/Intent;",
                &[JValue::Object(&mime)],
            )?;
            env.call_method(
                &intent,
                "addFlags",
                "(I)Landroid/content/Intent;",
                &[JValue::Int(1 | 64)],
            )?;
            if let Some(initial) = &initial_uri {
                let key = env.new_string("android.provider.extra.INITIAL_URI")?;
                let initial = uri(env, initial)?;
                env.call_method(
                    &intent,
                    "putExtra",
                    "(Ljava/lang/String;Landroid/os/Parcelable;)Landroid/content/Intent;",
                    &[JValue::Object(&key), JValue::Object(&initial)],
                )?;
            }
            env.call_method(
                activity,
                "startActivityForResult",
                "(Landroid/content/Intent;I)V",
                &[JValue::Object(&intent), JValue::Int(42)],
            )?;
            Ok(())
        });
        if result.is_err() {
            push(Command::Error("Could not open the document picker.".into()));
        }
    }));
}
pub fn menu(app: &AndroidApp, show: bool, message: Option<String>) {
    let recovery = message.is_some();
    let mut actions = Vec::new();
    if recovery {
        actions.push((2, "Restart with current saves".to_owned()));
    }
    actions.push((6, "Library".to_owned()));
    actions.push((1, "Add game".to_owned()));
    actions.push((8, "Touch controls".to_owned()));
    actions.push((9, "About ntx".to_owned()));
    render_menu(
        app,
        show,
        message.unwrap_or_else(|| {
            "Your games\nAdd a GameCube or N64 game. Your progress stays with it.".into()
        }),
        actions,
        0,
    );
}
pub fn relink_menu(app: &AndroidApp, title: &str) {
    render_menu(
        app,
        true,
        format!(
            "{}\nChoose this game's file to restore access. Your progress and retained scene stay with it.",
            ntx::presentation::game_title(title)
        ),
        vec![(12, "Choose game file".into()), (6, "Library".into())],
        0,
    );
}
pub fn confirm_legacy_media_menu(app: &AndroidApp, title: &str, filename: &str) {
    render_menu(
        app,
        true,
        format!(
            "{}\nIs {} the original file for this game? This older entry cannot verify a replacement automatically. Confirming keeps its in-game saves and starts with current progress.",
            ntx::presentation::game_title(title),
            ntx::presentation::game_title(filename)
        ),
        vec![
            (17, "Use this game file".into()),
            (12, "Choose another file".into()),
            (6, "Library".into()),
        ],
        0,
    );
}
pub fn unloaded_menu(app: &AndroidApp, title: &str, error: Option<String>) {
    let recovery = error.is_some();
    let text = error.unwrap_or_else(|| {
        format!(
            "{}\nGame stopped · in-game saves kept",
            ntx::presentation::game_title(title)
        )
    });
    render_menu(
        app,
        true,
        text,
        vec![
            (
                if recovery { 2 } else { 5 },
                if recovery {
                    "Restart with current saves".into()
                } else {
                    "Play".into()
                },
            ),
            (7, "Add game to Home screen".into()),
            (6, "Library".into()),
            (1, "Add game".into()),
            (8, "Touch controls".into()),
        ],
        0,
    );
}
pub fn player_menu(app: &AndroidApp, runtime: u64, can_resume: bool, error: Option<String>) {
    let mut text = error
        .map(|s| format!("Paused\n\n{s}"))
        .unwrap_or_else(|| "Paused".into());
    text.push_str("\nYour place is kept while paused. Stop game discards this resume point; save in-game to keep progress.");
    let mut actions = Vec::new();
    if can_resume {
        actions.push((3, "Resume".into()));
    } else {
        actions.push((2, "Restart with current saves".into()));
    }
    actions.push((4, "Stop game".into()));
    actions.push((7, "Add game to Home screen".into()));
    actions.push((6, "Library".into()));
    actions.push((8, "Touch controls".into()));
    render_menu(app, true, text, actions, runtime);
}
pub fn menu_current(generation: u64) -> bool {
    MENU_GENERATION.load(Ordering::Acquire) == generation
}
// At most one acknowledgment per outcome per Activity. Android accepting a
// request is distinct from the owner feeling a vibration; never override its
// system feedback preference or add work to native pad polling.
static HAPTIC_OUTCOMES: AtomicU64 = AtomicU64::new(0);
fn feedback(env: &mut JNIEnv<'_>, activity: &JObject<'_>) {
    if !crate::player::haptics_enabled() {
        return;
    }
    let result: Result<(u64, &str)> = (|| {
        // A modal menu owns its own focused window. The Activity decor loses
        // focus while that Dialog is up, including a queued Pause contact.
        let dialog = MENU.lock().map_err(|_| "menu lock poisoned")?.clone();
        let owner = dialog.as_ref().map_or(activity, |d| d.as_obj());
        let window = env
            .call_method(owner, "getWindow", "()Landroid/view/Window;", &[])?
            .l()?;
        let view = env
            .call_method(window, "getDecorView", "()Landroid/view/View;", &[])?
            .l()?;
        if !env.call_method(&view, "hasWindowFocus", "()Z", &[])?.z()? {
            return Ok((4, "no_window_focus"));
        }
        // VIRTUAL_KEY respects Android's haptic setting and capability.
        let accepted = env
            .call_method(view, "performHapticFeedback", "(I)Z", &[JValue::Int(1)])?
            .z()?;
        Ok(if accepted {
            (1, "accepted")
        } else {
            (2, "declined")
        })
    })();
    let (flag, detail) = result.unwrap_or_else(|_| {
        let _ = env.exception_clear();
        (8, "jni_error")
    });
    if HAPTIC_OUTCOMES.fetch_or(flag, Ordering::Relaxed) & flag == 0 {
        trace_current("haptic.result", detail);
    }
}
pub fn haptic(app: &AndroidApp) {
    let epoch = EPOCH.load(Ordering::Acquire);
    app.run_on_java_main_thread(Box::new(move || {
        if EPOCH.load(Ordering::Acquire) != epoch {
            return;
        }
        let _ = with_env(|env, activity| {
            feedback(env, activity);
            Ok(())
        });
    }));
}
fn menu_caption(
    env: &mut JNIEnv<'_>,
    activity: &JObject<'_>,
    parent: &JObject<'_>,
    text: &str,
    size: f32,
    color: u32,
) -> Result<()> {
    let view = env.new_object(
        "android/widget/TextView",
        "(Landroid/content/Context;)V",
        &[JValue::Object(activity)],
    )?;
    let text = env.new_string(text)?;
    env.call_method(
        &view,
        "setText",
        "(Ljava/lang/CharSequence;)V",
        &[JValue::Object(&text)],
    )?;
    env.call_method(&view, "setTextSize", "(F)V", &[JValue::Float(size)])?;
    env.call_method(&view, "setTextColor", "(I)V", &[JValue::Int(color as i32)])?;
    env.call_method(
        parent,
        "addView",
        "(Landroid/view/View;)V",
        &[JValue::Object(&view)],
    )?;
    Ok(())
}
fn menu_background(
    env: &mut JNIEnv<'_>,
    view: &JObject<'_>,
    color: u32,
    density: f32,
) -> Result<()> {
    let shape = env.new_object("android/graphics/drawable/GradientDrawable", "()V", &[])?;
    env.call_method(&shape, "setColor", "(I)V", &[JValue::Int(color as i32)])?;
    env.call_method(
        &shape,
        "setCornerRadius",
        "(F)V",
        &[JValue::Float(16. * density)],
    )?;
    let colors = env
        .call_static_method(
            "android/content/res/ColorStateList",
            "valueOf",
            "(I)Landroid/content/res/ColorStateList;",
            &[JValue::Int(0x287fba87)],
        )?
        .l()?;
    let ripple=env.new_object("android/graphics/drawable/RippleDrawable","(Landroid/content/res/ColorStateList;Landroid/graphics/drawable/Drawable;Landroid/graphics/drawable/Drawable;)V",&[JValue::Object(&colors),JValue::Object(&shape),JValue::Object(&JObject::null())])?;
    env.call_method(
        view,
        "setBackground",
        "(Landroid/graphics/drawable/Drawable;)V",
        &[JValue::Object(&ripple)],
    )?;
    Ok(())
}
fn menu_game_label<'a>(env: &mut JNIEnv<'a>, text: &str) -> Result<JObject<'a>> {
    let value = env.new_string(text)?;
    let rich = env.new_object(
        "android/text/SpannableString",
        "(Ljava/lang/CharSequence;)V",
        &[JValue::Object(&value)],
    )?;
    if let Some((title, _)) = text.split_once('\n') {
        let end = title.encode_utf16().count() as i32;
        let length = text.encode_utf16().count() as i32;
        let weight = env.new_object("android/text/style/StyleSpan", "(I)V", &[JValue::Int(1)])?;
        let color = env.new_object(
            "android/text/style/ForegroundColorSpan",
            "(I)V",
            &[JValue::Int(ntx::presentation::MUTED as i32)],
        )?;
        let size = env.new_object(
            "android/text/style/RelativeSizeSpan",
            "(F)V",
            &[JValue::Float(0.875)],
        )?;
        for (span, start, finish) in [
            (&weight, 0, end),
            (&color, end + 1, length),
            (&size, end + 1, length),
        ] {
            env.call_method(
                &rich,
                "setSpan",
                "(Ljava/lang/Object;III)V",
                &[
                    JValue::Object(span),
                    JValue::Int(start),
                    JValue::Int(finish),
                    JValue::Int(33),
                ],
            )?;
        }
    }
    Ok(rich)
}
fn render_menu(
    app: &AndroidApp,
    show: bool,
    text: String,
    actions: Vec<(i32, String)>,
    runtime: u64,
) {
    render_menu_with_games(app, show, text, actions, runtime, Vec::new(), None);
}
pub fn library_menu(
    app: &AndroidApp,
    games: Vec<(String, String)>,
    runtime: Option<u64>,
    can_resume: bool,
) {
    let mut actions = Vec::new();
    actions.push((8, "Touch controls".into()));
    actions.push((9, "About ntx".into()));
    if let Some(epoch) = runtime {
        if can_resume {
            actions.push((3, "Resume current game".into()));
        }
        actions.push((4, "Stop current game".into()));
        actions.push((1, "Add game".into()));
        render_menu_with_games(
            app,
            true,
            "Your games\nPick up where you left off.".into(),
            actions,
            epoch,
            games,
            None,
        );
    } else {
        actions.push((1, "Add game".into()));
        let text = if games.is_empty() {
            "Your games\nAdd a GameCube or N64 game. Your progress stays with it."
        } else {
            "Your games\nPick up where you left off."
        };
        render_menu_with_games(app, true, text.into(), actions, 0, games, None);
    }
}
pub fn controls_menu(
    app: &AndroidApp,
    runtime: u64,
    width: f32,
    height: f32,
    settings: ntx::controls::Settings,
) {
    controls_ui::show(
        app,
        runtime,
        ntx::controls::Editor::new(width, height, settings),
    );
}
pub fn about_menu(app: &AndroidApp) {
    render_menu(app,true,"ntx\nAlpha 1 · GameCube + N64\n\nYour games. Open, play, leave, return.\nMade by Machine Rhythm.\n\nBuilt with Dolphin and Mupen64Plus-Next. Original ntx work is MIT. The combined package follows applicable GPLv3 and component terms; no warranty.\nSource and notices: github.com/WigglyDon/ntx".into(),vec![(6,"Back to your games".into())],0);
}
pub fn n64_controls_menu(app: &AndroidApp, runtime: u64, settings: &ntx::controls::Settings) {
    render_menu(
        app,
        true,
        format!(
            "N64 touch controls\n\nOne analog stick, four C buttons, digital L/R and Z.\nVisibility: {}% · Stick dead zone: {}%",
            settings.opacity, settings.dead_zone
        ),
        vec![
            (13, "Less visible controls".into()),
            (14, "More visible controls".into()),
            (
                15,
                if settings.haptics {
                    "Turn touch feedback off".into()
                } else {
                    "Turn touch feedback on".into()
                },
            ),
            (16, "Change stick dead zone".into()),
            (11, "Back to your games".into()),
        ],
        runtime,
    );
}
fn render_menu_with_games(
    app: &AndroidApp,
    show: bool,
    text: String,
    actions: Vec<(i32, String)>,
    runtime: u64,
    games: Vec<(String, String)>,
    controls: Option<ntx::controls::Editor>,
) {
    let epoch = EPOCH.load(Ordering::Acquire);
    let generation = MENU_GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    let editing = controls.is_some();
    controls_ui::bind(controls, generation, epoch);
    trace(
        runtime,
        generation,
        "menu.post",
        if show { "show" } else { "hide" },
    );
    app.run_on_java_main_thread(Box::new(move || {
        if EPOCH.load(Ordering::Acquire) != epoch || !menu_current(generation) {
            trace(
                runtime,
                generation,
                "menu.reject",
                &format!("posted_activity={epoch} stale_activity_or_menu"),
            );
            return;
        }
        trace(runtime, generation, "menu.execute", "");
        let result = with_env(|env, activity| {
            PLAYING.store(!show, Ordering::Release);
            trace(
                runtime,
                generation,
                "window.policy.begin",
                if show { "menu" } else { "playing" },
            );
            playing_window(env, activity, !show)?;
            trace(runtime, generation, "window.policy.end", "ok");
            let old = MENU.lock().map_err(|_| "menu lock poisoned")?.take();
            if let Some(dialog) = old {
                env.call_method(dialog.as_obj(), "dismiss", "()V", &[])?;
            }
            if show {
                trace(runtime, generation, "dialog.build", "");
                let view = env.new_object(
                    "android/widget/LinearLayout",
                    "(Landroid/content/Context;)V",
                    &[JValue::Object(activity)],
                )?;
                env.call_method(&view, "setOrientation", "(I)V", &[JValue::Int(1)])?;
                env.call_method(&view, "setGravity", "(I)V", &[JValue::Int(49)])?;
                let resources = env
                    .call_method(
                        activity,
                        "getResources",
                        "()Landroid/content/res/Resources;",
                        &[],
                    )?
                    .l()?;
                let metrics = env
                    .call_method(
                        &resources,
                        "getDisplayMetrics",
                        "()Landroid/util/DisplayMetrics;",
                        &[],
                    )?
                    .l()?;
                let density = env
                    .get_field(&metrics, "density", "F")?
                    .f()?
                    .clamp(0.5, 8.0);
                let dp = |value: f32| (value * density).round() as i32;
                env.call_method(
                    &view,
                    "setPadding",
                    "(IIII)V",
                    &[
                        JValue::Int(dp(16.0)),
                        JValue::Int(dp(12.0)),
                        JValue::Int(dp(16.0)),
                        JValue::Int(dp(12.0)),
                    ],
                )?;
                let label = env.new_object(
                    "android/widget/TextView",
                    "(Landroid/content/Context;)V",
                    &[JValue::Object(activity)],
                )?;
                menu_caption(
                    env,
                    activity,
                    &view,
                    "ntx   /   ALPHA 1",
                    if runtime != 0 && !editing { 18.0 } else { 30.0 },
                    ntx::presentation::TEXT,
                )?;
                menu_caption(
                    env,
                    activity,
                    &view,
                    "Machine Rhythm · Nintendo",
                    13.0,
                    ntx::presentation::MUTED,
                )?;
                let (heading, detail) = text.split_once('\n').unwrap_or((&text, ""));
                let text = env.new_string(heading)?;
                env.call_method(
                    &label,
                    "setText",
                    "(Ljava/lang/CharSequence;)V",
                    &[JValue::Object(&text)],
                )?;
                env.call_method(&label, "setTextSize", "(F)V", &[JValue::Float(23.0)])?;
                env.call_method(&label, "setGravity", "(I)V", &[JValue::Int(3)])?;
                env.call_method(
                    &label,
                    "setPadding",
                    "(IIII)V",
                    &[
                        JValue::Int(dp(4.0)),
                        JValue::Int(dp(8.0)),
                        JValue::Int(dp(4.0)),
                        JValue::Int(dp(20.0)),
                    ],
                )?;
                env.call_method(&label, "setTextColor", "(I)V", &[JValue::Int(-1)])?;
                env.call_method(
                    &view,
                    "addView",
                    "(Landroid/view/View;)V",
                    &[JValue::Object(&label)],
                )?;
                env.call_method(
                    &view,
                    "setBackgroundColor",
                    "(I)V",
                    &[JValue::Int(ntx::presentation::BACKGROUND as i32)],
                )?;
                if !detail.is_empty() {
                    menu_caption(
                        env,
                        activity,
                        &view,
                        detail.trim(),
                        14.0,
                        ntx::presentation::MUTED,
                    )?;
                }
                if editing {
                    controls_ui::append(env, activity, &view, density, generation)?;
                }
                let stop_available = actions.iter().any(|(action, _)| *action == 4);
                let fixed_footer = editing || stop_available;
                let footer = if fixed_footer {
                    let row = env.new_object(
                        "android/widget/LinearLayout",
                        "(Landroid/content/Context;)V",
                        &[JValue::Object(activity)],
                    )?;
                    env.call_method(&row, "setOrientation", "(I)V", &[JValue::Int(0)])?;
                    row
                } else {
                    JObject::null()
                };
                // Public PSX's game-first column: entries precede auxiliary actions.
                let first_add = games.is_empty()
                    && runtime == 0
                    && !actions.iter().any(|(a, _)| matches!(a, 3 | 5 | 10));
                let mut actions = actions;
                if !editing {
                    actions.sort_by_key(|(a, _)| match a {
                        3 | 5 => 0,
                        1 => 1,
                        6 => 2,
                        8 => 3,
                        9 => 4,
                        7 => 5,
                        4 => 9,
                        _ => 6,
                    });
                }
                let rows = games
                    .into_iter()
                    .map(|(game, title)| (None, Some(game), title))
                    .chain(
                        actions
                            .into_iter()
                            .map(|(action, text)| (Some(action), None, text)),
                    );
                for (action, game, text) in rows {
                    // Pop temporary JNI refs for each row; large collections do
                    // not grow an attached Java thread's local reference table.
                    env.with_local_frame(24, |env| -> Result<()> {
                        let button = env.new_object(
                            "android/widget/Button",
                            "(Landroid/content/Context;)V",
                            &[JValue::Object(activity)],
                        )?;
                        env.call_method(&button, "setAllCaps", "(Z)V", &[JValue::Bool(0)])?;
                        env.call_method(&button, "setTextSize", "(F)V", &[JValue::Float(16.0)])?;
                        env.call_method(&button, "setMinHeight", "(I)V", &[JValue::Int(dp(56.0))])?;
                        let primary =
                            matches!(action, Some(3 | 5 | 10)) || (first_add && action == Some(1));
                        let color = if primary {
                            ntx::presentation::ACCENT
                        } else {
                            ntx::presentation::SURFACE
                        };
                        let ink = if primary {
                            ntx::presentation::INK
                        } else if action == Some(4) {
                            ntx::presentation::DANGER
                        } else {
                            ntx::presentation::TEXT
                        };
                        env.call_method(
                            &button,
                            "setTextColor",
                            "(I)V",
                            &[JValue::Int(ink as i32)],
                        )?;
                        env.call_method(&button, "setGravity", "(I)V", &[JValue::Int(3 | 16)])?;
                        env.call_method(
                            &button,
                            "setPadding",
                            "(IIII)V",
                            &[
                                JValue::Int(dp(20.)),
                                JValue::Int(dp(12.)),
                                JValue::Int(dp(20.)),
                                JValue::Int(dp(12.)),
                            ],
                        )?;
                        let tag = env.new_string(format!("ntx-action-{}", action.unwrap_or(0)))?;
                        env.call_method(
                            &button,
                            "setTag",
                            "(Ljava/lang/Object;)V",
                            &[JValue::Object(&tag)],
                        )?;
                        menu_background(env, &button, color, density)?;
                        let footer_action = action.is_some_and(|action| {
                            ntx::presentation::menu_footer_action(editing, stop_available, action)
                        });
                        let params = if footer_action {
                            env.new_object(
                                "android/widget/LinearLayout$LayoutParams",
                                "(IIF)V",
                                &[JValue::Int(0), JValue::Int(-2), JValue::Float(1.0)],
                            )?
                        } else {
                            env.new_object(
                                "android/widget/LinearLayout$LayoutParams",
                                "(II)V",
                                &[JValue::Int(-1), JValue::Int(-2)],
                            )?
                        };
                        env.call_method(
                            &params,
                            "setMargins",
                            "(IIII)V",
                            &[
                                JValue::Int(if footer_action { dp(4.) } else { 0 }),
                                JValue::Int(dp(5.)),
                                JValue::Int(if footer_action { dp(4.) } else { 0 }),
                                JValue::Int(dp(5.)),
                            ],
                        )?;
                        env.call_method(
                            &button,
                            "setLayoutParams",
                            "(Landroid/view/ViewGroup$LayoutParams;)V",
                            &[JValue::Object(&params)],
                        )?;
                        if game.is_some() {
                            env.call_method(
                                &button,
                                "setMinHeight",
                                "(I)V",
                                &[JValue::Int(dp(84.))],
                            )?;
                        }
                        let text: JObject = if game.is_some() {
                            menu_game_label(env, &text)?
                        } else {
                            env.new_string(text)?.into()
                        };
                        env.call_method(
                            &button,
                            "setText",
                            "(Ljava/lang/CharSequence;)V",
                            &[JValue::Object(&text)],
                        )?;
                        let listener = if let Some(game) = game {
                            let class = activity_class(
                                env,
                                activity,
                                "com.machinerhythm.ntx.NtxActivity$GameAction",
                            )?;
                            let game = env.new_string(game)?;
                            env.new_object(
                                class,
                                "(Lcom/machinerhythm/ntx/NtxActivity;Ljava/lang/String;J)V",
                                &[
                                    JValue::Object(activity),
                                    JValue::Object(&game),
                                    JValue::Long(generation as i64),
                                ],
                            )?
                        } else {
                            let class = activity_class(
                                env,
                                activity,
                                "com.machinerhythm.ntx.NtxActivity$Action",
                            )?;
                            env.new_object(
                                class,
                                "(Lcom/machinerhythm/ntx/NtxActivity;IJJ)V",
                                &[
                                    JValue::Object(activity),
                                    JValue::Int(action.ok_or("missing menu action")?),
                                    JValue::Long(generation as i64),
                                    JValue::Long(runtime as i64),
                                ],
                            )?
                        };
                        env.call_method(
                            &button,
                            "setOnClickListener",
                            "(Landroid/view/View$OnClickListener;)V",
                            &[JValue::Object(&listener)],
                        )?;
                        env.call_method(
                            if footer_action { &footer } else { &view },
                            "addView",
                            "(Landroid/view/View;)V",
                            &[JValue::Object(&button)],
                        )?;
                        Ok(())
                    })?;
                }
                let scroll = env.new_object(
                    "android/widget/ScrollView",
                    "(Landroid/content/Context;)V",
                    &[JValue::Object(activity)],
                )?;
                env.call_method(&scroll, "setFillViewport", "(Z)V", &[JValue::Bool(1)])?;
                env.call_method(
                    &scroll,
                    "setBackgroundColor",
                    "(I)V",
                    &[JValue::Int(ntx::presentation::BACKGROUND as i32)],
                )?;
                let parent_window = env
                    .call_method(activity, "getWindow", "()Landroid/view/Window;", &[])?
                    .l()?;
                let decor = env
                    .call_method(&parent_window, "getDecorView", "()Landroid/view/View;", &[])?
                    .l()?;
                let measured = env.call_method(&decor, "getWidth", "()I", &[])?.i()?;
                let width = if measured > 0 {
                    measured
                } else {
                    env.get_field(&metrics, "widthPixels", "I")?.i()?
                };
                // Adapt PSX's centered720-point column. Insets can make the
                // framework parent narrower; padding keeps children inside it.
                let side = dp(16.0).max((width - dp(720.0)) / 2);
                env.call_method(
                    &view,
                    "setPadding",
                    "(IIII)V",
                    &[
                        JValue::Int(side),
                        JValue::Int(dp(12.0)),
                        JValue::Int(side),
                        JValue::Int(dp(12.0)),
                    ],
                )?;
                env.call_method(
                    &scroll,
                    "addView",
                    "(Landroid/view/View;)V",
                    &[JValue::Object(&view)],
                )?;
                let content = if fixed_footer {
                    let content = env.new_object(
                        "android/widget/LinearLayout",
                        "(Landroid/content/Context;)V",
                        &[JValue::Object(activity)],
                    )?;
                    env.call_method(&content, "setOrientation", "(I)V", &[JValue::Int(1)])?;
                    env.call_method(
                        &content,
                        "setBackgroundColor",
                        "(I)V",
                        &[JValue::Int(ntx::presentation::BACKGROUND as i32)],
                    )?;
                    let params = env.new_object(
                        "android/widget/LinearLayout$LayoutParams",
                        "(IIF)V",
                        &[JValue::Int(-1), JValue::Int(0), JValue::Float(1.0)],
                    )?;
                    env.call_method(
                        &content,
                        "addView",
                        "(Landroid/view/View;Landroid/view/ViewGroup$LayoutParams;)V",
                        &[JValue::Object(&scroll), JValue::Object(&params)],
                    )?;
                    env.call_method(
                        &footer,
                        "setPadding",
                        "(IIII)V",
                        &[
                            JValue::Int(side),
                            JValue::Int(dp(4.0)),
                            JValue::Int(side),
                            JValue::Int(dp(8.0)),
                        ],
                    )?;
                    env.call_method(
                        &content,
                        "addView",
                        "(Landroid/view/View;)V",
                        &[JValue::Object(&footer)],
                    )?;
                    content
                } else {
                    scroll
                };
                // NativeActivity owns the root drawing surface. Views added to
                // that hierarchy can receive input while remaining unpainted.
                // This app-owned dialog has a framework-rendered window; the
                // Rust-built content and all actions/policy remain here.
                let dialog = env.new_object(
                    "android/app/Dialog",
                    "(Landroid/content/Context;)V",
                    &[JValue::Object(activity)],
                )?;
                // This Rust-built menu supplies its own heading. A framework
                // title reserves unused content space on default dialog themes.
                env.call_method(&dialog, "requestWindowFeature", "(I)Z", &[JValue::Int(1)])?;
                env.call_method(&dialog, "setCancelable", "(Z)V", &[JValue::Bool(0)])?;
                env.call_method(
                    &dialog,
                    "setContentView",
                    "(Landroid/view/View;)V",
                    &[JValue::Object(&content)],
                )?;
                trace(runtime, generation, "dialog.show.begin", "");
                env.call_method(&dialog, "show", "()V", &[])?;
                trace(runtime, generation, "dialog.show.end", "ok");
                let window = env
                    .call_method(&dialog, "getWindow", "()Landroid/view/Window;", &[])?
                    .l()?;
                env.call_method(
                    window,
                    "setLayout",
                    "(II)V",
                    &[JValue::Int(-1), JValue::Int(-1)],
                )?;
                *MENU.lock().map_err(|_| "menu lock poisoned")? = Some(env.new_global_ref(dialog)?);
                if editing {
                    controls_ui::refresh(env, generation)?;
                }
                trace(runtime, generation, "dialog.ready", "framework_window");
            }
            update_window(env, activity);
            Ok(())
        });
        if result.is_err() {
            trace(runtime, generation, "menu.error", "jni_or_window");
            if editing {
                // An editor-view failure is not evidence of a broken machine
                // checkpoint. Return to the ordinary library without changing
                // game recovery flags or applying the draft.
                push(Command::Notice(
                    "Could not open touch controls. Your game and saved controls are kept.".into(),
                ));
                push(Command::Library);
            } else {
                push(Command::Error(
                    "Could not prepare the player surface.".into(),
                ));
            }
        } else if !show {
            trace(runtime, generation, "menu.hidden.post", "");
            push(Command::MenuHidden(generation));
        }
    }));
}

#[unsafe(no_mangle)]
extern "system" fn Java_com_machinerhythm_ntx_NtxActivity_beforeDestroy(
    mut env: JNIEnv<'_>,
    activity: JObject<'_>,
) {
    if !current_activity(&env, &activity) {
        return;
    }
    EPOCH.fetch_add(1, Ordering::AcqRel);
    // Retire the binding before dismissing: reentrant and late window/focus
    // callbacks from this Activity must not publish geometry or hide bars.
    PLAYING.store(false, Ordering::Release);
    crate::player::measure_window(None);
    if let Ok(mut bound) = ACTIVITY.lock() {
        *bound = None;
    }
    let dialog = MENU.lock().ok().and_then(|mut menu| menu.take());
    if let Some(dialog) = dialog
        && let Err(error) = env.call_method(dialog.as_obj(), "dismiss", "()V", &[])
    {
        diagnostic(&format!("Dialog retirement failed: {error}"));
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
}

#[unsafe(no_mangle)]
extern "system" fn Java_com_machinerhythm_ntx_NtxActivity_uiAction(
    mut env: JNIEnv<'_>,
    activity: JObject<'_>,
    action: jint,
    menu: jlong,
    runtime: jlong,
) {
    if !current_activity(&env, &activity) || !menu_current(menu as u64) {
        trace(
            runtime as u64,
            menu as u64,
            "action.reject",
            "stale_activity_or_menu",
        );
        return;
    }
    trace(
        runtime as u64,
        menu as u64,
        "action.admit",
        &format!("action={action}"),
    );
    match action {
        1 | 12 => push(Command::Choose),
        2 => push(Command::Restart),
        3 => push(Command::Resume(runtime as u64)),
        4 => push(Command::Stop(runtime as u64)),
        5 => push(Command::Open),
        6 => push(Command::Library),
        7 => push(Command::PinCurrent),
        8 => {
            // UI hierarchy reads stay on Android's main thread. Dialog focus
            // neutralizes gameplay geometry, so never infer editor dimensions
            // from a retained/missing presentation on the emulation owner.
            match measure(&mut env, &activity)
                .ok()
                .and_then(|g| g.control_size())
            {
                Some((w, h)) => push(Command::Controls(menu as u64, w, h)),
                None => {
                    let _ = env.exception_clear();
                    push(Command::Notice(
                        "The display is changing. Try Touch controls again when it settles.".into(),
                    ));
                }
            }
        }
        9 => push(Command::About(menu as u64)),
        10 => {
            let actual = measure(&mut env, &activity)
                .ok()
                .and_then(|g| g.control_size())
                .map(|(w, h)| ntx::controls::display_key(w, h));
            if actual.is_none() || actual != controls_ui::display(menu as u64) {
                let _ = env.exception_clear();
                push(Command::Notice("The screen changed while editing. Your draft is retained; cancel and reopen Touch controls for the current screen.".into()));
                return;
            }
            if let Some(settings) = controls_ui::applied(menu as u64) {
                push(Command::ApplyControls(menu as u64, settings));
            } else {
                push(Command::Notice("Could not save this layout. The previous controls are retained. Reset this screen’s layout or cancel to continue.".into()));
            }
        }
        11 => push(Command::ControlsBack(menu as u64)),
        13..=16 => push(Command::N64Comfort(menu as u64, action)),
        17 => push(Command::ConfirmLegacyMedia(menu as u64)),
        20..=25 if controls_ui::action(&mut env, action, menu as u64).is_err() => {
            let _ = env.exception_clear();
        }
        _ => {}
    }
    if matches!(action, 1..=17 | 20..=25) {
        feedback(&mut env, &activity);
    }
}
#[unsafe(no_mangle)]
extern "system" fn Java_com_machinerhythm_ntx_NtxActivity_gameAction(
    mut env: JNIEnv<'_>,
    activity: JObject<'_>,
    game: JString<'_>,
    menu: jlong,
) {
    if !current_activity(&env, &activity) || !menu_current(menu as u64) {
        return;
    }
    let id = env.get_string(&game).map(String::from);
    match id {
        Ok(id) if ntx::library::valid_id(&id) => {
            feedback(&mut env, &activity);
            push(Command::Game(id));
        }
        _ => {
            let _ = env.exception_clear();
            push(Command::Notice("Invalid library game.".into()));
        }
    }
}
fn intent_game(env: &mut JNIEnv<'_>, intent: &JObject<'_>) -> Result<Option<String>> {
    if intent.is_null() {
        return Ok(None);
    }
    let data = env
        .call_method(intent, "getDataString", "()Ljava/lang/String;", &[])?
        .l()?;
    if data.is_null() {
        return Ok(None);
    }
    let data = JString::from(data);
    let uri: String = env.get_string(&data)?.into();
    Ok(Some(ntx::library::parse_play_uri(&uri)?))
}
pub fn initial_game_request() -> Result<Option<String>> {
    with_env(|env, activity| {
        let intent = env
            .call_method(activity, "getIntent", "()Landroid/content/Intent;", &[])?
            .l()?;
        intent_game(env, &intent)
    })
}
#[unsafe(no_mangle)]
extern "system" fn Java_com_machinerhythm_ntx_NtxActivity_launchIntent(
    mut env: JNIEnv<'_>,
    activity: JObject<'_>,
    intent: JObject<'_>,
) {
    if !current_activity(&env, &activity) {
        return;
    }
    match intent_game(&mut env, &intent) {
        Ok(Some(id)) => push(Command::Game(id)),
        Ok(None) => {}
        Err(_) => {
            let _ = env.exception_clear();
            push(Command::Notice(
                "Invalid ntx game link. Open a game from the library.".into(),
            ));
        }
    }
}
fn shortcut_manager<'a>(env: &mut JNIEnv<'a>, activity: &JObject<'_>) -> Result<JObject<'a>> {
    let name = env.new_string("shortcut")?;
    let manager = env
        .call_method(
            activity,
            "getSystemService",
            "(Ljava/lang/String;)Ljava/lang/Object;",
            &[JValue::Object(&name)],
        )?
        .l()?;
    if manager.is_null() {
        return Err("Home screen shortcuts are unavailable.".into());
    }
    Ok(manager)
}
/// Called on the Rust owner thread, not the Java UI thread. No service or legacy
/// INSTALL_SHORTCUT broadcast; the launcher owns its confirmation dialog.
pub fn pin_game(id: &str, title: &str) -> Result<bool> {
    let link = ntx::library::play_uri(id)?;
    let shortcut_id = format!("ntx-{id}");
    let title: String = title.chars().filter(|c| !c.is_control()).take(64).collect();
    let title = if title.trim().is_empty() {
        "GameCube game"
    } else {
        title.trim()
    };
    with_env(|env, activity| {
        let manager = shortcut_manager(env, activity)?;
        if !env
            .call_method(&manager, "isRequestPinShortcutSupported", "()Z", &[])?
            .z()?
        {
            return Ok(false);
        }
        let action = env.new_string("android.intent.action.VIEW")?;
        let intent = env.new_object(
            "android/content/Intent",
            "(Ljava/lang/String;)V",
            &[JValue::Object(&action)],
        )?;
        let link = uri(env, &link)?;
        env.call_method(
            &intent,
            "setData",
            "(Landroid/net/Uri;)Landroid/content/Intent;",
            &[JValue::Object(&link)],
        )?;
        let class = env
            .call_method(activity, "getClass", "()Ljava/lang/Class;", &[])?
            .l()?;
        env.call_method(
            &intent,
            "setClass",
            "(Landroid/content/Context;Ljava/lang/Class;)Landroid/content/Intent;",
            &[JValue::Object(activity), JValue::Object(&class)],
        )?;
        let id = env.new_string(&shortcut_id)?;
        let builder = env.new_object(
            "android/content/pm/ShortcutInfo$Builder",
            "(Landroid/content/Context;Ljava/lang/String;)V",
            &[JValue::Object(activity), JValue::Object(&id)],
        )?;
        let label = env.new_string(ntx::presentation::game_title(title))?;
        env.call_method(
            &builder,
            "setShortLabel",
            "(Ljava/lang/CharSequence;)Landroid/content/pm/ShortcutInfo$Builder;",
            &[JValue::Object(&label)],
        )?;
        env.call_method(
            &builder,
            "setIntent",
            "(Landroid/content/Intent;)Landroid/content/pm/ShortcutInfo$Builder;",
            &[JValue::Object(&intent)],
        )?;
        let info = env
            .call_method(
                &builder,
                "build",
                "()Landroid/content/pm/ShortcutInfo;",
                &[],
            )?
            .l()?;
        Ok(env
            .call_method(
                &manager,
                "requestPinShortcut",
                "(Landroid/content/pm/ShortcutInfo;Landroid/content/IntentSender;)Z",
                &[JValue::Object(&info), JValue::Object(&JObject::null())],
            )?
            .z()?)
    })
}
/// A slow Binder query belongs on the Rust worker, never Java's UI thread.
/// This service returns only this app's pins; no other app/library is inspected.
pub fn pinned_games() -> Result<Vec<String>> {
    with_env(|env, activity| {
        let manager = shortcut_manager(env, activity)?;
        let list = env
            .call_method(&manager, "getPinnedShortcuts", "()Ljava/util/List;", &[])?
            .l()?;
        let count = env.call_method(&list, "size", "()I", &[])?.i()?;
        if !(0..=4096).contains(&count) {
            return Err("Unexpected shortcut count.".into());
        }
        let mut games = Vec::new();
        for index in 0..count {
            let id = env.with_local_frame(8, |env| -> Result<Option<String>> {
                let info = env
                    .call_method(&list, "get", "(I)Ljava/lang/Object;", &[JValue::Int(index)])?
                    .l()?;
                let id = JString::from(
                    env.call_method(info, "getId", "()Ljava/lang/String;", &[])?
                        .l()?,
                );
                let id: String = env.get_string(&id)?.into();
                Ok(id
                    .strip_prefix("ntx-")
                    .filter(|id| ntx::library::valid_id(id))
                    .map(str::to_owned))
            })?;
            if let Some(id) = id {
                games.push(id);
            }
        }
        Ok(games)
    })
}
pub fn notice(app: &AndroidApp, message: String) {
    let epoch = EPOCH.load(Ordering::Acquire);
    app.run_on_java_main_thread(Box::new(move || {
        if EPOCH.load(Ordering::Acquire) != epoch {
            return;
        }
        let _ = with_env(|env, activity| {
            let message = env.new_string(message)?;
            let toast = env
                .call_static_method(
                    "android/widget/Toast",
                    "makeText",
                    "(Landroid/content/Context;Ljava/lang/CharSequence;I)Landroid/widget/Toast;",
                    &[
                        JValue::Object(activity),
                        JValue::Object(&message),
                        JValue::Int(1),
                    ],
                )?
                .l()?;
            env.call_method(toast, "show", "()V", &[])?;
            Ok(())
        });
    }));
}

#[unsafe(no_mangle)]
extern "system" fn Java_com_machinerhythm_ntx_NtxActivity_documentResult(
    mut env: JNIEnv<'_>,
    activity: JObject<'_>,
    request: jint,
    result: jint,
    intent: JObject<'_>,
) {
    if !current_activity(&env, &activity) {
        return;
    }
    if request != 42 {
        return;
    }
    if result != -1 || intent.is_null() {
        push(Command::Canceled);
        return;
    }
    let selected: Result<String> = (|| {
        let uri = env
            .call_method(intent, "getData", "()Landroid/net/Uri;", &[])?
            .l()?;
        if uri.is_null() {
            return Err("no document selected".into());
        }
        let value = JString::from(
            env.call_method(&uri, "toString", "()Ljava/lang/String;", &[])?
                .l()?,
        );
        let text = env.get_string(&value)?.into();
        with_env(|env, activity| {
            let resolver = resolver(env, activity)?;
            env.call_method(
                resolver,
                "takePersistableUriPermission",
                "(Landroid/net/Uri;I)V",
                &[JValue::Object(&uri), JValue::Int(1)],
            )?;
            Ok(())
        })?;
        Ok(text)
    })();
    match selected {
        Ok(uri) => push(Command::DocumentSelected(uri)),
        Err(_) => {
            let _ = env.exception_clear();
            push(Command::Error(
                "Could not retain read access to that document.".into(),
            ));
        }
    }
}

fn current_activity(env: &JNIEnv<'_>, activity: &JObject<'_>) -> bool {
    ACTIVITY
        .lock()
        .ok()
        .and_then(|a| a.clone())
        .is_some_and(|bound| {
            env.is_same_object(bound.as_obj(), activity)
                .unwrap_or(false)
        })
}

/// Called only by the mechanical native platform adapter; no provider writes.
pub extern "C" fn dispatch(
    op: u32,
    path: *const c_char,
    argument: *const c_char,
    output: *mut c_char,
    capacity: usize,
) -> i64 {
    let result: Result<i64> = (|| {
        // SAFETY: native/bridge.h lends valid NUL-terminated strings for this call.
        let path = unsafe { CStr::from_ptr(path) }.to_str()?;
        let argument = unsafe { CStr::from_ptr(argument) }.to_str()?;
        match op {
            // Native video workers can own the final surface reference. Android
            // 13's BLAST hang-callback destructor uses GetEnv, without attaching.
            // jni's TLS guard detaches this thread when it exits, after its last
            // native window release. No Activity or UI-thread dispatch is needed.
            10 => {
                VM.get()
                    .ok_or("VM unavailable")?
                    .attach_current_thread_permanently()?;
                Ok(1)
            }
            1 if argument == "r" => Ok(i64::from(open_document(path)?.into_raw_fd())),
            2 => Ok(0),
            3 => Ok(i64::try_from(open_document(path)?.metadata()?.len())?),
            4 => {
                let text = display_name(path)?;
                if !output.is_null() {
                    if text.len() > capacity {
                        return Err("metadata buffer too small".into());
                    }
                    // SAFETY: the caller provides capacity writable bytes; lengths checked.
                    unsafe {
                        std::ptr::copy_nonoverlapping(text.as_ptr(), output.cast(), text.len());
                    }
                }
                Ok(i64::try_from(text.len())?)
            }
            7..=9 => Ok(0),
            _ => Err("unsupported provider operation".into()),
        }
    })();
    result.unwrap_or(-1)
}
