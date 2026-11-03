// SPDX-License-Identifier: MIT
//! Owner-thread GL pass-through probes. No GL state, error flags or game data.
use crate::frame_timing::DurationStats;
use std::{
    cell::RefCell,
    ffi::{CStr, c_void},
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

type ObjectFn = unsafe extern "C" fn(u32);
type QueryFn = unsafe extern "C" fn(u32, u32, *mut i32);

#[derive(Clone, Copy)]
enum Stage {
    Compile,
    Link,
    ShaderQuery,
    ProgramQuery,
}

#[derive(Default)]
struct Timings {
    frames: u32,
    active: bool,
    calls: u32,
    elapsed: Duration,
    resolved: u8,
    shader_frames: u32,
    slow_core: u32,
    slow_with_shader: u32,
    residual: DurationStats,
    compile: DurationStats,
    link: DurationStats,
    shader_query: DurationStats,
    program_query: DurationStats,
}
impl Timings {
    fn record(&mut self, stage: Stage, duration: Duration) {
        if !self.active {
            return;
        }
        self.calls = self.calls.saturating_add(1);
        self.elapsed = self.elapsed.saturating_add(duration);
        match stage {
            Stage::Compile => &mut self.compile,
            Stage::Link => &mut self.link,
            Stage::ShaderQuery => &mut self.shader_query,
            Stage::ProgramQuery => &mut self.program_query,
        }
        .record(duration);
    }
}

#[derive(Default)]
struct Dispatch {
    owner: Weak<RefCell<Timings>>,
    compile: Option<ObjectFn>,
    link: Option<ObjectFn>,
    shader_query: Option<QueryFn>,
    program_query: Option<QueryFn>,
}
thread_local! { static DISPATCH: RefCell<Dispatch> = RefCell::new(Dispatch::default()); }

/// One non-Send scope follows the core's qualified owner thread and resources.
/// The host must join the old core before retiring this scope or starting another.
/// Function addresses are transient, never checkpoint or game identity.
pub struct ShaderProbe(Rc<RefCell<Timings>>);
impl ShaderProbe {
    pub fn new() -> Result<Self, &'static str> {
        let inner = Rc::new(RefCell::new(Timings::default()));
        DISPATCH.with(|d| {
            let mut d = d.borrow_mut();
            if d.owner.upgrade().is_some() {
                return Err("GL probe owner already present");
            }
            d.owner = Rc::downgrade(&inner);
            Ok(Self(inner))
        })
    }
    /// # Safety
    /// `address` must be the original driver entry point for the exact named GL
    /// function, resolved on this scope's owner thread. Never pass a wrapper back.
    pub unsafe fn intercept(&self, name: &CStr, address: *const c_void) -> *const c_void {
        if address.is_null() {
            return address;
        }
        DISPATCH.with(|d| {
            let mut d = d.borrow_mut();
            if !d.owner.upgrade().is_some_and(|o| Rc::ptr_eq(&o, &self.0)) {
                return address;
            }
            // SAFETY: caller supplies the exact named GL ABI on Linux/Android.
            let (result, bit) = unsafe {
                match name.to_bytes() {
                    b"glCompileShader" => {
                        d.compile = Some(std::mem::transmute::<*const c_void, ObjectFn>(address));
                        (compile as *const c_void, 1)
                    }
                    b"glLinkProgram" => {
                        d.link = Some(std::mem::transmute::<*const c_void, ObjectFn>(address));
                        (link as *const c_void, 2)
                    }
                    b"glGetShaderiv" => {
                        d.shader_query =
                            Some(std::mem::transmute::<*const c_void, QueryFn>(address));
                        (shader_query as *const c_void, 4)
                    }
                    b"glGetProgramiv" => {
                        d.program_query =
                            Some(std::mem::transmute::<*const c_void, QueryFn>(address));
                        (program_query as *const c_void, 8)
                    }
                    _ => return address,
                }
            };
            self.0.borrow_mut().resolved |= bit;
            result
        })
    }
    pub fn begin_frame(&self) {
        let mut t = self.0.borrow_mut();
        t.active = t.frames < DurationStats::LIMIT;
        t.calls = 0;
        t.elapsed = Duration::ZERO;
        if t.active {
            t.frames += 1;
        }
    }
    pub fn end_frame(&self, core_elapsed: Duration) {
        let mut t = self.0.borrow_mut();
        if !t.active {
            return;
        }
        t.shader_frames += u32::from(t.calls != 0);
        if core_elapsed > Duration::from_millis(40) {
            t.slow_core += 1;
            t.slow_with_shader += u32::from(t.calls != 0);
            let residual = core_elapsed.saturating_sub(t.elapsed);
            t.residual.record(residual);
        }
        t.active = false;
    }
    pub fn summary(&self) -> String {
        let t = self.0.borrow();
        format!(
            "frames={} resolved_mask={} shader_frames={} slow_core={} slow_with_shader={} slow_residual[{}] compile[{}] link[{}] shader_query[{}] program_query[{}]",
            t.frames,
            t.resolved,
            t.shader_frames,
            t.slow_core,
            t.slow_with_shader,
            t.residual,
            t.compile,
            t.link,
            t.shader_query,
            t.program_query
        )
    }
}
impl Drop for ShaderProbe {
    fn drop(&mut self) {
        DISPATCH.with(|d| {
            let mut d = d.borrow_mut();
            if d.owner.ptr_eq(&Rc::downgrade(&self.0)) {
                d.owner = Weak::new();
            }
            // Keep the driver pass-throughs for the host thread's lifetime.
            // Native join, rather than a function pointer, establishes that the
            // old core cannot call again before another runtime takes ownership.
        });
    }
}

