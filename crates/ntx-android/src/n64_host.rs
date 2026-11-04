// SPDX-License-Identifier: MIT
//! Android resource owner for a GLES3 libretro core. All EGL/core calls stay on
//! android_main. Surface replacement retains the same context and game identity.
use crate::platform;
use jni::{
    objects::{GlobalRef, JValue},
    sys::jshort,
};
use ndk::native_window::NativeWindow;
use ntx::{
    backend::gl_probe::ShaderProbe,
    backend::mupen::{AudioQueue, Host},
    frame_timing::DurationStats,
    n64_touch::{Target, View},
};
use std::{
    cell::RefCell,
    ffi::{CStr, c_char, c_void},
    ptr,
    rc::Rc,
    time::Instant,
};
type Result<T> = std::result::Result<T, String>;

type Handle = *mut c_void;
#[link(name = "EGL")]
unsafe extern "C" {
    fn eglGetDisplay(native: Handle) -> Handle;
    fn eglInitialize(d: Handle, major: *mut i32, minor: *mut i32) -> u32;
    fn eglChooseConfig(
        d: Handle,
        attrs: *const i32,
        cfg: *mut Handle,
        count: i32,
        n: *mut i32,
    ) -> u32;
    fn eglCreateContext(d: Handle, cfg: Handle, share: Handle, attrs: *const i32) -> Handle;
    fn eglCreatePbufferSurface(d: Handle, cfg: Handle, attrs: *const i32) -> Handle;
    fn eglCreateWindowSurface(d: Handle, cfg: Handle, window: Handle, attrs: *const i32) -> Handle;
    fn eglMakeCurrent(d: Handle, draw: Handle, read: Handle, context: Handle) -> u32;
    fn eglDestroySurface(d: Handle, s: Handle) -> u32;
    fn eglDestroyContext(d: Handle, c: Handle) -> u32;
    fn eglTerminate(d: Handle) -> u32;
    fn eglSwapBuffers(d: Handle, s: Handle) -> u32;
    fn eglSwapInterval(d: Handle, interval: i32) -> u32;
    fn eglQuerySurface(d: Handle, s: Handle, attr: i32, value: *mut i32) -> u32;
    fn eglGetProcAddress(name: *const c_char) -> *const c_void;
}
#[link(name = "GLESv3")]
unsafe extern "C" {
    fn glGenFramebuffers(n: i32, p: *mut u32);
    fn glBindFramebuffer(target: u32, name: u32);
    fn glDeleteFramebuffers(n: i32, p: *const u32);
    fn glGenTextures(n: i32, p: *mut u32);
    fn glBindTexture(target: u32, name: u32);
    fn glDeleteTextures(n: i32, p: *const u32);
    fn glTexImage2D(
        target: u32,
        level: i32,
        internal: i32,
        w: i32,
        h: i32,
        border: i32,
        format: u32,
        kind: u32,
        data: *const c_void,
    );
    fn glTexParameteri(target: u32, param: u32, value: i32);
    fn glFramebufferTexture2D(
        target: u32,
        attachment: u32,
        textarget: u32,
        texture: u32,
        level: i32,
    );
    fn glGenRenderbuffers(n: i32, p: *mut u32);
    fn glBindRenderbuffer(target: u32, name: u32);
    fn glDeleteRenderbuffers(n: i32, p: *const u32);
    fn glRenderbufferStorage(target: u32, format: u32, w: i32, h: i32);
    fn glFramebufferRenderbuffer(target: u32, attachment: u32, render_target: u32, name: u32);
    fn glCheckFramebufferStatus(target: u32) -> u32;
    fn glBlitFramebuffer(
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        dx0: i32,
        dy0: i32,
        dx1: i32,
        dy1: i32,
        mask: u32,
        filter: u32,
    );
    fn glViewport(x: i32, y: i32, w: i32, h: i32);
    fn glDisable(cap: u32);
    fn glEnable(cap: u32);
    fn glColorMask(r: u8, g: u8, b: u8, a: u8);
    fn glClearColor(r: f32, g: f32, b: f32, a: f32);
    fn glClear(mask: u32);
    fn glCreateShader(kind: u32) -> u32;
    fn glShaderSource(shader: u32, count: i32, text: *const *const c_char, length: *const i32);
    fn glCompileShader(shader: u32);
    fn glGetShaderiv(shader: u32, param: u32, value: *mut i32);
    fn glDeleteShader(shader: u32);
    fn glCreateProgram() -> u32;
    fn glAttachShader(program: u32, shader: u32);
    fn glLinkProgram(program: u32);
    fn glGetProgramiv(program: u32, param: u32, value: *mut i32);
    fn glDeleteProgram(program: u32);
    fn glUseProgram(program: u32);
    fn glGetUniformLocation(program: u32, name: *const c_char) -> i32;
    fn glUniform4f(loc: i32, a: f32, b: f32, c: f32, d: f32);
    fn glUniform1f(loc: i32, a: f32);
    fn glGenVertexArrays(n: i32, p: *mut u32);
    fn glBindVertexArray(name: u32);
    fn glDeleteVertexArrays(n: i32, p: *const u32);
    fn glBlendFunc(a: u32, b: u32);
    fn glDrawArrays(kind: u32, first: i32, count: i32);
    fn glGetError() -> u32;
}
pub struct Graphics {
    display: Handle,
    config: Handle,
    context: Handle,
    pbuffer: Handle,
    surface: Handle,
    window: Option<NativeWindow>,
    fbo: u32,
    texture: u32,
    depth: u32,
    program: u32,
    vao: u32,
    rect: i32,
    color: i32,
    round: i32,
    opacity: f32,
    pub backend_gl_errors: u64,
    pub present_timing: DurationStats,
    pub swap_timing: DurationStats,
}
impl Graphics {
    pub fn new(window: &NativeWindow) -> Result<Self> {
        let mut g = Self {
            display: ptr::null_mut(),
            config: ptr::null_mut(),
            context: ptr::null_mut(),
            pbuffer: ptr::null_mut(),
            surface: ptr::null_mut(),
            window: None,
            fbo: 0,
            texture: 0,
            depth: 0,
            program: 0,
            vao: 0,
            rect: 0,
            color: 0,
            round: 0,
            opacity: 0.85,
            backend_gl_errors: 0,
            present_timing: DurationStats::default(),
            swap_timing: DurationStats::default(),
        };
        // SAFETY: EGL outputs point to initialized owned fields, all calls on owner thread.
        unsafe {
            g.display = eglGetDisplay(ptr::null_mut());
            if g.display.is_null()
                || eglInitialize(g.display, ptr::null_mut(), ptr::null_mut()) == 0
            {
                return Err("N64 EGL display initialization failed".into());
            }
            let attrs = [
                0x3033, 5, 0x3040, 0x40, 0x3024, 8, 0x3023, 8, 0x3022, 8, 0x3021, 8, 0x3038,
            ];
            let mut n = 0;
            if eglChooseConfig(g.display, attrs.as_ptr(), &mut g.config, 1, &mut n) == 0 || n != 1 {
                return Err("N64 GLES3 configuration unavailable".into());
            }
            g.context = eglCreateContext(
                g.display,
                g.config,
                ptr::null_mut(),
                [0x3098, 3, 0x3038].as_ptr(),
            );
            g.pbuffer = eglCreatePbufferSurface(
                g.display,
                g.config,
                [0x3057, 1, 0x3056, 1, 0x3038].as_ptr(),
            );
            if g.context.is_null() || g.pbuffer.is_null() {
                return Err("N64 GLES3 context unavailable".into());
            }
        }
        g.set_window(Some(window))?;
        // SAFETY: current GLES3 context and live GL objects owned until Drop.
        unsafe {
            glGenFramebuffers(1, &mut g.fbo);
            glBindFramebuffer(0x8d40, g.fbo);
            glGenTextures(1, &mut g.texture);
            glBindTexture(0x0de1, g.texture);
            glTexImage2D(0x0de1, 0, 0x8058, 640, 480, 0, 0x1908, 0x1401, ptr::null());
            glTexParameteri(0x0de1, 0x2801, 0x2601);
            glTexParameteri(0x0de1, 0x2800, 0x2601);
            glFramebufferTexture2D(0x8d40, 0x8ce0, 0x0de1, g.texture, 0);
            glGenRenderbuffers(1, &mut g.depth);
            glBindRenderbuffer(0x8d41, g.depth);
            glRenderbufferStorage(0x8d41, 0x88f0, 640, 480);
            glFramebufferRenderbuffer(0x8d40, 0x821a, 0x8d41, g.depth);
            if glCheckFramebufferStatus(0x8d40) != 0x8cd5 {
                return Err("N64 framebuffer is incomplete".into());
            }
            let vertex=c"#version 300 es\nprecision highp float;uniform vec4 rect;out vec2 uv;void main(){vec2 p=vec2(float(gl_VertexID&1),float((gl_VertexID>>1)&1));uv=p*2.-1.;gl_Position=vec4(mix(rect.xy,rect.zw,p),0,1);}";
            let fragment=c"#version 300 es\nprecision mediump float;in vec2 uv;uniform vec4 color;uniform float roundness;out vec4 result;void main(){if(roundness>0.5 && dot(uv,uv)>1.)discard;result=color;}";
            let mut shaders = [0; 2];
            for (i, (kind, src)) in [(0x8b31, vertex), (0x8b30, fragment)]
                .into_iter()
                .enumerate()
            {
                shaders[i] = glCreateShader(kind);
                let p = src.as_ptr();
                glShaderSource(shaders[i], 1, &p, ptr::null());
                glCompileShader(shaders[i]);
                let mut ok = 0;
                glGetShaderiv(shaders[i], 0x8b81, &mut ok);
                if ok == 0 {
                    for s in shaders {
                        if s != 0 {
                            glDeleteShader(s);
                        }
                    }
                    return Err("N64 controls shader compilation failed".into());
                }
            }
            g.program = glCreateProgram();
            for s in shaders {
                glAttachShader(g.program, s);
            }
            glLinkProgram(g.program);
            for s in shaders {
                glDeleteShader(s);
            }
            let mut ok = 0;
            glGetProgramiv(g.program, 0x8b82, &mut ok);
            if ok == 0 {
                return Err("N64 controls shader link failed".into());
            }
            g.rect = glGetUniformLocation(g.program, c"rect".as_ptr());
            g.color = glGetUniformLocation(g.program, c"color".as_ptr());
            g.round = glGetUniformLocation(g.program, c"roundness".as_ptr());
            glGenVertexArrays(1, &mut g.vao);
            glBindFramebuffer(0x8d40, 0);
            glBindTexture(0x0de1, 0);
            glBindRenderbuffer(0x8d41, 0);
        }
        Ok(g)
    }
    pub fn set_window(&mut self, window: Option<&NativeWindow>) -> Result<()> {
        // SAFETY: serialize surface changes while core is quiescent; pbuffer retains context.
        unsafe {
            if eglMakeCurrent(self.display, self.pbuffer, self.pbuffer, self.context) == 0 {
                return Err("N64 context retention failed".into());
            }
            if !self.surface.is_null() {
                eglDestroySurface(self.display, self.surface);
                self.surface = ptr::null_mut();
            }
            self.window = None;
            if let Some(w) = window {
                self.surface = eglCreateWindowSurface(
                    self.display,
                    self.config,
                    w.ptr().as_ptr().cast(),
                    [0x3038].as_ptr(),
                );
                if self.surface.is_null() {
                    return Err("N64 drawable creation failed".into());
                }
                self.window = Some(w.clone());
                if eglMakeCurrent(self.display, self.surface, self.surface, self.context) == 0 {
                    return Err("N64 drawable binding failed".into());
                }
                eglSwapInterval(self.display, 1);
            }
        }
        Ok(())
    }
    fn draw(&self, a: [f32; 4], color: [f32; 4], round: bool, w: f32, h: f32) {
        // SAFETY: current context, initialized program/VAO, uniforms belong to this program.
        unsafe {
            glUniform4f(
                self.rect,
                a[0] * 2. / w - 1.,
                1. - a[3] * 2. / h,
                a[2] * 2. / w - 1.,
                1. - a[1] * 2. / h,
            );
            glUniform4f(
                self.color,
                color[0],
                color[1],
                color[2],
                color[3] * self.opacity,
            );
            glUniform1f(self.round, if round { 1. } else { 0. });
            glDrawArrays(5, 0, 4);
        }
    }
    fn present(&mut self, width: u32, height: u32, view: &View) -> Result<()> {
        self.opacity = crate::player::controls_opacity();
        if self.surface.is_null() {
            return Ok(());
        }
        if width > 640 || height > 480 {
            return Err("N64 framebuffer dimensions changed".into());
        }
        // SAFETY: core has unbound its tracked state before the host video callback.
        unsafe {
            // Error flags belong to the shared context and survive the core's
            // state unbind. Diagnose preexisting core flags before attributing
            // subsequent errors to our own framebuffer copy or overlay.
            for attempt in 0..16 {
                let error = glGetError();
                if error == 0 {
                    break;
                }
                if !matches!(error, 0x0500 | 0x0501 | 0x0502 | 0x0506) {
                    // OOM/context loss may leave state unusable; never continue.
                    return Err(format!(
                        "N64 backend GL failure before presentation {error:x}"
                    ));
                }
                self.backend_gl_errors = self.backend_gl_errors.saturating_add(1);
                if self.backend_gl_errors <= 8 {
                    platform::diagnostic(&format!(
                        "Mupen GL flag {error:x} already present before NTX presentation; backend diagnostic {}",
                        self.backend_gl_errors
                    ));
                }
                if attempt == 15 {
                    return Err("N64 backend GL flags did not clear at the host boundary".into());
                }
            }
            let (mut w, mut h) = (0, 0);
            if eglQuerySurface(self.display, self.surface, 0x3057, &mut w) == 0
                || eglQuerySurface(self.display, self.surface, 0x3056, &mut h) == 0
                || w < 2
                || h < 2
            {
                return Err("N64 drawable size unavailable".into());
            }
            if !view.controls_fit {
                platform::request_layout_choice();
                return Ok(());
            }
            let Some(picture) = view.picture(w as f32, h as f32, 4. / 3.) else {
                return Ok(());
            };
            glBindFramebuffer(0x8ca9, 0);
            glDisable(0x0c11);
            glDisable(0x0b71);
            glDisable(0x0b90);
            glDisable(0x0b44);
            glDisable(0x0be2);
            glColorMask(1, 1, 1, 1);
            glClearColor(0., 0., 0., 1.);
            glClear(0x4000);
            platform::record_editor_frame(picture, w as f32, h as f32, 4. / 3.);
            glBindFramebuffer(0x8ca8, self.fbo);
            glBlitFramebuffer(
                0,
                0,
                width as i32,
                height as i32,
                picture.left.round() as i32,
                h - picture.bottom.round() as i32,
                picture.right.round() as i32,
                h - picture.top.round() as i32,
                0x4000,
                0x2600,
            );
            let error = glGetError();
            if error != 0 {
                return Err(format!("N64 host framebuffer-copy GL error {error:x}"));
            }
            glBindFramebuffer(0x8d40, 0);
            glViewport(0, 0, w, h);
            glUseProgram(self.program);
            glBindVertexArray(self.vao);
            glEnable(0x0be2);
            glBlendFunc(0x0302, 0x0303);
            for zone in &view.zones {
                let a = zone.area;
                let Some((l, t)) = view.screen_to_buffer(a.left, a.top, w as f32, h as f32) else {
                    continue;
                };
                let Some((r, b)) = view.screen_to_buffer(a.right, a.bottom, w as f32, h as f32)
                else {
                    continue;
                };
                let mut color = match zone.target {
                    Target::Stick => [0.65, 0.67, 0.70, 0.42],
                    Target::Button(ntx::n64::Button::A) => [0.1, 0.45, 1., 0.60],
                    Target::Button(ntx::n64::Button::B) => [0.1, 0.8, 0.35, 0.60],
                    Target::Button(
                        ntx::n64::Button::CUp
                        | ntx::n64::Button::CDown
                        | ntx::n64::Button::CLeft
                        | ntx::n64::Button::CRight,
                    ) => [1., 0.8, 0.12, 0.6],
                    _ => [0.45, 0.47, 0.5, 0.65],
                };
                if let Target::Button(button) = zone.target
                    && view.sampled.pressed(button)
                {
                    color[3] = 0.95;
                }
                self.draw([l, t, r, b], color, true, w as f32, h as f32);
                if zone.target == Target::Stick {
                    let x = view.sampled.input_state(0, 5, 0, 0) as f32 / 32767.;
                    let y = view.sampled.input_state(0, 5, 0, 1) as f32 / 32767.;
                    let radius = (r - l).min(b - t) * 0.18;
                    let cx = (l + r) * 0.5 + x * (r - l) * 0.32;
                    let cy = (t + b) * 0.5 + y * (b - t) * 0.32;
                    self.draw(
                        [cx - radius, cy - radius, cx + radius, cy + radius],
                        [0.85, 0.87, 0.9, 0.85],
                        true,
                        w as f32,
                        h as f32,
                    );
                }
                let glyph = match zone.label {
                    "A" => [14, 17, 17, 31, 17, 17, 17],
                    "B" => [30, 17, 17, 30, 17, 17, 30],
                    "L" => [16, 16, 16, 16, 16, 16, 31],
                    "R" => [30, 17, 17, 30, 20, 18, 17],
                    "Z" => [31, 1, 2, 4, 8, 16, 31],
                    "S" => [15, 16, 16, 14, 1, 1, 30],
                    "<" => [2, 4, 8, 16, 8, 4, 2],
                    ">" => [8, 4, 2, 1, 2, 4, 8],
                    "^" => [4, 14, 21, 4, 4, 4, 4],
                    "v" => [4, 4, 4, 4, 21, 14, 4],
                    "II" => [10, 10, 10, 10, 10, 10, 10],
                    _ => [0; 7],
                };
                let pixel = ((r - l) / 13.).max(1.);
                let ox = (l + r) / 2. - 2.5 * pixel;
                let oy = (t + b) / 2. - 3.5 * pixel;
                for (row, bits) in glyph.iter().enumerate() {
                    for col in 0..5 {
                        if bits & (1 << (4 - col)) != 0 {
                            let x = ox + col as f32 * pixel;
                            let y = oy + row as f32 * pixel;
                            self.draw(
                                [x, y, x + pixel, y + pixel],
                                [1., 1., 1., 0.9],
                                false,
                                w as f32,
                                h as f32,
                            );
                        }
                    }
                }
            }
            glDisable(0x0be2);
            glBindVertexArray(0);
            glUseProgram(0);
            glBindFramebuffer(0x8d40, 0);
            let error = glGetError();
            if error != 0 {
                return Err(format!("N64 host controls-drawing GL error {error:x}"));
            }
            let swap_start = Instant::now();
            let swapped = eglSwapBuffers(self.display, self.surface);
            self.swap_timing.record(swap_start.elapsed());
            if swapped == 0 {
                return Err("N64 surface presentation failed".into());
            }
        }
        Ok(())
    }
}
impl Drop for Graphics {
    fn drop(&mut self) {
        // SAFETY: core and DLL are destroyed before this host resource owner.
        unsafe {
            if !self.context.is_null() && !self.pbuffer.is_null() {
                eglMakeCurrent(self.display, self.pbuffer, self.pbuffer, self.context);
                glDeleteFramebuffers(1, &self.fbo);
                glDeleteTextures(1, &self.texture);
                glDeleteRenderbuffers(1, &self.depth);
                if self.program != 0 {
                    glDeleteProgram(self.program);
                }
                glDeleteVertexArrays(1, &self.vao);
            }
            eglMakeCurrent(
                self.display,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            );
            if !self.surface.is_null() {
                eglDestroySurface(self.display, self.surface);
            }
            if !self.pbuffer.is_null() {
                eglDestroySurface(self.display, self.pbuffer);
            }
            if !self.context.is_null() {
                eglDestroyContext(self.display, self.context);
            }
            if !self.display.is_null() {
                eglTerminate(self.display);
            }
        }
    }
}
pub struct Audio {
    track: GlobalRef,
    playing: bool,
}
impl Audio {
    fn new() -> Result<Self> {
        platform::with_env(|env, _| {
            let size = env
                .call_static_method(
                    "android/media/AudioTrack",
                    "getMinBufferSize",
                    "(III)I",
                    &[44100.into(), 12.into(), 2.into()],
                )?
                .i()?;
            if size <= 0 {
                return Err("N64 stereo audio unavailable".into());
            }
            let track = env.new_object(
                "android/media/AudioTrack",
                "(IIIIII)V",
                &[
                    3.into(),
                    44100.into(),
                    12.into(),
                    2.into(),
                    size.max(16384).into(),
                    1.into(),
                ],
            )?;
            if env.call_method(&track, "getState", "()I", &[])?.i()? != 1 {
                return Err("N64 audio initialization failed".into());
            }
            Ok(Self {
                track: env.new_global_ref(track)?,
                playing: false,
            })
        })
        .map_err(|e| e.to_string())
    }
    fn write(&mut self, bytes: &[i16]) -> Result<usize> {
        platform::with_env(|env, _| {
            if !self.playing {
                env.call_method(self.track.as_obj(), "play", "()V", &[])?;
                self.playing = true;
            }
            let array = env.new_short_array(bytes.len() as i32)?;
            env.set_short_array_region(&array, 0, bytes as &[jshort])?;
            let mut written = 0;
            while written < bytes.len() {
                let n = env
                    .call_method(
                        self.track.as_obj(),
                        "write",
                        "([SII)I",
                        &[
                            JValue::Object(array.as_ref()),
                            (written as i32).into(),
                            ((bytes.len() - written) as i32).into(),
                        ],
                    )?
                    .i()?;
                if n <= 0 {
                    return Err("N64 audio write failed".into());
                }
                written += n as usize;
            }
            Ok(written / 2)
        })
        .map_err(|e| e.to_string())
    }
    fn silence(&mut self) {
        if self.playing {
            let _ = platform::with_env(|env, _| {
                env.call_method(self.track.as_obj(), "pause", "()V", &[])?;
                env.call_method(self.track.as_obj(), "flush", "()V", &[])?;
                Ok(())
            });
            self.playing = false;
        }
    }
}
impl Drop for Audio {
    fn drop(&mut self) {
        self.silence();
        let _ = platform::with_env(|env, _| {
            env.call_method(self.track.as_obj(), "release", "()V", &[])?;
            Ok(())
        });
    }
}
pub struct Resources {
    pub graphics: Graphics,
    audio: Audio,
    pending_audio: AudioQueue,
    pub written_audio_frames: u64,
    pub muted: bool,
    pub shader_probe: ShaderProbe,
}
impl Resources {
    pub fn new(window: &NativeWindow) -> Result<Self> {
        Ok(Self {
            graphics: Graphics::new(window)?,
            audio: Audio::new()?,
            pending_audio: AudioQueue::default(),
            written_audio_frames: 0,
            muted: true,
            shader_probe: ShaderProbe::new().map_err(str::to_owned)?,
        })
    }
    /// JNI may enter managed code only on the outer Android owner stack, after
    /// retro_run has returned from Mupen's libco/dynarec stack.
    pub fn deliver_audio(&mut self) -> Result<()> {
        if self.muted {
            self.pending_audio.clear();
            return Ok(());
        }
        let audio = &mut self.audio;
        let written = self.pending_audio.drain(|samples| audio.write(samples))?;
        self.written_audio_frames = self.written_audio_frames.saturating_add(written as u64);
        Ok(())
    }
}
pub struct Adapter(pub Rc<RefCell<Resources>>);
impl Host for Adapter {
    fn jit_capable(&self) -> bool {
        unsafe extern "C" {
            fn mmap(
                addr: *mut c_void,
                len: usize,
                prot: i32,
                flags: i32,
                fd: i32,
                offset: i64,
            ) -> *mut c_void;
            fn munmap(addr: *mut c_void, len: usize) -> i32;
        }
        // SAFETY: probe only a new anonymous private page; no executable code is run.
        unsafe {
            let p = mmap(ptr::null_mut(), 16384, 7, 0x22, -1, 0);
            if p as isize == -1 {
                false
            } else {
                munmap(p, 16384);
                true
            }
        }
    }
    fn framebuffer(&self) -> usize {
        self.0.borrow().graphics.fbo as usize
    }
    fn proc_address(&self, name: &CStr) -> *const c_void {
        // These four base functions already belong to this host's GLES3 ABI;
        // base-function lookup through EGL is not required to expose them.
        // SAFETY: exact imported GL types or the trusted EGL resolver supply the
        // named ABI. Probes preserve arguments/results and GL state/error flags.
        unsafe {
            let address = match name.to_bytes() {
                b"glCompileShader" => glCompileShader as *const c_void,
                b"glLinkProgram" => glLinkProgram as *const c_void,
                b"glGetShaderiv" => glGetShaderiv as *const c_void,
                b"glGetProgramiv" => glGetProgramiv as *const c_void,
                _ => eglGetProcAddress(name.as_ptr()),
            };
            self.0.borrow().shader_probe.intercept(name, address)
        }
    }
    fn present(&mut self, w: u32, h: u32) -> Result<()> {
        let start = Instant::now();
        let mut r = self.0.borrow_mut();
        let view = view().lock().map_err(|_| "N64 input lock poisoned")?;
        let result = r.graphics.present(w, h, &view);
        r.graphics.present_timing.record(start.elapsed());
        result
    }
    fn audio(&mut self, bytes: &[i16]) -> Result<usize> {
        let mut r = self.0.borrow_mut();
        if r.muted {
            Ok(bytes.len() / 2)
        } else {
            // No JNI on this callback's alternate coroutine stack.
            r.pending_audio.push(bytes)
        }
    }
    fn poll_input(&mut self) {
        if let Ok(mut view) = view().lock() {
            view.poll();
        }
    }
    fn input(&self, port: u32, device: u32, index: u32, id: u32) -> i16 {
        view()
            .lock()
            .map_or(0, |v| v.sampled.input_state(port, device, index, id))
    }
    fn silence(&mut self) {
        let mut r = self.0.borrow_mut();
        r.pending_audio.clear();
        r.audio.silence();
        if let Ok(mut view) = view().lock() {
            view.cancel();
        }
    }
}

pub fn view() -> &'static std::sync::Mutex<View> {
    static VIEW: std::sync::OnceLock<std::sync::Mutex<View>> = std::sync::OnceLock::new();
    VIEW.get_or_init(|| std::sync::Mutex::new(View::default()))
}
