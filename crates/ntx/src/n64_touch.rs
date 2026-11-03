// SPDX-License-Identifier: MIT
//! N64 controls in the host's measured coordinate space. Pointer capture and
//! quick-press delivery belong to input policy, independently of rendering.
use crate::{
    n64::{Button, Pad},
    touch::Area,
    viewport::WindowGeometry,
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Target {
    Button(Button),
    Stick,
    Pause,
}
#[derive(Clone, Copy)]
pub struct Zone {
    pub area: Area,
    pub target: Target,
    pub label: &'static str,
}
#[derive(Default)]
pub struct View {
    measured: Option<WindowGeometry>,
    pub zones: Vec<Zone>,
    pointers: BTreeMap<i32, (usize, f32, f32)>,
    pulses: [bool; 14],
    pub sampled: Pad,
    dead_zone: u8,
    profiles: Vec<crate::n64_controls::Profile>,
    picture: Option<Area>,
}
impl View {
    /// Picture and controls use the same measured window and buffer transform.
    pub fn picture(&self, width: f32, height: f32, aspect: f32) -> Option<Area> {
        let g = self.measured?;
        g.control_bounds()?;
        let frame = Area {
            left: 0.,
            top: 0.,
            right: width,
            bottom: height,
        };
        let sx = width / (g.surface.right - g.surface.left);
        let sy = height / (g.surface.bottom - g.surface.top);
        let zones = self.zones.iter().map(|z| Area {
            left: (z.area.left - g.surface.left) * sx,
            top: (z.area.top - g.surface.top) * sy,
            right: (z.area.right - g.surface.left) * sx,
            bottom: (z.area.bottom - g.surface.top) * sy,
        });
        if let Some(p) = self.picture {
            let a = crate::layout_edit::fit(crate::layout_edit::denormalized(p, frame), aspect);
            if !zones.clone().any(|b| crate::controls::overlaps(a, b)) {
                return Some(a);
            }
        }
        Some(crate::layout_edit::clear_picture(frame, zones, aspect))
    }
    pub fn configure(&mut self, settings: &crate::controls::Settings) {
        self.configure_dead_zone(settings.dead_zone);
        if self.profiles != settings.n64_profiles {
            self.cancel();
            self.profiles.clone_from(&settings.n64_profiles);
            let geometry = self.measured.take();
            self.measure(geometry);
        }
    }
    pub fn configure_dead_zone(&mut self, percent: u8) {
        let percent = percent.min(25);
        if self.dead_zone != percent {
            self.cancel();
            self.dead_zone = percent;
        }
    }
    pub fn measure(&mut self, geometry: Option<WindowGeometry>) {
        if self.measured == geometry {
            return;
        }
        self.cancel();
        self.measured = geometry;
        self.zones.clear();
        self.picture = None;
        let Some(g) = geometry else {
            return;
        };
        let Some(a) = g.control_bounds() else {
            return;
        };
        let key = crate::controls::display_key(
            (a.right - a.left) / g.density,
            (a.bottom - a.top) / g.density,
        );
        self.picture = self
            .profiles
            .iter()
            .find(|p| p.display == key)
            .and_then(|p| p.picture);
        let w = a.right - a.left;
        let h = a.bottom - a.top;
        let r = (22.0 * g.density).min(w / 22.0).min(h / 9.0);
        let mut add = |x: f32, y: f32, r: f32, target, label| {
            self.zones.push(Zone {
                area: Area {
                    left: x - r,
                    top: y - r,
                    right: x + r,
                    bottom: y + r,
                },
                target,
                label,
            })
        };
        let x = a.left;
        let y = a.bottom;
        add(x + 3.1 * r, y - 3.0 * r, 2.0 * r, Target::Stick, "");
        for (dx, dy, b, label) in [
            (6., 1.1, Button::Left, "<"),
            (7., 2.1, Button::Up, "^"),
            (7., 0.1, Button::Down, "v"),
            (8., 1.1, Button::Right, ">"),
        ] {
            add(
                x + dx * r,
                y - (dy + 0.8) * r,
                0.47 * r,
                Target::Button(b),
                label,
            );
        }
        let right = a.right;
        for (dx, dy, b, label) in [
            (1.3, 1.8, Button::A, "A"),
            (3.4, 2.5, Button::B, "B"),
            (4.7, 5.6, Button::CLeft, "<"),
            (3.3, 7., Button::CUp, "^"),
            (3.3, 4.2, Button::CDown, "v"),
            (1.9, 5.6, Button::CRight, ">"),
        ] {
            add(
                right - dx * r,
                y - dy * r,
                if matches!(b, Button::A | Button::B) {
                    r
                } else {
                    0.61 * r
                },
                Target::Button(b),
                label,
            );
        }
        add(
            x + 1.4 * r,
            a.top + 1.2 * r,
            0.9 * r,
            Target::Button(Button::L),
            "L",
        );
        add(
            x + 3.7 * r,
            a.top + 1.2 * r,
            0.9 * r,
            Target::Button(Button::Z),
            "Z",
        );
        add(
            right - 1.4 * r,
            a.top + 1.2 * r,
            0.9 * r,
            Target::Button(Button::R),
            "R",
        );
        // Keep auxiliary controls out of the landscape game field. A centered
        // Pause/Start pair needlessly reduced the clear 4:3 picture's height.
        let landscape = w > h * 1.25;
        let aux = if landscape { x + 6.8 * r } else { x + w * 0.5 };
        add(aux, a.top + 1.2 * r, 0.8 * r, Target::Pause, "II");
        add(
            aux,
            if landscape {
                (y - 7. * r).max(a.top + 3. * r)
            } else {
                y - r
            },
            0.7 * r,
            Target::Button(Button::Start),
            "S",
        );
        crate::n64_controls::apply_profile(&self.profiles, &mut self.zones, a, g.density);
    }
    pub fn cancel(&mut self) {
        self.pointers.clear();
        self.pulses.fill(false);
        self.sampled.cancel();
    }
    pub fn down(&mut self, id: i32, x: f32, y: f32) -> Option<Target> {
        let g = self.measured?;
        let (x, y) = (x + g.input_origin.0, y + g.input_origin.1);
        let (index, z) = self.zones.iter().enumerate().rev().find(|(_, z)| {
            x >= z.area.left && x <= z.area.right && y >= z.area.top && y <= z.area.bottom
        })?;
        if z.target == Target::Stick
            && self
                .pointers
                .values()
                .any(|(i, _, _)| self.zones[*i].target == Target::Stick)
        {
            return None;
        }
        let target = z.target;
        if let Target::Button(b) = target {
            self.pulses[b as usize] = true;
        }
        if target != Target::Pause {
            self.pointers.insert(id, (index, x, y));
        }
        Some(target)
    }
    pub fn move_pointer(&mut self, id: i32, x: f32, y: f32) {
        if let (Some(g), Some(p)) = (self.measured, self.pointers.get_mut(&id)) {
            p.1 = x + g.input_origin.0;
            p.2 = y + g.input_origin.1;
        }
    }
    pub fn up(&mut self, id: i32) {
        self.pointers.remove(&id);
    }
    pub fn poll(&mut self) {
        let mut pad = Pad::default();
        for z in &self.zones {
            if let Target::Button(b) = z.target {
                pad.button(b, self.pulses[b as usize]);
            }
        }
        self.pulses.fill(false);
        for (i, x, y) in self.pointers.values() {
            let z = self.zones[*i];
            match z.target {
                Target::Button(b) => pad.button(b, true),
                Target::Stick => {
                    let (cx, cy) = z.area.center();
                    let reach = (z.area.right - z.area.left) * 0.4;
                    let dx = (x - cx) / reach;
                    let dy = (cy - y) / reach;
                    let raw_length = (dx * dx + dy * dy).sqrt();
                    let dead = self.dead_zone as f32 / 100.;
                    let response = if raw_length <= dead {
                        0.
                    } else {
                        ((raw_length - dead) / (1. - dead)).min(1.)
                    };
                    let length = raw_length.max(f32::EPSILON);
                    pad.stick(
                        (dx / length * response * 32767.) as i16,
                        (dy / length * response * 32767.) as i16,
                    );
                }
                Target::Pause => {}
            }
        }
        self.sampled = pad;
    }
    pub fn screen_to_buffer(&self, x: f32, y: f32, width: f32, height: f32) -> Option<(f32, f32)> {
        let g = self.measured?;
        Some((
            (x - g.surface.left) * width / (g.surface.right - g.surface.left),
            (y - g.surface.top) * height / (g.surface.bottom - g.surface.top),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distinct_targets_remain_disjoint_on_short_narrow_and_tall_safe_rectangles() {
        for (w, h) in [(484., 198.), (1000., 500.), (400., 900.)] {
            let a = Area {
                left: 21.,
                top: 13.,
                right: 21. + w,
                bottom: 13. + h,
            };
            let mut view = View::default();
            view.measure(Some(WindowGeometry {
                window: a,
                surface: a,
                content: a,
                input_origin: (21., 13.),
                clearance: Default::default(),
                density: 1.,
            }));
            for (i, z) in view.zones.iter().enumerate() {
                assert!(
                    z.area.left >= a.left
                        && z.area.top >= a.top
                        && z.area.right <= a.right
                        && z.area.bottom <= a.bottom
                );
                for other in &view.zones[..i] {
                    let p = z.area;
                    let q = other.area;
                    assert!(
                        !(p.left < q.right
                            && p.right > q.left
                            && p.top < q.bottom
                            && p.bottom > q.top),
                        "Overlapping {:?} and {:?}",
                        z.target,
                        other.target
                    );
                }
            }
        }
    }
    #[test]
    fn quick_simultaneous_c_press_survives_until_poll_and_cancel_clears_every_pointer() {
        let a = Area {
            left: 0.,
            top: 0.,
            right: 1000.,
            bottom: 500.,
        };
        let mut view = View::default();
        view.measure(Some(WindowGeometry {
            window: a,
            surface: a,
            content: a,
            input_origin: (0., 0.),
            clearance: Default::default(),
            density: 1.,
        }));
        for (id, b) in [(1, Button::CUp), (2, Button::CRight)] {
            let (x, y) = view
                .zones
                .iter()
                .find(|z| z.target == Target::Button(b))
                .unwrap()
                .area
                .center();
            assert_eq!(view.down(id, x, y), Some(Target::Button(b)));
            view.up(id);
        }
        view.poll();
        assert_eq!(view.sampled.input_state(0, 1, 0, 9), 1);
        assert_eq!(view.sampled.input_state(0, 1, 0, 11), 1);
        view.poll();
        assert_eq!(view.sampled.input_state(0, 1, 0, 9), 0);
        let (x, y) = view
            .zones
            .iter()
            .find(|z| z.target == Target::Stick)
            .unwrap()
            .area
            .center();
        view.down(3, x, y);
        view.move_pointer(3, x + 30., y - 30.);
        view.poll();
        assert!(view.sampled.input_state(0, 5, 0, 0) > 0);
        view.cancel();
        view.poll();
        assert_eq!(view.sampled.input_state(0, 5, 0, 0), 0);
    }
}

#[cfg(test)]
mod picture_space_tests {
    use super::*;
    #[test]
    fn landscape_auxiliary_controls_leave_a_large_clear_aspect_correct_game_field() {
        let a = Area {
            left: 0.,
            top: 0.,
            right: 2280.,
            bottom: 1080.,
        };
        let mut view = View::default();
        view.measure(Some(WindowGeometry {
            window: a,
            surface: a,
            content: a,
            input_origin: (0., 0.),
            density: 2.75,
            clearance: crate::viewport::Insets {
                top: 66.,
                bottom: 66.,
                ..Default::default()
            },
        }));
        let p = view.picture(2280., 1080., 4. / 3.).unwrap();
        assert!(p.bottom - p.top >= 900., "game field lost priority: {p:?}");
        assert!(((p.right - p.left) / (p.bottom - p.top) - 4. / 3.).abs() < 0.0001);
        for (i, z) in view.zones.iter().enumerate() {
            assert!(!crate::controls::overlaps(p, z.area));
            assert!(
                view.zones[..i]
                    .iter()
                    .all(|other| !crate::controls::overlaps(z.area, other.area))
            );
        }
    }
}
