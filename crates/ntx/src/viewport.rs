// SPDX-License-Identifier: MIT
//! One presentation and input coordinate owner. Platform measurements are in
//! screen pixels; native input is in window pixels and drawing in buffer pixels.
//! Picture placement adapts public PSX MIT PlayerViewport/clear_picture policy;
//! see docs/PSX_TRANSFER.md. The engine supplies the GameCube aspect.
use crate::{
    Pad,
    touch::{Area, DownReport, Layout, TouchPad},
};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowGeometry {
    /// Actual window bounds, including any screen-space origin.
    pub window: Area,
    /// NativeActivity takes the decor's surface, not its inset content view.
    pub surface: Area,
    /// Framework content bounds, which may already exclude bars/cutouts.
    pub content: Area,
    /// Screen position of native input's window-local (0,0).
    pub input_origin: (f32, f32),
    /// Control clearance only: stable bars, cutout and mandatory gestures.
    /// The picture independently uses the complete taken drawable, as in PSX.
    pub clearance: Insets,
    pub density: f32,
}
impl WindowGeometry {
    /// Measured control-safe points, also used by the non-emulating editor.
    pub fn control_size(self) -> Option<(f32, f32)> {
        let safe = self.control_bounds()?;
        Some((
            (safe.right - safe.left) / self.density,
            (safe.bottom - safe.top) / self.density,
        ))
    }
    pub fn control_bounds(self) -> Option<Area> {
        let i = self.clearance;
        if !valid(self.window)
            || !valid(self.surface)
            || !valid(self.content)
            || ![i.left, i.top, i.right, i.bottom]
                .iter()
                .all(|n| n.is_finite() && *n >= 0.0)
            || !self.input_origin.0.is_finite()
            || !self.input_origin.1.is_finite()
            || !self.density.is_finite()
            || self.density <= 0.0
        {
            return None;
        }
        let safe = intersection(
            intersection(self.surface, self.content),
            Area {
                left: self.window.left + i.left,
                top: self.window.top + i.top,
                right: self.window.right - i.right,
                bottom: self.window.bottom - i.bottom,
            },
        );
        (valid(safe) && safe.right - safe.left >= 2.0 && safe.bottom - safe.top >= 2.0)
            .then_some(safe)
    }
    fn same_mapping(self, other: Self) -> bool {
        self.window == other.window
            && self.surface == other.surface
            && self.input_origin == other.input_origin
            && self.density == other.density
            && self
                .control_bounds()
                .is_some_and(|safe| Some(safe) == other.control_bounds())
    }
}

fn valid(a: Area) -> bool {
    [a.left, a.top, a.right, a.bottom]
        .iter()
        .all(|n| n.is_finite())
        && a.right > a.left
        && a.bottom > a.top
}
fn intersection(a: Area, b: Area) -> Area {
    Area {
        left: a.left.max(b.left),
        top: a.top.max(b.top),
        right: a.right.min(b.right),
        bottom: a.bottom.min(b.bottom),
    }
}

