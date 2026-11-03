// SPDX-License-Identifier: MIT
//! N64 layout drafts and test input. Settings never own game saves or runtimes.
use crate::{
    controls::{Settings, display_key, finite, overlaps},
    n64::Pad,
    n64_touch::{Target, View, Zone},
    touch::Area,
    viewport::WindowGeometry,
};
use serde::{Deserialize, Serialize};
use std::io;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub display: String,
    pub zones: Vec<Placement>,
    /// Normalized game-field bounds within the complete drawable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<Area>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    pub target: Target,
    pub bounds: Area,
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Invalid N64 layout; original retained",
    )
}
pub(crate) fn validate_profiles(profiles: &[Profile]) -> io::Result<()> {
    if profiles.len() > 16 {
        return Err(invalid());
    }
    for (i, p) in profiles.iter().enumerate() {
        let size = p
            .display
            .split_once('x')
            .and_then(|(w, h)| Some((w.parse::<u16>().ok()?, h.parse::<u16>().ok()?)));
        if !matches!(size, Some((1..=8192, 1..=8192)))
            || p.display.len() > 32
            || !p.display.bytes().all(|c| c.is_ascii_digit() || c == b'x')
            || profiles[..i].iter().any(|other| other.display == p.display)
            || p.zones.len() > 16
            || p.picture
                .is_some_and(|a| !crate::layout_edit::valid_picture(a))
        {
            return Err(invalid());
        }
        for (j, z) in p.zones.iter().enumerate() {
            let a = z.bounds;
            if !finite(a)
                || a.left < 0.
                || a.top < 0.
                || a.right > 1.
                || a.bottom > 1.
                || a.left >= a.right
                || a.top >= a.bottom
                || p.zones[..j].iter().any(|other| other.target == z.target)
            {
                return Err(invalid());
            }
        }
    }
    Ok(())
}
fn usable(a: Area, w: f32, h: f32) -> bool {
    finite(a)
        && a.left >= 0.
        && a.top >= 0.
        && a.right <= w
        && a.bottom <= h
        && a.right - a.left + 0.01 >= 16_f32.min(w)
        && a.bottom - a.top + 0.01 >= 16_f32.min(h)
}
pub(crate) fn apply_profile(profiles: &[Profile], zones: &mut Vec<Zone>, safe: Area, density: f32) {
    let w = (safe.right - safe.left) / density;
    let h = (safe.bottom - safe.top) / density;
    let Some(profile) = profiles.iter().find(|p| p.display == display_key(w, h)) else {
        return;
    };
    let mut candidate = zones.clone();
    for placement in &profile.zones {
        let Some(zone) = candidate.iter_mut().find(|z| z.target == placement.target) else {
            return;
        };
        let a = placement.bounds;
        let local = Area {
            left: a.left * w,
            top: a.top * h,
            right: a.right * w,
            bottom: a.bottom * h,
        };
        if !usable(local, w, h) {
            return;
        }
        zone.area = Area {
            left: safe.left + local.left * density,
            top: safe.top + local.top * density,
            right: safe.left + local.right * density,
            bottom: safe.top + local.bottom * density,
        };
    }
    let local = |a: Area| Area {
        left: (a.left - safe.left) / density,
        top: (a.top - safe.top) / density,
        right: (a.right - safe.left) / density,
        bottom: (a.bottom - safe.top) / density,
    };
    if candidate.iter().enumerate().any(|(i, z)| {
        candidate[..i]
            .iter()
            .any(|other| overlaps(local(z.area), local(other.area)))
    }) {
        return;
    }
    *zones = candidate;
}
fn preview(width: f32, height: f32, settings: &Settings) -> View {
    let mut view = View::default();
    view.configure(settings);
    let a = Area {
        left: 0.,
        top: 0.,
        right: width,
        bottom: height,
    };
    view.measure(Some(WindowGeometry {
        window: a,
        surface: a,
        content: a,
        input_origin: (0., 0.),
        clearance: Default::default(),
        density: 1.,
    }));
    view
}
pub struct Editor {
    pub settings: Settings,
    pub zones: Vec<Zone>,
    pub width: f32,
    pub height: f32,
    pub selected: Option<Target>,
    pub testing: bool,
    pub rejected: bool,
    pub draft: crate::layout_edit::Draft,
    test: View,
}
impl Editor {
    pub fn new(width: f32, height: f32, settings: Settings) -> Self {
        let test = preview(width, height, &settings);
        let mut default_settings = settings.clone();
        default_settings.n64_profiles.clear();
        let defaults = preview(width, height, &default_settings);
        let layout = &test;
        let mut groups = vec![
            layout
                .zones
                .iter()
                .enumerate()
                .filter(|(_, z)| {
                    matches!(
                        z.target,
                        Target::Button(crate::n64::Button::A | crate::n64::Button::B)
                    )
                })
                .map(|(i, _)| i)
                .collect(),
        ];

        for buttons in [
            [
                crate::n64::Button::CUp,
                crate::n64::Button::CDown,
                crate::n64::Button::CLeft,
                crate::n64::Button::CRight,
            ],
            [
                crate::n64::Button::Up,
                crate::n64::Button::Down,
                crate::n64::Button::Left,
                crate::n64::Button::Right,
            ],
        ] {
            groups.push(
                layout
                    .zones
                    .iter()
                    .enumerate()
                    .filter(|(_, z)| matches!(z.target,Target::Button(b) if buttons.contains(&b)))
                    .map(|(i, _)| i)
                    .collect(),
            );
        }

        groups.retain(|g: &Vec<usize>| g.len() > 1);
        let picture = settings
            .n64_profiles
            .iter()
            .find(|p| p.display == display_key(width, height))
            .and_then(|p| p.picture);
        let draft = crate::layout_edit::Draft::new(
            width,
            height,
            layout.zones.iter().map(|z| z.area).collect(),
            defaults.zones.iter().map(|z| z.area).collect(),
            groups,
            picture,
            16.,
        );

        Self {
            zones: test.zones.clone(),
            test,
            settings,
            width,
            height,
            selected: None,
            testing: false,
            rejected: false,
            draft,
        }
    }
    pub fn sync(&mut self) {
        for (zone, area) in self.zones.iter_mut().zip(self.draft.areas()) {
            zone.area = *area;
        }
        self.selected = match self.draft.selected {
            Some(crate::layout_edit::Selection::Control(i)) => self.zones.get(i).map(|z| z.target),
            _ => None,
        };
        self.rejected = self.draft.rejected;
    }
    pub fn select(&mut self, target: Target) {
        self.draft.cancel();
        self.draft.selected = self
            .zones
            .iter()
            .position(|z| z.target == target)
            .map(crate::layout_edit::Selection::Control);
        self.sync();
    }
    pub fn down(&mut self, id: i32, x: f32, y: f32) -> bool {
        if self.testing {
            return self.test.down(id, x, y).is_some();
        }
        let handled = self.draft.down(id, x, y);
        self.sync();
        handled
    }
    pub fn moved(&mut self, id: i32, x: f32, y: f32) {
        if self.testing {
            self.test.move_pointer(id, x, y);
        } else {
            self.draft.moved(id, x, y);
            self.sync();
        }
    }
    pub fn up(&mut self, id: i32) {
        self.test.up(id);
        self.draft.up(id);
        self.sync();
    }
    pub fn cancel(&mut self) {
        self.draft.cancel();
        self.test.cancel();
        self.sync();
    }
    pub fn toggle_test(&mut self) {
        self.cancel();
        self.testing = !self.testing;
        self.test.zones = self.zones.clone();
    }
    pub fn configure_test(&mut self) {
        self.test.configure_dead_zone(self.settings.dead_zone);
    }
    pub fn pad(&mut self) -> Pad {
        self.test.poll();
        self.test.sampled.clone()
    }
    pub fn resize(&mut self, delta: f32) {
        if !self.testing && delta.is_finite() {
            self.draft.resize(delta);
            self.sync();
        }
    }
    pub fn reset_display(&mut self) {
        self.cancel();
        self.draft.reset_display();
        self.sync();
    }
    pub fn finish(&mut self) -> io::Result<Settings> {
        self.cancel();
        let zones = self
            .zones
            .iter()
            .enumerate()
            .filter(|(i, z)| z.area != self.draft.default_area(*i))
            .map(|(_, z)| Placement {
                target: z.target,
                bounds: crate::layout_edit::normalized(
                    z.area,
                    Area {
                        left: 0.,
                        top: 0.,
                        right: self.width,
                        bottom: self.height,
                    },
                ),
            })
            .collect::<Vec<_>>();
        let key = display_key(self.width, self.height);
        self.settings.n64_profiles.retain(|p| p.display != key);
        let picture = self.draft.picture_preference();
        if !zones.is_empty() || picture.is_some() {
            self.settings.n64_profiles.push(Profile {
                display: key,
                zones,
                picture,
            });
        }
        self.settings.validate()?;
        Ok(self.settings.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        controls::{Placement as GcPlacement, Profile as GcProfile},
        n64::Button,
        touch::Target as GcTarget,
    };
    #[test]
    fn runtime_reload_uses_density_and_origin_and_cancels_old_contacts_atomically() {
        let settings = Settings::default();
        let mut draft = Editor::new(840., 420., settings.clone());
        let target = Target::Pause;
        let a = draft
            .zones
            .iter()
            .find(|z| z.target == target)
            .unwrap()
            .area;
        let (x, y) = a.center();
        assert!(draft.down(7, x, y));
        draft.moved(7, x + 80., y);
        draft.up(7);
        let saved = draft.finish().unwrap();
        let mut runtime = preview(840., 420., &settings);
        let stick = runtime.zones[0].area.center();
        runtime.down(1, stick.0 + 20., stick.1);
        runtime.poll();
        assert!(runtime.sampled.input_state(0, 5, 0, 0) > 0);
        runtime.configure(&saved);
        runtime.poll();
        assert_eq!(runtime.sampled.input_state(0, 5, 0, 0), 0);
        let changed = runtime
            .zones
            .iter()
            .find(|z| z.target == target)
            .unwrap()
            .area;
        let bounds = Area {
            left: 13.,
            top: 7.,
            right: 1693.,
            bottom: 847.,
        };
        let geometry = WindowGeometry {
            window: bounds,
            surface: bounds,
            content: bounds,
            input_origin: (13., 7.),
            clearance: Default::default(),
            density: 2.,
        };
        runtime.measure(Some(geometry));
        let scaled = runtime
            .zones
            .iter()
            .find(|z| z.target == target)
            .unwrap()
            .area;
        assert!((scaled.left - (13. + changed.left * 2.)).abs() < 0.001);
        assert!((scaled.top - (7. + changed.top * 2.)).abs() < 0.001);
        let mut invalid = saved.clone();
        let overlap = invalid.n64_profiles[0].zones[0].bounds;
        invalid.n64_profiles[0].zones.push(Placement {
            target: Target::Button(Button::A),
            bounds: overlap,
        });
        runtime.configure(&invalid);
        let mut defaults = View::default();
        defaults.configure(&settings);
        defaults.measure(Some(geometry));
        assert_eq!(
            runtime.zones.iter().map(|z| z.area).collect::<Vec<_>>(),
            defaults.zones.iter().map(|z| z.area).collect::<Vec<_>>()
        );
        assert_eq!(invalid.n64_profiles[0].zones.len(), 2);
    }
    #[test]
    fn saved_n64_draft_is_independent_of_gc_and_test_input_never_leaks_after_cancel() {
        let mut settings = Settings::default();
        settings.profiles.push(GcProfile {
            picture: None,
            display: display_key(840., 420.),
            zones: vec![GcPlacement {
                target: GcTarget::Pause,
                bounds: Area {
                    left: 0.4,
                    top: 0.05,
                    right: 0.45,
                    bottom: 0.15,
                },
            }],
        });
        let gc = settings.profiles.clone();
        let unchanged = settings.clone();
        let mut e = Editor::new(840., 420., settings);
        let a = e
            .zones
            .iter()
            .find(|z| z.target == Target::Pause)
            .unwrap()
            .area;
        let (x, y) = a.center();
        assert!(e.down(7, x, y));
        e.moved(9, x + 80., y);
        assert_eq!(
            e.zones
                .iter()
                .find(|z| z.target == Target::Pause)
                .unwrap()
                .area,
            a
        );
        e.moved(7, x + 80., y);
        e.up(7);
        e.resize(4.);
        assert!(!e.rejected);
        let saved = e.finish().unwrap();
        assert_eq!(saved.profiles, gc);
        assert_eq!(unchanged.n64_profiles.len(), 0);
        let decoded: Settings =
            serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
        decoded.validate().unwrap();
        let mut reopened = Editor::new(840., 420., decoded);
        for (actual, expected) in reopened.zones.iter().zip(&e.zones) {
            assert_eq!(actual.target, expected.target);
            let a = actual.area;
            let b = expected.area;
            assert!(
                [
                    a.left - b.left,
                    a.top - b.top,
                    a.right - b.right,
                    a.bottom - b.bottom
                ]
                .iter()
                .all(|v| v.abs() < 0.001)
            );
        }
        let root = std::env::temp_dir().join(format!(
            "ntx-n64-controls-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let mut store = crate::n64_save::SaveStore::open(&root.join("profiles/game")).unwrap();
        let image = vec![0x6a; crate::n64::SAVE_IMAGE_BYTES];
        let generation = store.commit(&image).unwrap();
        saved.save(&root).unwrap();
        assert_eq!(Settings::load(&root).unwrap(), saved);
        assert_eq!(store.generation(), Some(&generation));
        assert_eq!(store.load().unwrap(), Some(image));
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
        reopened.toggle_test();
        let stick = reopened
            .zones
            .iter()
            .find(|z| z.target == Target::Stick)
            .unwrap()
            .area
            .center();
        let a = reopened
            .zones
            .iter()
            .find(|z| z.target == Target::Button(Button::A))
            .unwrap()
            .area
            .center();
        assert!(reopened.down(1, stick.0 + 20., stick.1));
        assert!(reopened.down(2, a.0, a.1));
        let pad = reopened.pad();
        assert!(pad.pressed(Button::A));
        assert!(pad.input_state(0, 5, 0, 0) > 0);
        reopened.up(2);
        assert!(!reopened.pad().pressed(Button::A));
        assert!(reopened.pad().input_state(0, 5, 0, 0) > 0);
        reopened.cancel();
        let pad = reopened.pad();
        assert!(!pad.pressed(Button::A));
        assert_eq!(pad.input_state(0, 5, 0, 0), 0);
        reopened.reset_display();
        assert!(reopened.finish().unwrap().n64_profiles.is_empty());
        assert_eq!(reopened.settings.profiles, gc);
    }
}