fn observe(stage: Stage, call: impl FnOnce()) {
    let owner = DISPATCH.with(|d| d.borrow().owner.upgrade());
    let owner = owner.filter(|o| o.borrow().active);
    let start = owner.as_ref().map(|_| Instant::now());
    // Hold no Rust borrow across the driver call or possible driver callbacks.
    call();
    if let (Some(owner), Some(start)) = (owner, start) {
        owner.borrow_mut().record(stage, start.elapsed());
    }
}
unsafe extern "C" fn compile(object: u32) {
    let f = DISPATCH
        .with(|d| d.borrow().compile)
        .unwrap_or_else(|| std::process::abort());
    observe(Stage::Compile, || unsafe { f(object) });
}
unsafe extern "C" fn link(object: u32) {
    let f = DISPATCH
        .with(|d| d.borrow().link)
        .unwrap_or_else(|| std::process::abort());
    observe(Stage::Link, || unsafe { f(object) });
}
unsafe extern "C" fn shader_query(object: u32, parameter: u32, output: *mut i32) {
    let f = DISPATCH
        .with(|d| d.borrow().shader_query)
        .unwrap_or_else(|| std::process::abort());
    observe(Stage::ShaderQuery, || unsafe {
        f(object, parameter, output)
    });
}
unsafe extern "C" fn program_query(object: u32, parameter: u32, output: *mut i32) {
    let f = DISPATCH
        .with(|d| d.borrow().program_query)
        .unwrap_or_else(|| std::process::abort());
    observe(Stage::ProgramQuery, || unsafe {
        f(object, parameter, output)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_function_pointer_passthrough_preserves_arguments_outputs_and_owner_retirement() {
        thread_local! { static OBJECT: RefCell<u32> = const { RefCell::new(0) }; }
        unsafe extern "C" fn original(object: u32) {
            OBJECT.with(|o| *o.borrow_mut() = object);
        }
        unsafe extern "C" fn query(object: u32, parameter: u32, output: *mut i32) {
            unsafe {
                *output = (object + parameter) as i32;
            }
        }
        let p = ShaderProbe::new().unwrap();
        assert!(ShaderProbe::new().is_err());
        let original = original as *const c_void;
        assert_eq!(unsafe { p.intercept(c"unrelated", original) }, original);
        assert!(unsafe { p.intercept(c"glLinkProgram", std::ptr::null()) }.is_null());
        let f = unsafe { p.intercept(c"glCompileShader", original) };
        let f = unsafe { std::mem::transmute::<*const c_void, ObjectFn>(f) };
        let q = unsafe { p.intercept(c"glGetProgramiv", query as *const c_void) };
        let q = unsafe { std::mem::transmute::<*const c_void, QueryFn>(q) };
        p.begin_frame();
        unsafe {
            f(37);
        }
        let mut output = 0;
        unsafe {
            q(40, 2, &mut output);
        }
        p.end_frame(Duration::from_millis(50));
        assert_eq!(OBJECT.with(|o| *o.borrow()), 37);
        assert_eq!(output, 42);
        assert_eq!(p.0.borrow().compile.samples(), 1);
        assert_eq!(p.0.borrow().program_query.samples(), 1);
        let old = Rc::downgrade(&p.0);
        drop(p);
        assert!(old.upgrade().is_none());
        // Retirement keeps the original driver pass-through but no old metrics.
        unsafe {
            f(7);
        }
        assert_eq!(OBJECT.with(|o| *o.borrow()), 7);
        let next = ShaderProbe::new().unwrap();
        assert_eq!(next.0.borrow().compile.samples(), 0);
    }
    #[test]
    fn slow_frame_correlation_uses_its_own_shader_time_and_stops_at_the_frame_bound() {
        let p = ShaderProbe::new().unwrap();
        p.begin_frame();
        p.0.borrow_mut()
            .record(Stage::Link, Duration::from_millis(75));
        p.end_frame(Duration::from_millis(80));
        p.begin_frame();
        p.end_frame(Duration::from_millis(60));
        assert_eq!(p.0.borrow().slow_core, 2);
        assert_eq!(p.0.borrow().slow_with_shader, 1);
        assert!(p.0.borrow().residual.to_string().contains("max_us=60000"));
        for _ in 2..DurationStats::LIMIT {
            p.begin_frame();
            p.end_frame(Duration::ZERO);
        }
        let frozen = p.summary();
        p.begin_frame();
        p.0.borrow_mut()
            .record(Stage::Compile, Duration::from_secs(2));
        p.end_frame(Duration::from_secs(3));
        assert_eq!(p.summary(), frozen);
    }
}