#[derive(Clone, Debug)]
pub struct Geometry {
    pub layout: Layout,
    pub density: f32,
    pub controls_fit: bool,
    picture: Option<Area>,
    picture_policy: crate::layout_edit::PicturePolicy,
    window: WindowGeometry,
    buffer: (f32, f32),
}
impl Geometry {
    pub fn new(window: WindowGeometry, width: f32, height: f32) -> Option<Self> {
        Self::with_controls(window, width, height, None)
    }
    pub fn with_controls(
        window: WindowGeometry,
        width: f32,
        height: f32,
        settings: Option<&crate::controls::Settings>,
    ) -> Option<Self> {
        Self::with_controls_aspect(window, width, height, settings, 4. / 3.)
    }
    pub fn with_controls_aspect(
        window: WindowGeometry,
        width: f32,
        height: f32,
        settings: Option<&crate::controls::Settings>,
        aspect: f32,
    ) -> Option<Self> {
        let safe = window.control_bounds()?;
        if !width.is_finite() || !height.is_finite() || width < 2.0 || height < 2.0 {
            return None;
        }
        // This player never requests an independently stretched surface. During
        // rotation the old Java bounds and new buffer can briefly disagree;
        // wait for a coherent measurement instead of admitting shifted targets.
        let surface_aspect = (window.surface.right - window.surface.left)
            / (window.surface.bottom - window.surface.top);
        if ((width / height) / surface_aspect - 1.0).abs() > 0.02 {
            return None;
        }
        // Intersect rectangles in one space before transforming: an already
        // inset surface/content is never charged the same inset twice.
        let mut layout = Layout::for_controls(
            safe.right - safe.left,
            safe.bottom - safe.top,
            window.density,
            None,
        );
        let sx = width / (window.surface.right - window.surface.left);
        let sy = height / (window.surface.bottom - window.surface.top);
        let map = |a: &mut Area| {
            a.left = (a.left + safe.left - window.surface.left) * sx;
            a.right = (a.right + safe.left - window.surface.left) * sx;
            a.top = (a.top + safe.top - window.surface.top) * sy;
            a.bottom = (a.bottom + safe.top - window.surface.top) * sy;
        };
        map(&mut layout.game);
        for zone in &mut layout.zones {
            map(&mut zone.area);
        }
        // Picture pixels do not share the controls' gesture/content margins.
        // The OS can transiently draw its bars over this stable picture. Actual
        // touch targets remain clear and share the same buffer/input mapping.
        let picture = intersection(window.surface, window.window);
        layout.game = Area {
            left: (picture.left - window.surface.left) * sx,
            top: (picture.top - window.surface.top) * sy,
            right: (picture.right - window.surface.left) * sx,
            bottom: (picture.bottom - window.surface.top) * sy,
        };
        let profile = settings.and_then(|s| {
            s.profiles.iter().find(|p| {
                p.display
                    == crate::controls::display_key(
                        (safe.right - safe.left) / window.density,
                        (safe.bottom - safe.top) / window.density,
                    )
            })
        });
        let picture = profile.and_then(|p| p.picture);
        let picture_policy = profile.map_or(Default::default(), |p| p.picture_policy);
        let mut areas = layout.zones.iter().map(|z| z.area).collect::<Vec<_>>();
        let safe_buffer = Area {
            left: (safe.left - window.surface.left) * sx,
            top: (safe.top - window.surface.top) * sy,
            right: (safe.right - window.surface.left) * sx,
            bottom: (safe.bottom - window.surface.top) * sy,
        };
        let buffer_aspect = aspect * sx / sy;
        let controls_fit = !picture_policy.is_game_first()
            || crate::layout_edit::place_free(
                &mut areas,
                safe_buffer,
                crate::layout_edit::edge_picture(layout.game, buffer_aspect),
                16. * window.density * sx.min(sy),
                &crate::controls::groups(&layout.zones),
            );
        for (zone, area) in layout.zones.iter_mut().zip(areas) {
            zone.area = area;
        }
        let defaults = layout.clone();
        if let Some(profile) = profile {
            crate::controls::apply_profile_to(
                profile,
                &mut layout,
                safe_buffer,
                window.density * sx.min(sy),
            );
            let p = picture.map_or_else(
                || crate::layout_edit::edge_picture(layout.game, buffer_aspect),
                |p| {
                    crate::layout_edit::fit(
                        crate::layout_edit::denormalized(p, layout.game),
                        buffer_aspect,
                    )
                },
            );
            if picture_policy.is_game_first()
                && layout
                    .zones
                    .iter()
                    .any(|z| crate::controls::overlaps(z.area, p))
            {
                layout = defaults;
            }
        }
        let active_picture = picture.map_or_else(
            || crate::layout_edit::edge_picture(layout.game, buffer_aspect),
            |p| crate::layout_edit::fit(crate::layout_edit::denormalized(p, layout.game), aspect),
        );
        let controls_fit = (!picture_policy.is_game_first()
            || layout
                .zones
                .iter()
                .all(|z| !crate::controls::overlaps(z.area, active_picture)))
            && (controls_fit || picture.is_some());
        Some(Self {
            picture,
            controls_fit,
            picture_policy,
            layout,
            density: window.density * sx.min(sy),
            window,
            buffer: (width, height),
        })
    }
    pub fn input_to_buffer(&self, x: f32, y: f32) -> (f32, f32) {
        (
            (x + self.window.input_origin.0 - self.window.surface.left) * self.buffer.0
                / (self.window.surface.right - self.window.surface.left),
            (y + self.window.input_origin.1 - self.window.surface.top) * self.buffer.1
                / (self.window.surface.bottom - self.window.surface.top),
        )
    }
    /// The presented Pause target in screen pixels; useful for scoped diagnosis.
    pub fn pause_screen_area(&self) -> Option<Area> {
        let a = self
            .layout
            .zones
            .iter()
            .find(|z| z.target == crate::touch::Target::Pause)?
            .area;
        let sx = (self.window.surface.right - self.window.surface.left) / self.buffer.0;
        let sy = (self.window.surface.bottom - self.window.surface.top) / self.buffer.1;
        Some(Area {
            left: self.window.surface.left + a.left * sx,
            top: self.window.surface.top + a.top * sy,
            right: self.window.surface.left + a.right * sx,
            bottom: self.window.surface.top + a.bottom * sy,
        })
    }
    pub fn picture(&self, aspect: f32) -> Area {
        let aspect = aspect
            * (self.buffer.0 / (self.window.surface.right - self.window.surface.left))
            / (self.buffer.1 / (self.window.surface.bottom - self.window.surface.top));
        if let Some(p) = self.picture {
            let a = crate::layout_edit::fit(
                crate::layout_edit::denormalized(p, self.layout.game),
                aspect,
            );
            if self.picture_policy.is_overlay()
                || !self
                    .layout
                    .zones
                    .iter()
                    .any(|z| crate::controls::overlaps(a, z.area))
            {
                return a;
            }
        }
        if self.picture_policy != crate::layout_edit::PicturePolicy::KeepClear {
            crate::layout_edit::edge_picture(self.layout.game, aspect)
        } else {
            crate::layout_edit::clear_picture(
                self.layout.game,
                self.layout.zones.iter().map(|z| z.area),
                aspect,
            )
        }
    }
}

