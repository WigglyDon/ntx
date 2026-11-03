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
    bitmap: Option<GlobalRef>,
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
    fn draft(&mut self) -> &mut ntx::layout_edit::Draft {
        match self {
            Self::GameCube(e) => &mut e.draft,
            Self::Nintendo64(e) => &mut e.draft,
        }
    }
    fn sync(&mut self) {
        match self {
            Self::GameCube(e) => e.sync(),
            Self::Nintendo64(e) => e.sync(),
        }
    }
    fn testing(&self) -> bool {
        match self {
            Self::GameCube(e) => e.testing,
            Self::Nintendo64(e) => e.testing,
        }
    }
    fn mirror(&mut self) {
        use ntx::{Control as C, n64::Button as B, n64_touch::Target as N, touch::Target as G};
        let partner = match self {
            Self::GameCube(e) => {
                let target = match e.selected {
                    Some(G::Stick(C::StickX, _)) => Some(G::Stick(C::CX, C::CY)),
                    Some(G::Stick(C::CX, _)) => Some(G::Stick(C::StickX, C::StickY)),
                    Some(G::Travel(C::LTravel)) => Some(G::Travel(C::RTravel)),
                    Some(G::Travel(C::RTravel)) => Some(G::Travel(C::LTravel)),
                    Some(G::Button(C::LClick)) => Some(G::Button(C::RClick)),
                    Some(G::Button(C::RClick)) => Some(G::Button(C::LClick)),
                    _ => None,
                };
                target.and_then(|t| e.layout.zones.iter().position(|z| z.target == t))
            }
            Self::Nintendo64(e) => {
                let target = match e.selected {
                    Some(N::Button(B::L)) => Some(N::Button(B::R)),
                    Some(N::Button(B::R)) => Some(N::Button(B::L)),
                    _ => None,
                };
                target.and_then(|t| e.zones.iter().position(|z| z.target == t))
            }
        };
        self.draft().mirror(partner);
        self.sync();
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
            bitmap: None,
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
        Editing::GameCube(e) => (!e.draft.requires_choice())
            .then(|| e.finish().ok())
            .flatten(),
        Editing::Nintendo64(e) => (!e.draft.requires_choice())
            .then(|| e.finish().ok())
            .flatten(),
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
fn add(env: &mut JNIEnv<'_>, parent: &JObject<'_>, child: &JObject<'_>) -> Result<()> {
    env.call_method(
        parent,
        "addView",
        "(Landroid/view/View;)V",
        &[JValue::Object(child)],
    )?;
    Ok(())
}
fn linear<'a>(env: &mut JNIEnv<'a>, activity: &JObject<'_>, vertical: bool) -> Result<JObject<'a>> {
    let v = env.new_object(
        "android/widget/LinearLayout",
        "(Landroid/content/Context;)V",
        &[JValue::Object(activity)],
    )?;
    env.call_method(
        &v,
        "setOrientation",
        "(I)V",
        &[JValue::Int(i32::from(vertical))],
    )?;
    Ok(v)
}
fn button(
    env: &mut JNIEnv<'_>,
    activity: &JObject<'_>,
    parent: &JObject<'_>,
    menu: u64,
    runtime: u64,
    action: (i32, &str),
    density: f32,
) -> Result<()> {
    let (id, text) = action;
    let b = env.new_object(
        "android/widget/Button",
        "(Landroid/content/Context;)V",
        &[JValue::Object(activity)],
    )?;
    set_text(env, &b, text)?;
    tag(env, &b, &format!("ntx-action-{id}"))?;
    env.call_method(&b, "setAllCaps", "(Z)V", &[JValue::Bool(0)])?;
    env.call_method(&b, "setTextSize", "(F)V", &[JValue::Float(13.)])?;
    env.call_method(
        &b,
        "setMinimumHeight",
        "(I)V",
        &[JValue::Int((48. * density).round() as i32)],
    )?;
    let primary = id == 10;
    menu_background(
        env,
        &b,
        if primary {
            theme::ACCENT
        } else {
            theme::SURFACE
        },
        density,
    )?;
    env.call_method(
        &b,
        "setTextColor",
        "(I)V",
        &[JValue::Int(
            if primary { theme::INK } else { theme::TEXT } as i32
        )],
    )?;
    let params = env.new_object(
        "android/widget/LinearLayout$LayoutParams",
        "(II)V",
        &[JValue::Int(-2), JValue::Int(-2)],
    )?;
    let gap = (3. * density).round() as i32;
    env.call_method(
        &params,
        "setMargins",
        "(IIII)V",
        &[
            JValue::Int(gap),
            JValue::Int(gap),
            JValue::Int(gap),
            JValue::Int(gap),
        ],
    )?;
    env.call_method(
        &b,
        "setLayoutParams",
        "(Landroid/view/ViewGroup$LayoutParams;)V",
        &[JValue::Object(&params)],
    )?;
    let class = activity_class(env, activity, "com.machinerhythm.ntx.NtxActivity$Action")?;
    let listener = env.new_object(
        class,
        "(Lcom/machinerhythm/ntx/NtxActivity;IJJ)V",
        &[
            JValue::Object(activity),
            JValue::Int(id),
            JValue::Long(menu as i64),
            JValue::Long(runtime as i64),
        ],
    )?;
    env.call_method(
        &b,
        "setOnClickListener",
        "(Landroid/view/View$OnClickListener;)V",
        &[JValue::Object(&listener)],
    )?;
    add(env, parent, &b)
}
pub(super) fn build_dialog(
    env: &mut JNIEnv<'_>,
    activity: &JObject<'_>,
    menu: u64,
    runtime: u64,
) -> Result<GlobalRef> {
    let geometry = measure(env, activity)?;
    let safe = geometry.control_bounds().ok_or("No safe display")?;
    let density = geometry.density;
    with_editing(menu, |e| {
        let frame = Area {
            left: (geometry.surface.left - safe.left) / density,
            top: (geometry.surface.top - safe.top) / density,
            right: (geometry.surface.right - safe.left) / density,
            bottom: (geometry.surface.bottom - safe.top) / density,
        };
        let aspect = last_frame(runtime).map_or(4. / 3., |f| f.aspect);
        match e {
            Editing::GameCube(e) => e.configure_frame(frame, aspect),
            Editing::Nintendo64(e) => e.configure_frame(frame, aspect),
        }
    })
    .ok_or("Editor expired")?;
    let content = env.new_object(
        "android/widget/FrameLayout",
        "(Landroid/content/Context;)V",
        &[JValue::Object(activity)],
    )?;
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
    let desc = env.new_string("Full size game field and controller layout editor")?;
    env.call_method(
        &preview,
        "setContentDescription",
        "(Ljava/lang/CharSequence;)V",
        &[JValue::Object(&desc)],
    )?;
    let params = env.new_object(
        "android/widget/FrameLayout$LayoutParams",
        "(II)V",
        &[JValue::Int(-1), JValue::Int(-1)],
    )?;
    env.call_method(
        &content,
        "addView",
        "(Landroid/view/View;Landroid/view/ViewGroup$LayoutParams;)V",
        &[JValue::Object(&preview), JValue::Object(&params)],
    )?;
    let dock = linear(env, activity, true)?;
    tag(env, &dock, "ntx-editor-dock")?;
    env.call_method(
        &dock,
        "setBackgroundColor",
        "(I)V",
        &[JValue::Int(0xee14191cu32 as i32)],
    )?;
    let hint = label(
        env,
        activity,
        "Game field and controls · select, move or resize",
        13.,
        theme::TEXT,
    )?;
    tag(env, &hint, "ntx-controls-hint")?;
    add(env, &dock, &hint)?;
    let scroll = env.new_object(
        "android/widget/HorizontalScrollView",
        "(Landroid/content/Context;)V",
        &[JValue::Object(activity)],
    )?;
    let row = linear(env, activity, false)?;
    for (id, text) in [
        (10, "Done"),
        (11, "Cancel"),
        (26, "Game field"),
        (38, "Fill game field"),
        (27, "Move"),
        (28, "Resize"),
        (29, "Undo"),
        (30, "Grid"),
        (31, "Group"),
        (32, "Mirror"),
        (33, "Reset selected"),
        (39, "Fit game around controls"),
        (40, "Allow overlap"),
        (36, "Dock bottom"),
        (37, "Options"),
    ] {
        button(env, activity, &row, menu, runtime, (id, text), density)?;
    }
    add(env, &scroll, &row)?;
    add(env, &dock, &scroll)?;
    let options = linear(env, activity, true)?;
    tag(env, &options, "ntx-editor-options")?;
    let option_scroll = env.new_object(
        "android/widget/HorizontalScrollView",
        "(Landroid/content/Context;)V",
        &[JValue::Object(activity)],
    )?;
    let option_row = linear(env, activity, false)?;
    for (id, text) in [
        (20, "Size −"),
        (21, "Size +"),
        (34, "Spacing −"),
        (35, "Spacing +"),
        (23, "Test inputs"),
        (24, "Haptics"),
        (22, "Reset screen"),
    ] {
        button(
            env,
            activity,
            &option_row,
            menu,
            runtime,
            (id, text),
            density,
        )?;
    }
    if with_editing(menu, |e| matches!(e, Editing::GameCube(_))).unwrap_or(false) {
        button(
            env,
            activity,
            &option_row,
            menu,
            runtime,
            (25, "Following sticks"),
            density,
        )?;
    }
    add(env, &option_scroll, &option_row)?;
    add(env, &options, &option_scroll)?;
    let settings = with_editing(menu, |e| e.settings().clone()).ok_or("Editor expired")?;
    append_sliders(env, activity, &options, density, menu, &settings)?;
    env.call_method(&options, "setVisibility", "(I)V", &[JValue::Int(8)])?;
    add(env, &dock, &options)?;
    let params = env.new_object(
        "android/widget/FrameLayout$LayoutParams",
        "(III)V",
        &[JValue::Int(-1), JValue::Int(-2), JValue::Int(48)],
    )?;
    let dp = |v: f32| (v * density).round() as i32;
    env.call_method(
        &params,
        "setMargins",
        "(IIII)V",
        &[
            JValue::Int(dp((-geometry.surface.left + safe.left) / density)),
            JValue::Int(dp((-geometry.surface.top + safe.top) / density)),
            JValue::Int(dp((geometry.surface.right - safe.right) / density)),
            JValue::Int(dp((geometry.surface.bottom - safe.bottom) / density)),
        ],
    )?;
    env.call_method(
        &content,
        "addView",
        "(Landroid/view/View;Landroid/view/ViewGroup$LayoutParams;)V",
        &[JValue::Object(&dock), JValue::Object(&params)],
    )?;
    let dialog = env.new_object(
        "android/app/Dialog",
        "(Landroid/content/Context;)V",
        &[JValue::Object(activity)],
    )?;
    env.call_method(&dialog, "requestWindowFeature", "(I)Z", &[JValue::Int(1)])?;
    env.call_method(&dialog, "setCancelable", "(Z)V", &[JValue::Bool(0)])?;
    env.call_method(
        &dialog,
        "setContentView",
        "(Landroid/view/View;)V",
        &[JValue::Object(&content)],
    )?;
    let window = env
        .call_method(&dialog, "getWindow", "()Landroid/view/Window;", &[])?
        .l()?;
    let bg = env.new_object(
        "android/graphics/drawable/ColorDrawable",
        "(I)V",
        &[JValue::Int(theme::BACKGROUND as i32)],
    )?;
    env.call_method(
        &window,
        "setBackgroundDrawable",
        "(Landroid/graphics/drawable/Drawable;)V",
        &[JValue::Object(&bg)],
    )?;
    env.call_method(&window, "clearFlags", "(I)V", &[JValue::Int(2)])?;
    env.call_method(&window, "addFlags", "(I)V", &[JValue::Int(0x80 | 0x400)])?;
    let decor = env
        .call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?
        .l()?;
    env.call_method(
        &decor,
        "setSystemUiVisibility",
        "(I)V",
        &[JValue::Int(0x1706)],
    )?;
    if sdk(env)? >= 28 {
        let parent = env
            .call_method(activity, "getWindow", "()Landroid/view/Window;", &[])?
            .l()?;
        let source = env
            .call_method(
                parent,
                "getAttributes",
                "()Landroid/view/WindowManager$LayoutParams;",
                &[],
            )?
            .l()?;
        let cutout = env
            .get_field(source, "layoutInDisplayCutoutMode", "I")?
            .i()?;
        let attrs = env
            .call_method(
                &window,
                "getAttributes",
                "()Landroid/view/WindowManager$LayoutParams;",
                &[],
            )?
            .l()?;
        env.set_field(
            &attrs,
            "layoutInDisplayCutoutMode",
            "I",
            JValue::Int(cutout),
        )?;
        env.call_method(
            &window,
            "setAttributes",
            "(Landroid/view/WindowManager$LayoutParams;)V",
            &[JValue::Object(&attrs)],
        )?;
    }
    if sdk(env)? >= 30 {
        env.call_method(
            &window,
            "setDecorFitsSystemWindows",
            "(Z)V",
            &[JValue::Bool(0)],
        )?;
    }
    env.call_method(&dialog, "show", "()V", &[])?;
    env.call_method(
        &window,
        "setLayout",
        "(II)V",
        &[JValue::Int(-1), JValue::Int(-1)],
    )?;
    request_frame(env, activity, menu, runtime, geometry)?;
    Ok(env.new_global_ref(dialog)?)
}
fn append_sliders(
    env: &mut JNIEnv<'_>,
    activity: &JObject<'_>,
    parent: &JObject<'_>,
    density: f32,
    menu: u64,
    settings: &Settings,
) -> Result<()> {
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
    if matches!(action, 36 | 37) {
        let bound = MENU
            .lock()
            .map_err(|_| "Menu lock poisoned")?
            .clone()
            .ok_or("No editor")?;
        let window = env
            .call_method(bound.as_obj(), "getWindow", "()Landroid/view/Window;", &[])?
            .l()?;
        let root = env
            .call_method(window, "getDecorView", "()Landroid/view/View;", &[])?
            .l()?;
        if action == 36 {
            let dock = find(env, &root, "ntx-editor-dock")?;
            let params = env
                .call_method(
                    &dock,
                    "getLayoutParams",
                    "()Landroid/view/ViewGroup$LayoutParams;",
                    &[],
                )?
                .l()?;
            let top = env.get_field(&params, "gravity", "I")?.i()? == 48;
            env.set_field(
                &params,
                "gravity",
                "I",
                JValue::Int(if top { 80 } else { 48 }),
            )?;
            env.call_method(
                &dock,
                "setLayoutParams",
                "(Landroid/view/ViewGroup$LayoutParams;)V",
                &[JValue::Object(&params)],
            )?;
            let button = find(env, &root, "ntx-action-36")?;
            set_text(env, &button, if top { "Dock top" } else { "Dock bottom" })?;
        } else {
            let panel = find(env, &root, "ntx-editor-options")?;
            let visible = env.call_method(&panel, "getVisibility", "()I", &[])?.i()? == 0;
            env.call_method(
                &panel,
                "setVisibility",
                "(I)V",
                &[JValue::Int(if visible { 8 } else { 0 })],
            )?;
        }
        return Ok(());
    }
    if with_editing(menu, |e| e.testing()).unwrap_or(false) && !matches!(action, 23..=25) {
        return Ok(());
    }
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
        26 => {
            e.cancel();
            e.draft().selected = Some(ntx::layout_edit::Selection::Picture);
            e.sync();
        }
        27 | 28 => {
            e.cancel();
            e.draft().mode = if action == 27 {
                ntx::layout_edit::Mode::Move
            } else {
                ntx::layout_edit::Mode::Resize
            };
        }
        29 => {
            e.draft().undo();
            e.sync();
        }
        30 => {
            e.cancel();
            let d = e.draft();
            d.grid = !d.grid;
        }
        31 => {
            e.cancel();
            let d = e.draft();
            d.grouped = !d.grouped;
        }
        32 => e.mirror(),
        33 => {
            e.draft().reset_selected();
            e.sync();
        }
        38 => {
            e.draft().fill_picture();
            e.sync();
        }
        39 | 40 => {
            e.draft().set_picture_policy(if action == 39 {
                ntx::layout_edit::PicturePolicy::KeepClear
            } else {
                ntx::layout_edit::PicturePolicy::Overlay
            });
            e.sync();
        }
        34 | 35 => {
            e.draft().spacing(if action == 34 { -8. } else { 8. });
            e.sync();
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
            "Keep the game field and controls apart and within the screen".into()
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
            format!("{selected} · drag to move; Resize shows corner grips")
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
                "Keep the game field and controls apart and within the screen".into()
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
                format!("{selected} · drag to move; Resize shows corner grips")
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
    let (hint, selected, dragging, mode, grid, grouped, undo) = with_editing(menu, |e| {
        let testing = e.testing();
        let d = e.draft();
        let hint = if !testing && d.selected == Some(ntx::layout_edit::Selection::Picture) {
            if d.rejected {
                "Keep the game field clear of controls and inside the screen".into()
            } else {
                if d.picture_policy().is_game_first() {
                    "Game field · full drawable; aspect intact; controls use free space".into()
                } else if d.picture_policy().is_overlay() {
                    "Game field · full drawable; aspect intact; overlap allowed".into()
                } else {
                    "Game field · kept clear of controls; aspect intact".into()
                }
            }
        } else {
            hint
        };
        (
            hint,
            selected || d.selected.is_some(),
            d.dragging(),
            d.mode,
            d.grid,
            d.grouped,
            d.can_undo(),
        )
    })
    .ok_or("Editor expired")?;
    let needs_choice =
        with_editing(menu, |e| e.draft().requires_choice()).ok_or("Editor expired")?;
    let hint = if needs_choice {
        "No spare control space: choose Fit game around controls or Allow overlap.".to_owned()
    } else {
        hint
    };
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
    let dock = find(env, &root, "ntx-editor-dock")?;
    env.call_method(
        dock,
        "setVisibility",
        "(I)V",
        &[JValue::Int(if dragging { 4 } else { 0 })],
    )?;
    for (id, title) in [
        (
            27,
            if mode == ntx::layout_edit::Mode::Move {
                "Move ✓"
            } else {
                "Move"
            },
        ),
        (
            28,
            if mode == ntx::layout_edit::Mode::Resize {
                "Resize ✓"
            } else {
                "Resize"
            },
        ),
        (30, if grid { "Grid · On" } else { "Grid · Off" }),
        (31, if grouped { "Group" } else { "Single" }),
    ] {
        let b = find(env, &root, &format!("ntx-action-{id}"))?;
        if !b.is_null() {
            set_text(env, &b, title)?;
        }
    }
    let policy = with_editing(menu, |e| e.draft().picture_policy()).ok_or("Editor expired")?;
    let b = find(env, &root, "ntx-action-39")?;
    set_text(
        env,
        &b,
        if policy == ntx::layout_edit::PicturePolicy::KeepClear {
            "Fit game around controls ✓"
        } else {
            "Fit game around controls"
        },
    )?;
    let b = find(env, &root, "ntx-action-40")?;
    set_text(
        env,
        &b,
        if policy.is_overlay() {
            "Allow overlap ✓"
        } else {
            "Allow overlap"
        },
    )?;
    let b = find(env, &root, "ntx-action-29")?;
    env.call_method(
        b,
        "setEnabled",
        "(Z)V",
        &[JValue::Bool(u8::from(undo && !testing))],
    )?;
    let done = find(env, &root, "ntx-action-10")?;
    env.call_method(
        done,
        "setEnabled",
        "(Z)V",
        &[JValue::Bool(u8::from(!needs_choice))],
    )?;
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
fn mapping(width: i32, height: i32, frame: Area) -> (f32, f32, f32) {
    let scale = (width as f32 / (frame.right - frame.left))
        .min(height as f32 / (frame.bottom - frame.top))
        .max(0.001);
    (scale, -frame.left * scale, -frame.top * scale)
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
        let (scale, ox, oy) = mapping(width, height, e.draft().frame);
        let (x, y) = ((x - ox) / scale, (y - oy) / scale);
        match action {
            0 | 5 => {
                if !e.draft().frame.contains(x, y) {
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
            selected: e.selected,
            pad,
            opacity: e.settings.opacity,
            decor: Decor::new(&e.draft),
        }
    }) {
        draw_n64(&mut env, &canvas, width, height, menu as u64, preview);
        return;
    }
    let Some((layout, selected, pad, opacity, decor)) = with_editor(menu as u64, |e| {
        (
            e.layout.clone(),
            e.selected,
            e.pad(),
            e.settings.opacity,
            Decor::new(&e.draft),
        )
    }) else {
        return;
    };
    let mut canvas_save = None;
    let result: Result<()> = (|| {
        let paint = env.new_object("android/graphics/Paint", "(I)V", &[JValue::Int(1)])?;
        let (scale, ox, oy) = mapping(width, height, decor.frame);
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
        draw_field(&mut env, &canvas, &paint, &decor, menu as u64)?;
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
        draw_selection(&mut env, &canvas, &paint, &decor)?;
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
    selected: Option<ntx::n64_touch::Target>,
    pad: ntx::n64::Pad,
    opacity: u8,
    decor: Decor,
}
fn draw_n64(
    env: &mut JNIEnv<'_>,
    canvas: &JObject<'_>,
    width: i32,
    height: i32,
    menu: u64,
    preview: N64Preview,
) {
    let N64Preview {
        zones,
        selected,
        pad,
        opacity,
        decor,
    } = preview;
    use ntx::{n64::Button as B, n64_touch::Target as T};
    let mut canvas_save = None;
    let result: Result<()> = (|| {
        let paint = env.new_object("android/graphics/Paint", "(I)V", &[JValue::Int(1)])?;
        let (scale, ox, oy) = mapping(width, height, decor.frame);
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
        draw_field(env, canvas, &paint, &decor, menu)?;
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
        draw_selection(env, canvas, &paint, &decor)?;
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

#[derive(Clone, Copy)]
struct Frame {
    runtime: u64,
    activity: u64,
    picture: Area,
    width: f32,
    height: f32,
    aspect: f32,
}
static FRAME: Mutex<Option<Frame>> = Mutex::new(None);
pub(super) fn record_frame(picture: Area, width: f32, height: f32, aspect: f32) {
    if let Ok(mut slot) = FRAME.lock() {
        let frame = Frame {
            runtime: TRACE_RUNTIME.load(Ordering::Relaxed),
            activity: EPOCH.load(Ordering::Acquire),
            picture,
            width,
            height,
            aspect,
        };
        let changed = slot.is_none_or(|f| {
            f.runtime != frame.runtime
                || f.activity != frame.activity
                || f.width != width
                || f.height != height
                || f.aspect != aspect
                || !ntx::layout_edit::same_area(f.picture, picture)
        });
        *slot = Some(frame);
        if changed {
            trace_presentation(&format!(
                "game_field buffer={width}x{height} aspect={aspect} bounds={picture:?}"
            ));
        }
    }
}
fn last_frame(runtime: u64) -> Option<Frame> {
    FRAME.lock().ok()?.filter(|f| {
        runtime != 0 && f.runtime == runtime && f.activity == EPOCH.load(Ordering::Acquire)
    })
}
fn request_frame(
    env: &mut JNIEnv<'_>,
    activity: &JObject<'_>,
    menu: u64,
    runtime: u64,
    g: ntx::viewport::WindowGeometry,
) -> Result<()> {
    if with_editing(menu, |e| e.draft().picture_policy().is_overlay()).unwrap_or(false) {
        return Ok(());
    }
    let Some(f) = last_frame(runtime) else {
        return Ok(());
    };
    if f.width < 2. || f.height < 2. {
        return Ok(());
    }
    let sx = (g.surface.right - g.surface.left) / f.width;
    let sy = (g.surface.bottom - g.surface.top) / f.height;
    let p = f.picture;
    let (l, t, r, b) = (
        (g.surface.left - g.window.left + p.left * sx).round() as i32,
        (g.surface.top - g.window.top + p.top * sy).round() as i32,
        (g.surface.left - g.window.left + p.right * sx).round() as i32,
        (g.surface.top - g.window.top + p.bottom * sy).round() as i32,
    );
    if r - l < 2 || b - t < 2 || r - l > 4096 || b - t > 4096 {
        return Ok(());
    }
    let result: Result<()> = (|| {
        let config = env
            .get_static_field(
                "android/graphics/Bitmap$Config",
                "ARGB_8888",
                "Landroid/graphics/Bitmap$Config;",
            )?
            .l()?;
        let bitmap = env
            .call_static_method(
                "android/graphics/Bitmap",
                "createBitmap",
                "(IILandroid/graphics/Bitmap$Config;)Landroid/graphics/Bitmap;",
                &[
                    JValue::Int(r - l),
                    JValue::Int(b - t),
                    JValue::Object(&config),
                ],
            )?
            .l()?;
        let rect = env.new_object(
            "android/graphics/Rect",
            "(IIII)V",
            &[
                JValue::Int(l),
                JValue::Int(t),
                JValue::Int(r),
                JValue::Int(b),
            ],
        )?;
        let class = activity_class(env, activity, "com.machinerhythm.ntx.NtxActivity$FrameCopy")?;
        let listener = env.new_object(
            class,
            "(Lcom/machinerhythm/ntx/NtxActivity;Landroid/graphics/Bitmap;J)V",
            &[
                JValue::Object(activity),
                JValue::Object(&bitmap),
                JValue::Long(menu as i64),
            ],
        )?;
        let looper = env
            .call_static_method(
                "android/os/Looper",
                "getMainLooper",
                "()Landroid/os/Looper;",
                &[],
            )?
            .l()?;
        let handler = env.new_object(
            "android/os/Handler",
            "(Landroid/os/Looper;)V",
            &[JValue::Object(&looper)],
        )?;
        let window = env
            .call_method(activity, "getWindow", "()Landroid/view/Window;", &[])?
            .l()?;
        env.call_static_method("android/view/PixelCopy","request","(Landroid/view/Window;Landroid/graphics/Rect;Landroid/graphics/Bitmap;Landroid/view/PixelCopy$OnPixelCopyFinishedListener;Landroid/os/Handler;)V",
            &[JValue::Object(&window),JValue::Object(&rect),JValue::Object(&bitmap),JValue::Object(&listener),JValue::Object(&handler)])?;
        Ok(())
    })();
    if result.is_err() {
        env.exception_clear()?;
    }
    Ok(()) // A missing paused frame does not prevent editing or alter the runtime.
}
#[unsafe(no_mangle)]
extern "system" fn Java_com_machinerhythm_ntx_NtxActivity_controlsFrame(
    mut env: JNIEnv<'_>,
    activity: JObject<'_>,
    bitmap: JObject<'_>,
    result: jint,
    menu: jlong,
) {
    if result != 0 || !current_activity(&env, &activity) || !menu_current(menu as u64) {
        return;
    }
    let Ok(bitmap) = env.new_global_ref(bitmap) else {
        let _ = env.exception_clear();
        return;
    };
    if let Ok(mut slot) = EDITOR.lock()
        && let Some(s) = slot.as_mut()
        && s.menu == menu as u64
        && s.activity == EPOCH.load(Ordering::Acquire)
    {
        s.bitmap = Some(bitmap);
    }
    if refresh(&mut env, menu as u64).is_err() {
        let _ = env.exception_clear();
    }
}
struct Decor {
    frame: Area,
    picture: Area,
    bounds: Option<Area>,
    resize: bool,
    grid: bool,
    guides: (Option<f32>, Option<f32>),
}
impl Decor {
    fn new(d: &ntx::layout_edit::Draft) -> Self {
        Self {
            frame: d.frame,
            picture: d.picture(),
            bounds: d.selection_bounds(),
            resize: d.mode == ntx::layout_edit::Mode::Resize,
            grid: d.grid,
            guides: d.guides,
        }
    }
}
fn draw_field(
    env: &mut JNIEnv<'_>,
    canvas: &JObject<'_>,
    paint: &JObject<'_>,
    d: &Decor,
    menu: u64,
) -> Result<()> {
    rectangle(env, canvas, paint, d.frame, theme::BACKGROUND, 0.)?;
    let p = d.picture;
    rectangle(env, canvas, paint, p, 0xff1d252a, 0.)?;
    let bitmap = EDITOR.lock().ok().and_then(|s| {
        s.as_ref()
            .filter(|s| s.menu == menu)
            .and_then(|s| s.bitmap.clone())
    });
    if let Some(bitmap) = bitmap {
        let rect = env.new_object(
            "android/graphics/RectF",
            "(FFFF)V",
            &[
                JValue::Float(p.left),
                JValue::Float(p.top),
                JValue::Float(p.right),
                JValue::Float(p.bottom),
            ],
        )?;
        env.call_method(paint, "setAlpha", "(I)V", &[JValue::Int(255)])?;
        env.call_method(paint, "setFilterBitmap", "(Z)V", &[JValue::Bool(1)])?;
        env.call_method(canvas,"drawBitmap","(Landroid/graphics/Bitmap;Landroid/graphics/Rect;Landroid/graphics/RectF;Landroid/graphics/Paint;)V",&[JValue::Object(bitmap.as_obj()),JValue::Object(&JObject::null()),JValue::Object(&rect),JValue::Object(paint)])?;
    } else {
        text(
            env,
            canvas,
            paint,
            "Game field",
            (p.center().0 - 34., p.center().1),
            14.,
            theme::MUTED,
        )?;
    }
    if d.grid {
        let mut x = d.frame.center().0;
        while x > d.frame.left {
            x -= 8.;
        }
        while x <= d.frame.right {
            line(
                env,
                canvas,
                paint,
                (x, d.frame.top, x, d.frame.bottom),
                0x183e9e79,
                0.5,
            )?;
            x += 8.;
        }
        let mut y = (d.frame.top / 8.).ceil() * 8.;
        while y <= d.frame.bottom {
            line(
                env,
                canvas,
                paint,
                (d.frame.left, y, d.frame.right, y),
                0x183e9e79,
                0.5,
            )?;
            y += 8.;
        }
    }
    Ok(())
}
fn line(
    env: &mut JNIEnv<'_>,
    canvas: &JObject<'_>,
    paint: &JObject<'_>,
    a: (f32, f32, f32, f32),
    color: u32,
    width: f32,
) -> Result<()> {
    env.call_method(paint, "setColor", "(I)V", &[JValue::Int(color as i32)])?;
    env.call_method(paint, "setStrokeWidth", "(F)V", &[JValue::Float(width)])?;
    env.call_method(
        canvas,
        "drawLine",
        "(FFFFLandroid/graphics/Paint;)V",
        &[
            JValue::Float(a.0),
            JValue::Float(a.1),
            JValue::Float(a.2),
            JValue::Float(a.3),
            JValue::Object(paint),
        ],
    )?;
    Ok(())
}
fn draw_selection(
    env: &mut JNIEnv<'_>,
    canvas: &JObject<'_>,
    paint: &JObject<'_>,
    d: &Decor,
) -> Result<()> {
    if let Some(a) = d.bounds {
        for (x, y, r, b) in [
            (a.left, a.top, a.right, a.top),
            (a.right, a.top, a.right, a.bottom),
            (a.right, a.bottom, a.left, a.bottom),
            (a.left, a.bottom, a.left, a.top),
        ] {
            line(env, canvas, paint, (x, y, r, b), theme::ACCENT, 2.)?;
        }
        if d.resize {
            for (x, y) in ntx::layout_edit::corners(a) {
                rectangle(
                    env,
                    canvas,
                    paint,
                    Area {
                        left: x - 8.,
                        top: y - 8.,
                        right: x + 8.,
                        bottom: y + 8.,
                    },
                    theme::ACCENT,
                    4.,
                )?;
                rectangle(
                    env,
                    canvas,
                    paint,
                    Area {
                        left: x - 3.,
                        top: y - 3.,
                        right: x + 3.,
                        bottom: y + 3.,
                    },
                    theme::BACKGROUND,
                    1.,
                )?;
            }
        }
    }
    if let Some(x) = d.guides.0 {
        line(
            env,
            canvas,
            paint,
            (x, d.frame.top, x, d.frame.bottom),
            theme::ACCENT,
            1.,
        )?;
    }
    if let Some(y) = d.guides.1 {
        line(
            env,
            canvas,
            paint,
            (d.frame.left, y, d.frame.right, y),
            theme::ACCENT,
            1.,
        )?;
    }
    Ok(())
}
