// SPDX-License-Identifier: MIT
//! App-local editor views. Rust owns all state; the Java classes forward View callbacks.
use super::*;
use ntx::{
    controls::{Editor, Settings},
    presentation as theme,
    touch::{Area, Target},
};

struct Session {
    editor: Editing,
    menu: u64,
    activity: u64,
}
pub(super) enum Editing {
    GameCube(Box<Editor>),
    Nintendo64(Box<ntx::n64_controls::Editor>),
}
impl Editing {
    fn size(&self) -> (f32, f32) {
        match self {
            Self::GameCube(e) => (e.width, e.height),
            Self::Nintendo64(e) => (e.width, e.height),
        }
    }
    fn settings(&mut self) -> &mut Settings {
        match self {
            Self::GameCube(e) => &mut e.settings,
            Self::Nintendo64(e) => &mut e.settings,
        }
    }
    fn configure_test(&mut self) {
        match self {
            Self::GameCube(e) => e.configure_test(),
            Self::Nintendo64(e) => e.configure_test(),
        }
    }
    fn resize(&mut self, v: f32) {
        match self {
            Self::GameCube(e) => e.resize(v),
            Self::Nintendo64(e) => e.resize(v),
        }
    }
    fn reset_display(&mut self) {
        match self {
            Self::GameCube(e) => e.reset_display(),
            Self::Nintendo64(e) => e.reset_display(),
        }
    }
    fn toggle_test(&mut self) {
        match self {
            Self::GameCube(e) => e.toggle_test(),
            Self::Nintendo64(e) => e.toggle_test(),
        }
    }
    fn down(&mut self, p: i32, x: f32, y: f32) -> bool {
        match self {
            Self::GameCube(e) => e.down(p, x, y),
            Self::Nintendo64(e) => e.down(p, x, y),
        }
    }
    fn moved(&mut self, p: i32, x: f32, y: f32) {
        match self {
            Self::GameCube(e) => e.moved(p, x, y),
            Self::Nintendo64(e) => e.moved(p, x, y),
        }
    }
    fn up(&mut self, p: i32) {
        match self {
            Self::GameCube(e) => e.up(p),
            Self::Nintendo64(e) => e.up(p),
        }
    }
    fn cancel(&mut self) {
        match self {
            Self::GameCube(e) => e.cancel(),
            Self::Nintendo64(e) => e.cancel(),
        }
    }
}
static EDITOR: Mutex<Option<Session>> = Mutex::new(None);
pub(super) fn bind(editor: Option<Editing>, menu: u64, activity: u64) {
    if let Ok(mut slot) = EDITOR.lock() {
        *slot = editor.map(|editor| Session {
            editor,
            menu,
            activity,
        });
    }
}
fn with_editing<T>(menu: u64, f: impl FnOnce(&mut Editing) -> T) -> Option<T> {
    if !menu_current(menu) {
        return None;
    }
    let mut slot = EDITOR.lock().ok()?;
    let session = slot.as_mut()?;
    (session.menu == menu && session.activity == EPOCH.load(Ordering::Acquire))
        .then(|| f(&mut session.editor))
}
pub(super) fn active() -> bool {
    with_editing(MENU_GENERATION.load(Ordering::Acquire), |_| ()).is_some()
}
fn with_editor<T>(menu: u64, f: impl FnOnce(&mut Editor) -> T) -> Option<T> {
    with_editing(menu, |e| match e {
        Editing::GameCube(e) => Some(f(e)),
        _ => None,
    })
    .flatten()
}
fn with_n64_editor<T>(menu: u64, f: impl FnOnce(&mut ntx::n64_controls::Editor) -> T) -> Option<T> {
    with_editing(menu, |e| match e {
        Editing::Nintendo64(e) => Some(f(e)),
        _ => None,
    })
    .flatten()
}
pub(super) fn applied(menu: u64) -> Option<Settings> {
    with_editing(menu, |e| match e {
        Editing::GameCube(e) => e.finish().ok(),
        Editing::Nintendo64(e) => e.finish().ok(),
    })
    .flatten()
}
pub(super) fn display(menu: u64) -> Option<String> {
    with_editing(menu, |e| {
        let (w, h) = e.size();
        ntx::controls::display_key(w, h)
    })
}
pub(super) fn show(app: &AndroidApp, runtime: u64, editor: Editor) {
    render_menu_with_games(app,true,"Touch controls\nDrag a control to move it. Select it, then change its size. Layouts are remembered separately for each screen.".into(),
        vec![(20,"Smaller selected control".into()),(21,"Larger selected control".into()),(23,"Test inputs".into()),(24,"Haptics".into()),(25,"Following sticks".into()),(22,"Reset this screen’s layout".into()),(10,"Save controls".into()),(11,"Cancel".into())],runtime,Vec::new(),Some(Editing::GameCube(Box::new(editor))));
}
pub(super) fn show_n64(app: &AndroidApp, runtime: u64, editor: ntx::n64_controls::Editor) {
    render_menu_with_games(app,true,"N64 touch controls\nDrag a control to move it. Select it, then change its size. N64 layouts are remembered separately for each screen.".into(),
        vec![(20,"Smaller selected control".into()),(21,"Larger selected control".into()),(23,"Test inputs".into()),(24,"Haptics".into()),(22,"Reset this screen’s layout".into()),(10,"Save controls".into()),(11,"Cancel".into())],runtime,Vec::new(),Some(Editing::Nintendo64(Box::new(editor))));
}
fn label<'a>(
    env: &mut JNIEnv<'a>,
    activity: &JObject<'_>,
    text: &str,
    size: f32,
    color: u32,
) -> Result<JObject<'a>> {
    let view = env.new_object(
        "android/widget/TextView",
        "(Landroid/content/Context;)V",
        &[JValue::Object(activity)],
    )?;
    set_text(env, &view, text)?;
    env.call_method(&view, "setTextSize", "(F)V", &[JValue::Float(size)])?;
    env.call_method(&view, "setTextColor", "(I)V", &[JValue::Int(color as i32)])?;
    Ok(view)
}
fn set_text(env: &mut JNIEnv<'_>, view: &JObject<'_>, text: &str) -> Result<()> {
    let text = env.new_string(text)?;
    env.call_method(
        view,
        "setText",
        "(Ljava/lang/CharSequence;)V",
        &[JValue::Object(&text)],
    )?;
    Ok(())
}
fn tag(env: &mut JNIEnv<'_>, view: &JObject<'_>, text: &str) -> Result<()> {
    let text = env.new_string(text)?;
    env.call_method(
        view,
        "setTag",
        "(Ljava/lang/Object;)V",
        &[JValue::Object(&text)],
    )?;
    Ok(())
}
pub(super) fn append(
    env: &mut JNIEnv<'_>,
    activity: &JObject<'_>,
    parent: &JObject<'_>,
    density: f32,
    menu: u64,
) -> Result<()> {
    let (width, height, settings, n64) = with_editing(menu, |e| {
        let (w, h) = e.size();
        let n64 = matches!(e, Editing::Nintendo64(_));
        (w, h, e.settings().clone(), n64)
    })
    .ok_or("Editor expired")?;
    let class = activity_class(
        env,
        activity,
        "com.machinerhythm.ntx.NtxActivity$ControlsView",
    )?;
    let preview = env.new_object(
        class,
        "(Lcom/machinerhythm/ntx/NtxActivity;J)V",
        &[JValue::Object(activity), JValue::Long(menu as i64)],
    )?;
    tag(env, &preview, "ntx-controls-preview")?;
    let description = env.new_string("Touch control layout preview")?;
    env.call_method(
        &preview,
        "setContentDescription",
        "(Ljava/lang/CharSequence;)V",
        &[JValue::Object(&description)],
    )?;
    let preview_height = (640. * height / width).clamp(220., 480.);
    let params = env.new_object(
        "android/widget/LinearLayout$LayoutParams",
        "(II)V",
        &[
            JValue::Int(-1),
            JValue::Int((preview_height * density).round() as i32),
        ],
    )?;
    env.call_method(
        parent,
        "addView",
        "(Landroid/view/View;Landroid/view/ViewGroup$LayoutParams;)V",
        &[JValue::Object(&preview), JValue::Object(&params)],
    )?;
    let hint = label(
        env,
        activity,
        "Select a control in the preview",
        13.,
        theme::MUTED,
    )?;
    tag(env, &hint, "ntx-controls-hint")?;
    env.call_method(
        parent,
        "addView",
        "(Landroid/view/View;)V",
        &[JValue::Object(&hint)],
    )?;
    for (setting, value, max) in [(0, settings.opacity - 30, 70), (1, settings.dead_zone, 25)] {
        let text = value_label(setting, i32::from(value));
        let heading = label(env, activity, &text, 15., theme::TEXT)?;
        env.call_method(
            parent,
            "addView",
            "(Landroid/view/View;)V",
            &[JValue::Object(&heading)],
        )?;
        let slider = env.new_object(
            "android/widget/SeekBar",
            "(Landroid/content/Context;)V",
            &[JValue::Object(activity)],
        )?;
        env.call_method(&slider, "setMax", "(I)V", &[JValue::Int(max)])?;
        env.call_method(
            &slider,
            "setProgress",
            "(I)V",
            &[JValue::Int(i32::from(value))],
        )?;
        env.call_method(
            &slider,
            "setMinimumHeight",
            "(I)V",
            &[JValue::Int((48. * density) as i32)],
        )?;
        let description = env.new_string(if setting == 0 {
            "Control opacity"
        } else {
            "Stick dead zone"
        })?;
        env.call_method(
            &slider,
            "setContentDescription",
            "(Ljava/lang/CharSequence;)V",
            &[JValue::Object(&description)],
        )?;
        let class = activity_class(
            env,
            activity,
            "com.machinerhythm.ntx.NtxActivity$ControlValue",
        )?;
        let listener = env.new_object(
            class,
            "(Lcom/machinerhythm/ntx/NtxActivity;Landroid/widget/TextView;IJ)V",
            &[
                JValue::Object(activity),
                JValue::Object(&heading),
                JValue::Int(setting),
                JValue::Long(menu as i64),
            ],
        )?;
        env.call_method(
            &slider,
            "setOnSeekBarChangeListener",
            "(Landroid/widget/SeekBar$OnSeekBarChangeListener;)V",
            &[JValue::Object(&listener)],
        )?;
        env.call_method(
            parent,
            "addView",
            "(Landroid/view/View;)V",
            &[JValue::Object(&slider)],
        )?;
    }
    let help = label(
        env,
        activity,
        if n64 {
            "One analog stick, four C buttons, digital L/R and Z. Test inputs never reach your game."
        } else {
            "Slide L/R for analog travel. Their separate click buttons work independently. Test inputs never reach your game."
        },
        13.,
        theme::MUTED,
    )?;
    env.call_method(
        parent,
        "addView",
        "(Landroid/view/View;)V",
        &[JValue::Object(&help)],
    )?;
    Ok(())
}
fn value_label(setting: i32, value: i32) -> String {
    if setting == 0 {
        format!("Control opacity · {}%", value.clamp(0, 70) + 30)
    } else {
        format!("Stick dead zone · {}%", value.clamp(0, 25))
    }
}
pub(super) fn action(env: &mut JNIEnv<'_>, action: i32, menu: u64) -> Result<()> {
    with_editing(menu, |e| match action {
        20 => e.resize(-8.),
        21 => e.resize(8.),
        22 => e.reset_display(),
        23 => e.toggle_test(),
        24 => {
            let settings = e.settings();
            settings.haptics = !settings.haptics;
        }
        25 => {
            if let Editing::GameCube(e) = e {
                e.settings.floating_sticks = !e.settings.floating_sticks;
                e.configure_test();
            }
        }
        _ => {}
    })
    .ok_or("Editor expired")?;
    refresh(env, menu)
}
fn find<'a>(env: &mut JNIEnv<'a>, root: &JObject<'_>, text: &str) -> Result<JObject<'a>> {
    let text = env.new_string(text)?;
    Ok(env
        .call_method(
            root,
            "findViewWithTag",
            "(Ljava/lang/Object;)Landroid/view/View;",
            &[JValue::Object(&text)],
        )?
        .l()?)
}
pub(super) fn refresh(env: &mut JNIEnv<'_>, menu: u64) -> Result<()> {
    let (hint, testing, haptics, following, selected) = with_editor(menu, |e| {
        let selected = e
            .layout
            .zones
            .iter()
            .find(|z| Some(z.target) == e.selected)
            .map(|z| z.label.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Select a control".into());
        let hint = if e.rejected {
            "Keep controls apart and within the preview".into()
        } else if e.testing {
            let p = e.pad();
            format!(
                "Test only · L {:.0}% · R {:.0}% · Stick {:.2}, {:.2} · C {:.2}, {:.2}",
                p.values()[12] * 100.,
                p.values()[13] * 100.,
                p.values()[14],
                p.values()[15],
                p.values()[16],
                p.values()[17]
            )
        } else {
            format!("{selected} · drag to move; size buttons below")
        };
        (
            hint,
            e.testing,
            e.settings.haptics,
            e.settings.floating_sticks,
            e.selected.is_some(),
        )
    })
    .or_else(|| {
        with_n64_editor(menu, |e| {
            let selected = e
                .selected
                .and_then(|target| e.zones.iter().find(|z| z.target == target))
                .map(|z| n64_name(z.target, z.label))
                .unwrap_or("Select a control");
            let hint = if e.rejected {
                "Keep controls apart and within the preview".into()
            } else if e.testing {
                let p = e.pad();
                let held = [
                    ntx::n64::Button::A,
                    ntx::n64::Button::B,
                    ntx::n64::Button::Start,
                    ntx::n64::Button::Z,
                    ntx::n64::Button::L,
                    ntx::n64::Button::R,
                    ntx::n64::Button::Up,
                    ntx::n64::Button::Down,
                    ntx::n64::Button::Left,
                    ntx::n64::Button::Right,
                    ntx::n64::Button::CUp,
                    ntx::n64::Button::CDown,
                    ntx::n64::Button::CLeft,
                    ntx::n64::Button::CRight,
                ]
                .into_iter()
                .filter(|b| p.pressed(*b))
                .map(|b| n64_name(ntx::n64_touch::Target::Button(b), ""))
                .collect::<Vec<_>>()
                .join(" + ");
                format!(
                    "Test only · Stick {:.2}, {:.2} · {}",
                    p.input_state(0, 5, 0, 0) as f32 / 32767.,
                    -(p.input_state(0, 5, 0, 1) as f32) / 32767.,
                    if held.is_empty() { "Released" } else { &held }
                )
            } else {
                format!("{selected} · drag to move; size buttons below")
            };
            (
                hint,
                e.testing,
                e.settings.haptics,
                false,
                e.selected.is_some(),
            )
        })
    })
    .ok_or("Editor expired")?;
    let bound = MENU
        .lock()
        .map_err(|_| "menu lock poisoned")?
        .clone()
        .ok_or("No editor window")?;
    let window = env
        .call_method(bound.as_obj(), "getWindow", "()Landroid/view/Window;", &[])?
        .l()?;
    let root = env
        .call_method(window, "getDecorView", "()Landroid/view/View;", &[])?
        .l()?;
    let preview = find(env, &root, "ntx-controls-preview")?;
    if !preview.is_null() {
        env.call_method(preview, "invalidate", "()V", &[])?;
    }
    let label = find(env, &root, "ntx-controls-hint")?;
    if !label.is_null() {
        set_text(env, &label, &hint)?;
    }
    for action in [20, 21] {
        let view = find(env, &root, &format!("ntx-action-{action}"))?;
        if !view.is_null() {
            env.call_method(
                &view,
                "setAlpha",
                "(F)V",
                &[JValue::Float(if selected && !testing { 1.0 } else { 0.38 })],
            )?;
            env.call_method(
                view,
                "setEnabled",
                "(Z)V",
                &[JValue::Bool(u8::from(selected && !testing))],
            )?;
        }
    }
    for (action, text) in [
        (
            23,
            if testing {
                "Finish input test"
            } else {
                "Test inputs"
            },
        ),
        (
            24,
            if haptics {
                "Haptics · On"
            } else {
                "Haptics · Off"
            },
        ),
        (
            25,
            if following {
                "Following sticks · On"
            } else {
                "Following sticks · Off"
            },
        ),
    ] {
        let view = find(env, &root, &format!("ntx-action-{action}"))?;
        if !view.is_null() {
            set_text(env, &view, text)?;
        }
    }
    Ok(())
}
fn mapping(width: i32, height: i32, w: f32, h: f32) -> (f32, f32, f32) {
    let scale = ((width as f32 - 24.) / w)
        .min((height as f32 - 24.) / h)
        .max(0.001);
    (
        scale,
        (width as f32 - w * scale) / 2.,
        (height as f32 - h * scale) / 2.,
    )
}
#[unsafe(no_mangle)]
extern "system" fn Java_com_machinerhythm_ntx_NtxActivity_controlsValue(
    mut env: JNIEnv<'_>,
    activity: JObject<'_>,
    label: JObject<'_>,
    setting: jint,
    value: jint,
    menu: jlong,
) {
    if !current_activity(&env, &activity) {
        return;
    }
    if with_editing(menu as u64, |e| match setting {
        0 => e.settings().opacity = (value.clamp(0, 70) + 30) as u8,
        1 => {
            e.settings().dead_zone = value.clamp(0, 25) as u8;
            e.configure_test();
        }
        _ => {}
    })
    .is_none()
    {
        return;
    }
    if set_text(&mut env, &label, &value_label(setting, value))
        .and_then(|_| refresh(&mut env, menu as u64))
        .is_err()
    {
        let _ = env.exception_clear();
    }
}
#[unsafe(no_mangle)]
extern "system" fn Java_com_machinerhythm_ntx_NtxActivity_controlsTouch(
    mut env: JNIEnv<'_>,
    activity: JObject<'_>,
    action: jint,
    pointer: jint,
    x: f32,
    y: f32,
    width: jint,
    height: jint,
    menu: jlong,
) -> jni::sys::jboolean {
    if !current_activity(&env, &activity) {
        return 0;
    }
    let handled = with_editing(menu as u64, |e| {
        let (ew, eh) = e.size();
        let (scale, ox, oy) = mapping(width, height, ew, eh);
        let (x, y) = ((x - ox) / scale, (y - oy) / scale);
        match action {
            0 | 5 => {
                if x < 0. || y < 0. || x > ew || y > eh {
                    return false;
                }
                if !e.down(pointer, x, y) {
                    return false;
                }
            }
            2 => e.moved(pointer, x, y),
            1 | 6 => e.up(pointer),
            3 => e.cancel(),
            _ => return false,
        };
        true
    })
    .unwrap_or(false);
    if handled && refresh(&mut env, menu as u64).is_err() {
        let _ = env.exception_clear();
    }
    u8::from(handled)
}
#[unsafe(no_mangle)]
extern "system" fn Java_com_machinerhythm_ntx_NtxActivity_controlsDraw(
    mut env: JNIEnv<'_>,
    activity: JObject<'_>,
    canvas: JObject<'_>,
    width: jint,
    height: jint,
    menu: jlong,
) {
    if !current_activity(&env, &activity) {
        return;
    }
    if let Some(preview) = with_n64_editor(menu as u64, |e| {
        let pad = e.pad();
        N64Preview {
            zones: e.zones.clone(),
            width: e.width,
            height: e.height,
            selected: e.selected,
            pad,
            opacity: e.settings.opacity,
        }
    }) {
        draw_n64(&mut env, &canvas, width, height, preview);
        return;
    }
    let Some((layout, w, h, selected, pad, opacity)) = with_editor(menu as u64, |e| {
        (
            e.layout.clone(),
            e.width,
            e.height,
            e.selected,
            e.pad(),
            e.settings.opacity,
        )
    }) else {
        return;
    };
    let mut canvas_save = None;
    let result: Result<()> = (|| {
        let paint = env.new_object("android/graphics/Paint", "(I)V", &[JValue::Int(1)])?;
        let (scale, ox, oy) = mapping(width, height, w, h);
        let save = env.call_method(&canvas, "save", "()I", &[])?.i()?;
        canvas_save = Some(save);
        env.call_method(
            &canvas,
            "translate",
            "(FF)V",
            &[JValue::Float(ox), JValue::Float(oy)],
        )?;
        env.call_method(
            &canvas,
            "scale",
            "(FF)V",
            &[JValue::Float(scale), JValue::Float(scale)],
        )?;
        rectangle(
            &mut env,
            &canvas,
            &paint,
            Area {
                left: 0.,
                top: 0.,
                right: w,
                bottom: h,
            },
            0xff14191c,
            12.,
        )?;
        rectangle(&mut env, &canvas, &paint, layout.game, 0xff1d252a, 8.)?;
        text(
            &mut env,
            &canvas,
            &paint,
            "Game picture",
            (w / 2. - 42., h * 0.3),
            14.,
            theme::MUTED,
        )?;
        for zone in &layout.zones {
            let a = zone.area;
            let pressed = matches!(zone.target,Target::Button(c) if pad.values()[c as usize]>0.0);
            if Some(zone.target) == selected || pressed {
                rectangle(
                    &mut env,
                    &canvas,
                    &paint,
                    Area {
                        left: a.left - 3.,
                        top: a.top - 3.,
                        right: a.right + 3.,
                        bottom: a.bottom + 3.,
                    },
                    if pressed { theme::TEXT } else { theme::ACCENT },
                    14.,
                )?;
            }
            let color = zone.target.color();
            let color = (color & 0xff00ff00) | ((color & 255) << 16) | ((color >> 16) & 255);
            let color = match zone.target {
                Target::Stick(ntx::Control::CX, _) => 0xffe6cf51,
                Target::Stick(..) => 0xffaaa6a2,
                Target::Pause => 0xff303b40,
                _ => color | 0xff000000,
            };
            let color = ntx::presentation::opacity(color, opacity);
            let mut cap = a;
            if zone.target == Target::Button(ntx::Control::X) {
                let inset = (a.right - a.left) * 0.14;
                cap.left += inset;
                cap.right -= inset;
            }
            if zone.target == Target::Button(ntx::Control::Y) {
                let inset = (a.bottom - a.top) * 0.14;
                cap.top += inset;
                cap.bottom -= inset;
            }
            rectangle(
                &mut env,
                &canvas,
                &paint,
                cap,
                if zone.target == Target::Dpad {
                    0xff20292b
                } else {
                    color
                },
                if matches!(zone.target, Target::Stick(..) | Target::Button(_)) {
                    (cap.right - cap.left).min(cap.bottom - cap.top) / 2.
                } else {
                    10.
                },
            )?;
            if zone.target == Target::Dpad {
                let step = (a.right - a.left) / 3.0;
                for r in [
                    Area {
                        left: a.left + step,
                        top: a.top,
                        right: a.right - step,
                        bottom: a.bottom,
                    },
                    Area {
                        left: a.left,
                        top: a.top + step,
                        right: a.right,
                        bottom: a.bottom - step,
                    },
                ] {
                    rectangle(&mut env, &canvas, &paint, r, color, 4.0)?;
                }
                for (control, x, y) in [
                    (ntx::Control::Up, a.left + step, a.top),
                    (ntx::Control::Down, a.left + step, a.bottom - step),
                    (ntx::Control::Left, a.left, a.top + step),
                    (ntx::Control::Right, a.right - step, a.top + step),
                ] {
                    if pad.values()[control as usize] > 0.0 {
                        rectangle(
                            &mut env,
                            &canvas,
                            &paint,
                            Area {
                                left: x,
                                top: y,
                                right: x + step,
                                bottom: y + step,
                            },
                            theme::TEXT,
                            4.0,
                        )?;
                    }
                }
                continue;
            }
            if let Target::Stick(cx, cy) = zone.target {
                let (x, y) = a.center();
                let radius = (a.right - a.left).min(a.bottom - a.top) * 0.5;
                let x = x + pad.values()[cx as usize] as f32 * radius * 0.64;
                let y = y - pad.values()[cy as usize] as f32 * radius * 0.64;
                let knob = radius * 0.36;
                rectangle(
                    &mut env,
                    &canvas,
                    &paint,
                    Area {
                        left: x - knob,
                        top: y - knob,
                        right: x + knob,
                        bottom: y + knob,
                    },
                    if cx == ntx::Control::CX {
                        0xffe6cf51
                    } else {
                        0xffcbc9be
                    },
                    knob,
                )?;
            }
            if let Target::Travel(c) = zone.target {
                rectangle(
                    &mut env,
                    &canvas,
                    &paint,
                    Area {
                        left: a.left,
                        top: a.bottom - 6.,
                        right: a.left + (a.right - a.left) * pad.values()[c as usize] as f32,
                        bottom: a.bottom,
                    },
                    theme::ACCENT,
                    3.,
                )?;
            }
            let name = zone.label.to_string_lossy();
            let (cx, cy) = a.center();
            let font = if name.len() > 4 { 11. } else { 16. };
            text(
                &mut env,
                &canvas,
                &paint,
                &name,
                (cx - name.len() as f32 * font * 0.25, cy + font * 0.35),
                font,
                if matches!(zone.target, Target::Pause) {
                    theme::TEXT
                } else {
                    0xff13231d
                },
            )?;
        }
        env.call_method(&canvas, "restoreToCount", "(I)V", &[JValue::Int(save)])?;
        canvas_save = None;
        Ok(())
    })();
    if result.is_err() {
        let _ = env.exception_clear();
    }
    if let Some(save) = canvas_save {
        let _ = env.call_method(&canvas, "restoreToCount", "(I)V", &[JValue::Int(save)]);
        let _ = env.exception_clear();
    }
}
fn n64_name(target: ntx::n64_touch::Target, _label: &str) -> &'static str {
    use ntx::{n64::Button as B, n64_touch::Target as T};
    match target {
        T::Stick => "Analog stick",
        T::Pause => "Pause",
        T::Button(b) => match b {
            B::A => "A",
            B::B => "B",
            B::Start => "Start",
            B::Z => "Z",
            B::L => "L",
            B::R => "R",
            B::Up => "D-pad ↑",
            B::Down => "D-pad ↓",
            B::Left => "D-pad ←",
            B::Right => "D-pad →",
            B::CUp => "C ↑",
            B::CDown => "C ↓",
            B::CLeft => "C ←",
            B::CRight => "C →",
        },
    }
}
struct N64Preview {
    zones: Vec<ntx::n64_touch::Zone>,
    width: f32,
    height: f32,
    selected: Option<ntx::n64_touch::Target>,
    pad: ntx::n64::Pad,
    opacity: u8,
}
fn draw_n64(
    env: &mut JNIEnv<'_>,
    canvas: &JObject<'_>,
    width: i32,
    height: i32,
    preview: N64Preview,
) {
    let N64Preview {
        zones,
        width: w,
        height: h,
        selected,
        pad,
        opacity,
    } = preview;
    use ntx::{n64::Button as B, n64_touch::Target as T};
    let mut canvas_save = None;
    let result: Result<()> = (|| {
        let paint = env.new_object("android/graphics/Paint", "(I)V", &[JValue::Int(1)])?;
        let (scale, ox, oy) = mapping(width, height, w, h);
        let save = env.call_method(canvas, "save", "()I", &[])?.i()?;
        canvas_save = Some(save);
        env.call_method(
            canvas,
            "translate",
            "(FF)V",
            &[JValue::Float(ox), JValue::Float(oy)],
        )?;
        env.call_method(
            canvas,
            "scale",
            "(FF)V",
            &[JValue::Float(scale), JValue::Float(scale)],
        )?;
        rectangle(
            env,
            canvas,
            &paint,
            Area {
                left: 0.,
                top: 0.,
                right: w,
                bottom: h,
            },
            0xff14191c,
            12.,
        )?;
        let size = (w / 4.).min(h / 3.);
        rectangle(
            env,
            canvas,
            &paint,
            Area {
                left: (w - 4. * size) / 2.,
                top: (h - 3. * size) / 2.,
                right: (w + 4. * size) / 2.,
                bottom: (h + 3. * size) / 2.,
            },
            0xff1d252a,
            8.,
        )?;
        text(
            env,
            canvas,
            &paint,
            "Game picture",
            (w / 2. - 42., h * 0.3),
            14.,
            theme::MUTED,
        )?;
        for z in zones {
            let a = z.area;
            let pressed = matches!(z.target,T::Button(b) if pad.pressed(b));
            if Some(z.target) == selected || pressed {
                rectangle(
                    env,
                    canvas,
                    &paint,
                    Area {
                        left: a.left - 3.,
                        top: a.top - 3.,
                        right: a.right + 3.,
                        bottom: a.bottom + 3.,
                    },
                    if pressed { theme::TEXT } else { theme::ACCENT },
                    14.,
                )?;
            }
            let color = match z.target {
                T::Button(B::A) => 0xff3984e8,
                T::Button(B::B) => 0xff27ad66,
                T::Button(B::CUp | B::CDown | B::CLeft | B::CRight) => 0xffe6c44c,
                T::Pause => 0xff303b40,
                _ => 0xffaaa6a2,
            };
            let radius = (a.right - a.left).min(a.bottom - a.top) / 2.;
            rectangle(
                env,
                canvas,
                &paint,
                a,
                ntx::presentation::opacity(color, opacity),
                radius,
            )?;
            let (x, y) = a.center();
            if z.target == T::Stick {
                let k = radius * 0.36;
                let x = x + pad.input_state(0, 5, 0, 0) as f32 / 32767. * radius * 0.64;
                let y = y + pad.input_state(0, 5, 0, 1) as f32 / 32767. * radius * 0.64;
                rectangle(
                    env,
                    canvas,
                    &paint,
                    Area {
                        left: x - k,
                        top: y - k,
                        right: x + k,
                        bottom: y + k,
                    },
                    0xffcbc9be,
                    k,
                )?;
            } else {
                let label = match z.target {
                    T::Button(B::Up | B::Down | B::Left | B::Right) => z.label,
                    _ => n64_name(z.target, z.label),
                };
                let font = if label.chars().count() > 4 { 9. } else { 12. };
                text(
                    env,
                    canvas,
                    &paint,
                    label,
                    (
                        x - label.chars().count() as f32 * font * 0.25,
                        y + font * 0.35,
                    ),
                    font,
                    if z.target == T::Pause {
                        theme::TEXT
                    } else {
                        0xff13231d
                    },
                )?;
            }
        }
        env.call_method(canvas, "restoreToCount", "(I)V", &[JValue::Int(save)])?;
        canvas_save = None;
        Ok(())
    })();
    if result.is_err() {
        let _ = env.exception_clear();
    }
    if let Some(save) = canvas_save {
        let _ = env.call_method(canvas, "restoreToCount", "(I)V", &[JValue::Int(save)]);
        let _ = env.exception_clear();
    }
}
fn rectangle(
    env: &mut JNIEnv<'_>,
    canvas: &JObject<'_>,
    paint: &JObject<'_>,
    a: Area,
    color: u32,
    radius: f32,
) -> Result<()> {
    env.call_method(paint, "setColor", "(I)V", &[JValue::Int(color as i32)])?;
    env.call_method(
        canvas,
        "drawRoundRect",
        "(FFFFFFLandroid/graphics/Paint;)V",
        &[
            JValue::Float(a.left),
            JValue::Float(a.top),
            JValue::Float(a.right),
            JValue::Float(a.bottom),
            JValue::Float(radius),
            JValue::Float(radius),
            JValue::Object(paint),
        ],
    )?;
    Ok(())
}
fn text(
    env: &mut JNIEnv<'_>,
    canvas: &JObject<'_>,
    paint: &JObject<'_>,
    text: &str,
    position: (f32, f32),
    size: f32,
    color: u32,
) -> Result<()> {
    env.call_method(paint, "setColor", "(I)V", &[JValue::Int(color as i32)])?;
    env.call_method(paint, "setTextSize", "(F)V", &[JValue::Float(size)])?;
    let text = env.new_string(text)?;
    env.call_method(
        canvas,
        "drawText",
        "(Ljava/lang/String;FFLandroid/graphics/Paint;)V",
        &[
            JValue::Object(&text),
            JValue::Float(position.0),
            JValue::Float(position.1),
            JValue::Object(paint),
        ],
    )?;
    Ok(())
}