/// Touch only addresses the geometry actually presented. Any new measurement,
/// invalidation or drawable size change releases captures before accepting input.
pub struct PlayerView {
    window: Option<WindowGeometry>,
    presented: Option<Arc<Geometry>>,
    touch: TouchPad,
    controls: Option<Arc<crate::controls::Settings>>,
    aspect: f32,
}
impl PlayerView {
    pub const fn new() -> Self {
        Self {
            window: None,
            presented: None,
            touch: TouchPad::new(),
            controls: None,
            aspect: 4. / 3.,
        }
    }
    pub fn configure_controls(&mut self, settings: crate::controls::Settings) {
        self.touch
            .configure(settings.dead_zone, settings.floating_sticks);
        self.controls = Some(Arc::new(settings));
        self.invalidate();
    }
    pub fn controls(&self) -> crate::controls::Settings {
        self.controls.as_deref().cloned().unwrap_or_default()
    }
    pub fn controls_opacity(&self) -> u8 {
        self.controls.as_deref().map_or(85, |s| s.opacity)
    }
    pub fn haptics(&self) -> bool {
        self.controls.as_deref().is_none_or(|s| s.haptics)
    }
    pub fn control_display(&self) -> Option<(f32, f32)> {
        self.window?.control_size()
    }
    pub fn measure(&mut self, window: Option<WindowGeometry>) {
        if self.window != window {
            // Bar/chrome measurements can change while effective targets and
            // input mapping stay identical. Retain the same immutable geometry
            // and gestures in that case; actual surface/mapping changes cancel.
            if !self
                .window
                .zip(window)
                .is_some_and(|(old, new)| old.same_mapping(new))
            {
                self.invalidate();
            }
            self.window = window;
        }
    }
    pub fn invalidate(&mut self) {
        self.presented = None;
        self.touch.cancel();
    }
    pub fn cancel(&mut self) {
        self.touch.cancel();
    }
    pub fn present_aspect(
        &mut self,
        width: f32,
        height: f32,
        aspect: f32,
    ) -> Option<&Arc<Geometry>> {
        if aspect.is_finite() && aspect > 0. && (self.aspect - aspect).abs() > 0.0001 {
            self.aspect = aspect;
            self.invalidate();
        }
        self.present(width, height)
    }
    pub fn present(&mut self, width: f32, height: f32) -> Option<&Arc<Geometry>> {
        if self
            .presented
            .as_ref()
            .is_none_or(|g| g.buffer != (width, height))
        {
            self.invalidate();
            self.presented = self
                .window
                .and_then(|w| {
                    Geometry::with_controls_aspect(
                        w,
                        width,
                        height,
                        self.controls.as_deref(),
                        self.aspect,
                    )
                })
                .map(Arc::new);
        }
        self.presented.as_ref()
    }
    pub fn down(&mut self, id: i32, x: f32, y: f32) -> bool {
        self.down_report(id, x, y) == DownReport::Pause
    }
    pub fn has_presented_geometry(&self) -> bool {
        self.presented.is_some()
    }
    pub fn down_report(&mut self, id: i32, x: f32, y: f32) -> DownReport {
        let Some(g) = &self.presented else {
            return DownReport::NoGeometry;
        };
        let (x, y) = g.input_to_buffer(x, y);
        self.touch.down_report(id, x, y, &g.layout)
    }
    pub fn moved(&mut self, id: i32, x: f32, y: f32) {
        if let Some(g) = &self.presented {
            let (x, y) = g.input_to_buffer(x, y);
            self.touch.moved(id, x, y);
        }
    }
    pub fn up(&mut self, id: i32) {
        self.touch.up(id);
    }
    pub fn pad(&self) -> Pad {
        self.touch.pad()
    }
    pub fn sample_pad(&mut self) -> Pad {
        self.touch.sample_pad()
    }
    pub fn begin_runtime(&mut self) {
        self.touch.begin_runtime();
    }
    pub fn delivery(&self) -> crate::touch::DeliveryStats {
        self.touch.delivery()
    }
    pub fn record_source_age(&mut self, age: Option<std::time::Duration>) {
        self.touch.record_source_age(age);
    }
    pub fn stick_center(&self, axis: crate::Control) -> Option<(f32, f32)> {
        self.touch.stick_center(axis)
    }
}
impl Default for PlayerView {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Control, touch::Target};
    #[test]
    fn custom_control_uses_one_presented_draw_hit_geometry_and_setting_change_releases_input() {
        let mut view = PlayerView::new();
        let w = window();
        view.measure(Some(w));
        let (width, height) = view.control_display().unwrap();
        let mut editor =
            crate::controls::Editor::new(width, height, crate::controls::Settings::default());
        // Move Pause into the clear centre using the actual display's editor.
        let a = editor
            .layout
            .zones
            .iter()
            .find(|z| z.target == Target::Pause)
            .unwrap()
            .area;
        let (x, y) = a.center();
        editor.down(1, x, y);
        editor.moved(1, width / 2., height / 2.);
        editor.up(1);
        view.configure_controls(editor.finish().unwrap());
        assert_eq!(view.down_report(1, x, y), DownReport::NoGeometry);
        let g = view.present(2280., 1080.).unwrap().clone();
        let (sx, sy) = g.pause_screen_area().unwrap().center();
        assert_eq!(
            view.down_report(1, sx - w.input_origin.0, sy - w.input_origin.1),
            DownReport::Pause
        );
        let a = g
            .layout
            .zones
            .iter()
            .find(|z| z.target == Target::Button(Control::A))
            .unwrap()
            .area;
        let (x, y) = a.center();
        view.down_report(2, x, y);
        assert_eq!(view.pad().values()[Control::A as usize], 1.);
        view.configure_controls(crate::controls::Settings::default());
        assert_eq!(view.sample_pad().values(), &[0.; 18]);
        assert!(!view.has_presented_geometry());
    }
    fn rect(left: f32, top: f32, right: f32, bottom: f32) -> Area {
        Area {
            left,
            top,
            right,
            bottom,
        }
    }
    fn window() -> WindowGeometry {
        WindowGeometry {
            window: rect(100.0, 200.0, 2380.0, 1280.0),
            surface: rect(100.0, 200.0, 2380.0, 1280.0),
            content: rect(100.0, 200.0, 2380.0, 1280.0),
            input_origin: (100.0, 200.0),
            clearance: Insets {
                left: 0.0,
                top: 66.0,
                right: 0.0,
                bottom: 66.0,
            },
            density: 2.75,
        }
    }
    #[test]
    fn pause_reentry_requires_fresh_presentation_and_reports_the_actual_hit() {
        let mut view = PlayerView::new();
        let w = window();
        view.measure(Some(w));
        assert_eq!(view.down_report(1, 0.0, 0.0), DownReport::NoGeometry);
        // A scaled buffer with translated screen/window origins must agree.
        let g = view.present(4560.0, 2160.0).unwrap().clone();
        let (sx, sy) = g.pause_screen_area().unwrap().center();
        let input = (sx - w.input_origin.0, sy - w.input_origin.1);
        assert_eq!(view.down_report(1, input.0, input.1), DownReport::Pause);
        view.measure(None); // focus/surface departure
        assert_eq!(
            view.down_report(1, input.0, input.1),
            DownReport::NoGeometry
        );
        view.measure(Some(w)); // measurement alone is not presentation
        assert_eq!(
            view.down_report(1, input.0, input.1),
            DownReport::NoGeometry
        );
        view.present(4560.0, 2160.0).unwrap();
        assert_eq!(view.down_report(1, input.0, input.1), DownReport::Pause);
        assert_eq!(view.down_report(2, -500.0, -500.0), DownReport::Miss);
    }
    #[test]
    fn down_reports_distinguish_capture_rejection_and_missing_geometry() {
        let mut view = PlayerView::new();
        let w = window();
        view.measure(Some(w));
        let g = view.present(2280.0, 1080.0).unwrap().clone();
        let (x, y) = g
            .layout
            .zones
            .iter()
            .find(|z| z.target == Target::Button(Control::A))
            .unwrap()
            .area
            .center();
        assert_eq!(view.down_report(7, x, y), DownReport::Captured);
        assert_eq!(view.down_report(7, x, y), DownReport::Rejected);
        assert_eq!(view.sample_pad().values()[Control::A as usize], 1.0);
        view.measure(None);
        assert_eq!(view.sample_pad().values(), &[0.0; 18]);
        assert_eq!(view.down_report(8, x, y), DownReport::NoGeometry);
    }
    #[test]
    fn stable_draws_reuse_geometry_and_native_poll_alone_consumes_input() {
        let mut view = PlayerView::new();
        let w = window();
        view.measure(Some(w));
        let first = view.present(2280.0, 1080.0).unwrap().clone();
        let a = first
            .layout
            .zones
            .iter()
            .find(|z| z.target == Target::Button(Control::A))
            .unwrap()
            .area;
        let (x, y) = a.center();
        view.down(1, x, y);
        view.up(1);
        for _ in 0..50 {
            view.measure(Some(w));
            let frame = view.present(2280.0, 1080.0).unwrap();
            assert!(Arc::ptr_eq(&first, frame));
            assert_eq!(view.pad().values()[Control::A as usize], 0.0);
        }
        assert_eq!(view.sample_pad().values()[Control::A as usize], 1.0);
        assert_eq!(view.sample_pad().values()[Control::A as usize], 0.0);
        view.down(2, x, y);
        view.up(2);
        let mut changed = w;
        changed.clearance.left = 50.0;
        view.measure(Some(changed));
        let second = view.present(2280.0, 1080.0).unwrap();
        assert!(!Arc::ptr_eq(&first, second));
        assert_eq!(view.sample_pad().values(), &[0.0; 18]);
    }
    #[test]
    fn equivalent_inset_notifications_keep_held_sticks_and_pending_presses() {
        let mut view = PlayerView::new();
        let hidden = window();
        view.measure(Some(hidden));
        let first = view.present(2280.0, 1080.0).unwrap().clone();
        let stick = first
            .layout
            .zones
            .iter()
            .find(|z| matches!(z.target, Target::Stick(Control::StickX, _)))
            .unwrap()
            .area;
        let button = first
            .layout
            .zones
            .iter()
            .find(|z| z.target == Target::Button(Control::A))
            .unwrap()
            .area;
        view.down(1, stick.center().0, stick.center().1);
        view.moved(1, stick.center().0 + 40.0, stick.center().1);
        view.down(2, button.center().0, button.center().1);
        view.up(2);
        let expected = view.pad().values()[Control::StickX as usize];
        let mut shown = hidden;
        shown.content = rect(100.0, 266.0, 2380.0, 1214.0);
        view.measure(Some(shown));
        assert!(Arc::ptr_eq(&first, view.present(2280.0, 1080.0).unwrap()));
        let packet = view.sample_pad();
        assert_eq!(packet.values()[Control::StickX as usize], expected);
        assert!(expected > 0.0);
        assert_eq!(packet.values()[Control::A as usize], 1.0);
        view.measure(Some(hidden));
        assert!(Arc::ptr_eq(&first, view.present(2280.0, 1080.0).unwrap()));
        assert_eq!(view.pad().values()[Control::StickX as usize], expected);
    }
    #[test]
    fn nonzero_origins_inset_and_edge_to_edge_surfaces_have_the_same_screen_layout() {
        let w = window();
        let full = Geometry::new(w, 2280.0, 1080.0).unwrap();
        let mut inset = w;
        inset.surface = rect(100.0, 266.0, 2380.0, 1214.0);
        inset.content = inset.surface;
        let fitted = Geometry::new(inset, 2280.0, 948.0).unwrap();
        for g in [&full, &fitted] {
            assert!(g.controls_fit);
            let picture = g.picture(4. / 3.);
            for z in &g.layout.zones {
                assert!(!crate::controls::overlaps(z.area, picture));
            }
            let p = g.layout.zones[0].area.center();
            let screen = (
                g.window.surface.left - g.window.input_origin.0 + p.0,
                g.window.surface.top - g.window.input_origin.1 + p.1,
            );
            let mapped = g.input_to_buffer(screen.0, screen.1);
            assert!((mapped.0 - p.0).abs() < 0.001 && (mapped.1 - p.1).abs() < 0.001);
        }
        assert_eq!(full.layout.game.top, 0.0);
        assert_eq!(fitted.layout.game.top, 0.0);
    }
    #[test]
    fn hidden_visible_bars_cutouts_density_and_scaled_buffer_keep_draw_and_touch_together() {
        for density in [1.0, 2.75, 3.0] {
            for cutout in [0.0, 80.0] {
                for right in [false, true] {
                    let mut w = window();
                    w.density = density;
                    if right {
                        w.clearance.right = cutout;
                    } else {
                        w.clearance.left = cutout;
                    }
                    // Hidden/transient bars keep measured stable clearance; visible
                    // bars can additionally inset content without changing safe bounds.
                    for visible in [false, true] {
                        if visible {
                            w.content = rect(
                                100.0 + w.clearance.left,
                                266.0,
                                2380.0 - w.clearance.right,
                                1214.0,
                            );
                        }
                        let mut view = PlayerView::new();
                        view.measure(Some(w));
                        let g = view.present(1140.0, 540.0).unwrap().clone();
                        for z in &g.layout.zones {
                            assert!(z.area.top >= 33.0 && z.area.bottom <= 507.01);
                            assert!(z.area.left >= cutout * 0.5 || right);
                            assert!(z.area.right <= 1140.0 - cutout * 0.5 || !right);
                            let (x, y) = z.area.center();
                            let input = (x * 2.0, y * 2.0);
                            let mapped = g.input_to_buffer(input.0, input.1);
                            assert!((mapped.0 - x).abs() < 0.001 && (mapped.1 - y).abs() < 0.001);
                            let pause = view.down(1, input.0, input.1);
                            assert_eq!(pause, z.target == Target::Pause);
                            if let Target::Button(c) = z.target {
                                assert_eq!(view.pad().values()[c as usize], 1.0);
                            }
                            view.cancel();
                        }
                        let picture = g.picture(4.0 / 3.0);
                        assert!(
                            ((picture.right - picture.left) / (picture.bottom - picture.top)
                                - 4.0 / 3.0)
                                .abs()
                                < 0.0001
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn geometry_change_or_invalid_transient_surface_neutralizes_held_input() {
        let mut view = PlayerView::new();
        let mut w = window();
        view.measure(Some(w));
        let a = view
            .present(2280.0, 1080.0)
            .unwrap()
            .layout
            .zones
            .iter()
            .find(|z| z.target == Target::Button(Control::A))
            .unwrap()
            .area
            .center();
        view.down(1, a.0, a.1);
        assert_eq!(view.pad().values()[0], 1.0);
        w.clearance.left = 80.0;
        view.measure(Some(w));
        assert!(view.pad().values().iter().all(|v| *v == 0.0));
        assert!(!view.down(2, a.0, a.1));
        assert!(view.pad().values().iter().all(|v| *v == 0.0));
        for (width, height) in [(0.0, 0.0), (1.0, 2.0), (f32::NAN, 1080.0)] {
            assert!(view.present(width, height).is_none());
        }
        assert!(view.present(2280.0, 1080.0).is_some());
        assert!(view.present(1080.0, 2280.0).is_none());
        view.measure(None);
        assert!(view.present(2280.0, 1080.0).is_none());
        w.content = rect(100.0, 200.0, 100.0, 200.0);
        assert!(Geometry::new(w, 2280.0, 1080.0).is_none());
    }
    #[test]
    fn inset_surface_input_origin_and_partial_travel_share_the_rendered_targets() {
        let mut w = window();
        w.surface = rect(130.0, 266.0, 2380.0, 1214.0);
        w.content = w.surface;
        let mut view = PlayerView::new();
        view.measure(Some(w));
        let g = view.present(1125.0, 474.0).unwrap().clone();
        let travel = g
            .layout
            .zones
            .iter()
            .find(|z| z.target == Target::Travel(Control::LTravel))
            .unwrap()
            .area;
        let click = g
            .layout
            .zones
            .iter()
            .find(|z| z.target == Target::Button(Control::LClick))
            .unwrap()
            .area;
        // Native motion is window-local, while this surface is offset by
        // (30,66) inside it; the buffer has half as many pixels in each axis.
        let input = |x: f32, y: f32| (x * 2.0 + 30.0, y * 2.0 + 66.0);
        let (x, y) = input(
            travel.left + (travel.right - travel.left) * 0.35,
            travel.center().1,
        );
        view.down(1, x, y);
        assert!((view.pad().values()[Control::LTravel as usize] - 0.35).abs() < 0.0001);
        assert_eq!(view.pad().values()[Control::LClick as usize], 0.0);
        let (x, y) = input(click.center().0, click.center().1);
        view.down(2, x, y);
        assert_eq!(view.pad().values()[Control::LClick as usize], 1.0);
        view.up(2);
        assert_eq!(view.pad().values()[Control::LClick as usize], 0.0);
        view.present(2250.0, 948.0);
        assert!(view.pad().values().iter().all(|v| *v == 0.0));
        // Short/transient but nonzero surfaces remain bounded, without NaNs.
        let short = Geometry::new(w, 200.0, 84.0).unwrap();
        for z in short.layout.zones {
            assert!(valid(z.area) && z.area.right <= 200.01 && z.area.bottom <= 84.01);
        }
    }
    #[test]
    fn content_clearance_applies_to_controls_while_picture_uses_the_drawable() {
        let mut w = window();
        w.content.top = 354.0;
        let chrome = Geometry::new(w, 2280.0, 1080.0).unwrap();
        w.content = w.surface;
        let g = Geometry::new(w, 2280.0, 1080.0).unwrap();
        assert_eq!(g.layout.game, rect(0.0, 0.0, 2280.0, 1080.0));
        assert_eq!(g.layout.game, chrome.layout.game);
        assert!(g.layout.zones[0].area.top < chrome.layout.zones[0].area.top);
        for zone in g.layout.zones {
            assert!(zone.area.top >= 66.0 && zone.area.bottom <= 1014.01);
        }
    }
    #[test]
    fn edge_to_edge_picture_uses_full_height_and_actual_engine_aspect_clear_of_targets() {
        for aspect in [1.25, 4.0 / 3.0, 16.0 / 9.0] {
            let g = Geometry::with_controls_aspect(window(), 2280., 1080., None, aspect).unwrap();
            let p = g.picture(aspect);
            assert_eq!(p.top, 0.0);
            assert!(((p.right - p.left) / (p.bottom - p.top) - aspect).abs() < 0.0001);
            assert!(p.bottom <= 1080.01 && p.left >= 0.0 && p.right <= 2280.01);
            for z in &g.layout.zones {
                let a = z.area;
                assert!(
                    p.right <= a.left + 0.01
                        || p.left >= a.right - 0.01
                        || p.bottom <= a.top + 0.01
                        || p.top >= a.bottom - 0.01
                );
            }
        }
        let g = Geometry::with_controls_aspect(window(), 2280., 1080., None, 1.25).unwrap();
        let p = g.picture(1.25);
        assert_eq!(p.bottom, 1080.0);
        assert!((p.right - p.left - 1350.0).abs() < 0.01);
    }
    #[test]
    fn fold_deck_allows_full_width_without_covering_controls_or_stretching_the_game() {
        for (density, bottom) in [(2.4375, 80.), (2.45, 78.), (2.625, 48.), (2.75, 16.)] {
            let bounds = rect(0., 0., 2076., 2152.);
            let w = WindowGeometry {
                window: bounds,
                surface: bounds,
                content: bounds,
                input_origin: (0., 0.),
                clearance: Insets {
                    bottom,
                    ..Insets::default()
                },
                density,
            };
            for aspect in [1.25, 4. / 3.] {
                let g = Geometry::with_controls_aspect(w, 2076., 2152., None, aspect).unwrap();
                let picture = g.picture(aspect);
                assert!(
                    picture.left.abs() < 0.01 && (picture.right - 2076.).abs() < 0.01,
                    "narrow Fold picture at density {density}: {picture:?}"
                );
                assert!(
                    ((picture.right - picture.left) / (picture.bottom - picture.top) - aspect)
                        .abs()
                        < 0.0001
                );
                for z in &g.layout.zones {
                    let a = z.area;
                    assert!(
                        picture.right <= a.left + 0.01
                            || picture.left >= a.right - 0.01
                            || picture.bottom <= a.top + 0.01
                            || picture.top >= a.bottom - 0.01
                    );
                }
            }
        }
    }
    #[test]
    fn revealing_bars_does_not_shrink_or_offset_the_picture_or_break_touch_mapping() {
        let w = window();
        let hidden = Geometry::new(w, 2280.0, 1080.0).unwrap();
        let mut revealed = w;
        revealed.content = rect(100.0, 266.0, 2380.0, 1214.0);
        let shown = Geometry::new(revealed, 2280.0, 1080.0).unwrap();
        assert_eq!(hidden.picture(1.25), shown.picture(1.25));
        for (a, b) in hidden.layout.zones.iter().zip(&shown.layout.zones) {
            assert_eq!(a.area, b.area);
            let (x, y) = a.area.center();
            let mapped = shown.input_to_buffer(x, y);
            assert!((mapped.0 - x).abs() < 0.001 && (mapped.1 - y).abs() < 0.001);
        }
    }
}

#[cfg(test)]
mod game_first_tests {
    use super::*;
    use crate::{
        controls::{Editor, Settings},
        layout_edit::{PicturePolicy, same_area},
        n64_touch::View,
    };
    fn fixture() -> WindowGeometry {
        let a = Area {
            left: 0.,
            top: 0.,
            right: 2280.,
            bottom: 1080.,
        };
        WindowGeometry {
            window: a,
            surface: a,
            content: a,
            input_origin: (0., 0.),
            density: 2.75,
            clearance: Insets {
                top: 84.,
                bottom: 84.,
                ..Default::default()
            },
        }
    }
    #[test]
    fn full_height_picture_and_free_space_controls_share_runtime_and_editor_coordinates() {
        let w = fixture();
        let safe = w.control_bounds().unwrap();
        let (cw, ch) = w.control_size().unwrap();
        let frame = Area {
            left: (w.surface.left - safe.left) / w.density,
            top: (w.surface.top - safe.top) / w.density,
            right: (w.surface.right - safe.left) / w.density,
            bottom: (w.surface.bottom - safe.top) / w.density,
        };
        for aspect in [1.25, 4. / 3., 16. / 9.] {
            let g = Geometry::with_controls_aspect(w, 1140., 540., None, aspect).unwrap();
            assert!(g.controls_fit);
            let p = g.picture(aspect);
            assert_eq!(p.top, 0.);
            assert_eq!(p.bottom, 540.);
            assert!(((p.right - p.left) / 540. - aspect).abs() < 0.0001);
            let mut editor = Editor::new(cw, ch, Settings::default());
            editor.configure_frame(frame, aspect);
            for (runtime, draft) in g.layout.zones.iter().zip(&editor.layout.zones) {
                assert_eq!(runtime.target, draft.target);
                assert!(!crate::controls::overlaps(runtime.area, p));
                let map = |a: Area| Area {
                    left: (a.left * w.density + safe.left) * 0.5,
                    top: (a.top * w.density + safe.top) * 0.5,
                    right: (a.right * w.density + safe.left) * 0.5,
                    bottom: (a.bottom * w.density + safe.top) * 0.5,
                };
                assert!(same_area(runtime.area, map(draft.area)));
            }
            assert!(editor.finish().unwrap().profiles.is_empty());
        }
        let mut view = View::default();
        view.measure(Some(w));
        let p = view.picture(2280., 1080., 4. / 3.).unwrap();
        assert!(view.controls_fit);
        assert_eq!((p.top, p.bottom), (0., 1080.));
        let mut editor = crate::n64_controls::Editor::new(cw, ch, Settings::default());
        editor.configure_frame(frame, 4. / 3.);
        for (runtime, draft) in view.zones.iter().zip(&editor.zones) {
            assert_eq!(runtime.target, draft.target);
            assert!(!crate::controls::overlaps(runtime.area, p));
            assert!(same_area(
                runtime.area,
                Area {
                    left: draft.area.left * w.density + safe.left,
                    top: draft.area.top * w.density + safe.top,
                    right: draft.area.right * w.density + safe.left,
                    bottom: draft.area.bottom * w.density + safe.top
                }
            ));
        }
        assert!(editor.finish().unwrap().n64_profiles.is_empty());
    }
    #[test]
    fn legacy_preferences_and_explicit_alternatives_preserve_their_meaning_without_silent_overlap()
    {
        let settings: Settings =
            serde_json::from_str(r#"{"profiles":[{"display":"800x600","zones":[]}]}"#).unwrap();
        assert_eq!(
            settings.profiles[0].picture_policy,
            PicturePolicy::KeepClear
        );
        let a = Area {
            left: 0.,
            top: 0.,
            right: 800.,
            bottom: 600.,
        };
        let w = WindowGeometry {
            window: a,
            surface: a,
            content: a,
            input_origin: (0., 0.),
            density: 1.,
            clearance: Insets::default(),
        };
        let g = Geometry::new(w, 800., 600.).unwrap();
        assert!(
            !g.controls_fit,
            "a matching-aspect drawable leaves no free space"
        );
        let mut editor = Editor::new(800., 600., Settings::default());
        assert!(editor.draft.requires_choice());
        editor.draft.set_picture_policy(PicturePolicy::KeepClear);
        assert!(!editor.draft.requires_choice());
        let clear = editor.draft.picture();
        assert!(clear.right - clear.left < 800.);
        editor.draft.undo();
        assert!(editor.draft.requires_choice());
        editor.draft.set_picture_policy(PicturePolicy::Overlay);
        assert!(!editor.draft.requires_choice());
        let saved = editor.finish().unwrap();
        assert_eq!(saved.profiles[0].picture_policy, PicturePolicy::Overlay);
        assert_eq!(
            Geometry::with_controls(w, 800., 600., Some(&saved))
                .unwrap()
                .picture(4. / 3.),
            a
        );
    }
    #[test]
    fn changed_backend_aspect_reflows_only_ephemeral_geometry_and_releases_old_contacts() {
        let mut view = PlayerView::new();
        view.measure(Some(fixture()));
        let a = view
            .present_aspect(2280., 1080., 1.25)
            .unwrap()
            .layout
            .zones
            .iter()
            .find(|z| z.target == crate::touch::Target::Button(crate::Control::A))
            .unwrap()
            .area
            .center();
        view.down(1, a.0, a.1);
        assert_eq!(view.pad().values()[crate::Control::A as usize], 1.);
        let next = view.present_aspect(2280., 1080., 16. / 9.).unwrap().clone();
        assert!(view.pad().values().iter().all(|v| *v == 0.));
        assert_eq!(next.picture(16. / 9.).bottom, 1080.);
        assert!(
            next.layout
                .zones
                .iter()
                .all(|z| !crate::controls::overlaps(z.area, next.picture(16. / 9.)))
        );
    }
}

#[cfg(test)]
mod buffer_scale_tests {
    use super::*;
    #[test]
    fn physical_picture_aspect_survives_independent_native_buffer_axis_scaling() {
        let a = Area {
            left: 0.,
            top: 0.,
            right: 2280.,
            bottom: 1080.,
        };
        let w = WindowGeometry {
            window: a,
            surface: a,
            content: a,
            input_origin: (0., 0.),
            density: 2.75,
            clearance: Insets {
                top: 84.,
                bottom: 84.,
                ..Default::default()
            },
        };
        let mut view = crate::n64_touch::View::default();
        view.measure(Some(w));
        for (width, height) in [(2280., 1080.), (1140., 507.), (1140., 540.)] {
            let p = view.picture(width, height, 4. / 3.).unwrap();
            assert_eq!((p.top, p.bottom), (0., height));
            let physical_width = (p.right - p.left) * 2280. / width;
            let physical_height = (p.bottom - p.top) * 1080. / height;
            assert!((physical_width / physical_height - 4. / 3.).abs() < 0.0001);
            assert!((physical_width - 1440.).abs() < 0.001);
        }
        // Small rounding differences in a coherent GC drawable are compensated too.
        let g = Geometry::with_controls_aspect(w, 1125., 540., None, 1.25).unwrap();
        let p = g.picture(1.25);
        assert!(
            (((p.right - p.left) * 2280. / 1125.) / ((p.bottom - p.top) * 1080. / 540.) - 1.25)
                .abs()
                < 0.0001
        );
    }
}
