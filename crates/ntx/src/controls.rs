// SPDX-License-Identifier: MIT
//! Player preferences and a non-emulating touch-layout editor. Never owns saves.
use crate::touch::{Area, Layout, Target, TouchPad};
use serde::{Deserialize, Serialize};
use std::{io, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub version: u8,
    pub opacity: u8,
    pub dead_zone: u8,
    pub haptics: bool,
    pub floating_sticks: bool,
    pub profiles: Vec<Profile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub n64_profiles: Vec<crate::n64_controls::Profile>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            opacity: 85,
            dead_zone: 12,
            haptics: true,
            floating_sticks: true,
            profiles: Vec::new(),
            n64_profiles: Vec::new(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub display: String,
    pub zones: Vec<Placement>,
    /// Normalized game-field bounds within the complete drawable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<Area>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    pub target: Target,
    /// Fractions of this display's actual control-safe rectangle, never pixels.
    pub bounds: Area,
}
pub fn display_key(width: f32, height: f32) -> String {
    let points = |n: f32| ((n.clamp(1., 8192.) / 8.).round() * 8.) as u32;
    format!("{}x{}", points(width), points(height))
}
pub(crate) fn finite(a: Area) -> bool {
    [a.left, a.top, a.right, a.bottom]
        .iter()
        .all(|v| v.is_finite())
}
pub(crate) fn overlaps(a: Area, b: Area) -> bool {
    a.left < b.right - 0.25
        && a.right > b.left + 0.25
        && a.top < b.bottom - 0.25
        && a.bottom > b.top + 0.25
}
impl Settings {
    pub fn validate(&self) -> io::Result<()> {
        crate::n64_controls::validate_profiles(&self.n64_profiles)?;
        let invalid = || {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid controls settings; original retained",
            )
        };
        if self.version != 1
            || !(30..=100).contains(&self.opacity)
            || self.dead_zone > 25
            || self.profiles.len() > 16
        {
            return Err(invalid());
        }
        for (i, profile) in self.profiles.iter().enumerate() {
            if profile.display.len() > 32
                || !profile
                    .display
                    .bytes()
                    .all(|c| c.is_ascii_digit() || c == b'x')
                || !profile.display.contains('x')
                || self.profiles[..i]
                    .iter()
                    .any(|p| p.display == profile.display)
                || profile.zones.len() > 20
                || profile
                    .picture
                    .is_some_and(|a| !crate::layout_edit::valid_picture(a))
            {
                return Err(invalid());
            }
            for (j, placement) in profile.zones.iter().enumerate() {
                let a = placement.bounds;
                if !finite(a)
                    || !(0. ..=1.).contains(&a.left)
                    || !(0. ..=1.).contains(&a.top)
                    || !(0. ..=1.).contains(&a.right)
                    || !(0. ..=1.).contains(&a.bottom)
                    || a.right <= a.left
                    || a.bottom <= a.top
                    || profile.zones[..j]
                        .iter()
                        .any(|z| z.target == placement.target)
                {
                    return Err(invalid());
                }
            }
        }
        Ok(())
    }
    pub fn load(root: &Path) -> io::Result<Self> {
        let path = root.join("controls.json");
        let file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e),
        };
        if file.metadata()?.len() > 65536 {
            return Err(io::Error::other(
                "Controls settings exceed size limit; original retained",
            ));
        }
        let settings: Self = serde_json::from_reader(file).map_err(io::Error::other)?;
        settings.validate()?;
        Ok(settings)
    }
    pub fn save(&self, root: &Path) -> io::Result<()> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)?;
        if bytes.len() > 65536 {
            return Err(io::Error::other(
                "Controls settings exceed size limit; original retained",
            ));
        }
        crate::storage::atomic_replace(&root.join("controls.json"), &bytes)
    }
    pub fn layout(&self, width: f32, height: f32) -> Layout {
        let mut layout = Layout::new(width, height);
        let key = display_key(width, height);
        if let Some(profile) = self.profiles.iter().find(|p| p.display == key) {
            // Admit the complete profile atomically. A transient tiny drawable
            // or malformed placement never partly shifts drawn/hit targets.
            let mut candidate = layout.clone();
            let mut valid = true;
            for placement in &profile.zones {
                let Some(zone) = candidate
                    .zones
                    .iter_mut()
                    .find(|z| z.target == placement.target)
                else {
                    valid = false;
                    break;
                };
                let minimum = 48_f32
                    .min(zone.area.right - zone.area.left)
                    .min(zone.area.bottom - zone.area.top);
                let a = placement.bounds;
                zone.area = Area {
                    left: a.left * width,
                    top: a.top * height,
                    right: a.right * width,
                    bottom: a.bottom * height,
                };
                if !usable(zone.area, width, height, minimum) {
                    valid = false;
                    break;
                }
            }
            if valid
                && profile.zones.iter().all(|p| {
                    let a = candidate
                        .zones
                        .iter()
                        .find(|z| z.target == p.target)
                        .unwrap()
                        .area;
                    candidate
                        .zones
                        .iter()
                        .all(|z| z.target == p.target || !overlaps(a, z.area))
                })
            {
                layout = candidate;
            }
        }
        layout
    }
}
fn usable(a: Area, w: f32, h: f32, minimum: f32) -> bool {
    finite(a)
        && a.left >= 0.
        && a.top >= 0.
        && a.right <= w
        && a.bottom <= h
        && a.right - a.left + 0.01 >= minimum.min(w)
        && a.bottom - a.top + 0.01 >= minimum.min(h)
}
/// The editor cannot emit game input. Its test pad is independent of LiveRuntime.
pub struct Editor {
    pub settings: Settings,
    pub layout: Layout,
    pub width: f32,
    pub height: f32,
    pub selected: Option<Target>,
    pub testing: bool,
    pub rejected: bool,
    pub draft: crate::layout_edit::Draft,
    test_pad: TouchPad,
}
impl Editor {
    pub fn new(width: f32, height: f32, settings: Settings) -> Self {
        let layout = settings.layout(width, height);
        let defaults = Layout::new(width, height);
        let mut groups = vec![
            layout
                .zones
                .iter()
                .enumerate()
                .filter(|(_, z)| {
                    matches!(
                        z.target,
                        Target::Button(
                            crate::Control::A
                                | crate::Control::B
                                | crate::Control::X
                                | crate::Control::Y
                        )
                    )
                })
                .map(|(i, _)| i)
                .collect(),
        ];

        groups.retain(|g: &Vec<usize>| g.len() > 1);
        let picture = settings
            .profiles
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
            48.,
        );
        let mut test_pad = TouchPad::new();
        test_pad.configure(settings.dead_zone, settings.floating_sticks);
        Self {
            layout,
            test_pad,
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
        for (zone, area) in self.layout.zones.iter_mut().zip(self.draft.areas()) {
            zone.area = *area;
        }
        self.selected = match self.draft.selected {
            Some(crate::layout_edit::Selection::Control(i)) => {
                self.layout.zones.get(i).map(|z| z.target)
            }
            _ => None,
        };
        self.rejected = self.draft.rejected;
    }
    pub fn select(&mut self, target: Target) {
        self.draft.cancel();
        self.draft.selected = self
            .layout
            .zones
            .iter()
            .position(|z| z.target == target)
            .map(crate::layout_edit::Selection::Control);
        self.sync();
    }
    pub fn down(&mut self, id: i32, x: f32, y: f32) -> bool {
        if self.testing {
            return matches!(
                self.test_pad.down_report(id, x, y, &self.layout),
                crate::touch::DownReport::Captured | crate::touch::DownReport::Pause
            );
        }
        let handled = self.draft.down(id, x, y);
        self.sync();
        handled
    }
    pub fn moved(&mut self, id: i32, x: f32, y: f32) {
        if self.testing {
            self.test_pad.moved(id, x, y);
        } else {
            self.draft.moved(id, x, y);
            self.sync();
        }
    }
    pub fn up(&mut self, id: i32) {
        self.test_pad.up(id);
        self.draft.up(id);
        self.sync();
    }
    pub fn cancel(&mut self) {
        self.draft.cancel();
        self.test_pad.cancel();
        self.sync();
    }
    pub fn toggle_test(&mut self) {
        self.cancel();
        self.testing = !self.testing;
    }
    pub fn configure_test(&mut self) {
        self.test_pad
            .configure(self.settings.dead_zone, self.settings.floating_sticks);
    }
    pub fn pad(&self) -> crate::Pad {
        self.test_pad.pad()
    }
    pub fn resize(&mut self, delta: f32) {
        if !self.testing && delta.is_finite() {
            self.draft.resize(delta);
            self.sync();
        }
    }
    #[cfg(test)]
    fn place(&mut self, area: Area) {
        if let Some(target) = self.selected {
            let i = self
                .layout
                .zones
                .iter()
                .position(|z| z.target == target)
                .unwrap();
            self.draft.selected = Some(crate::layout_edit::Selection::Control(i));
            self.draft.set_selected_area(area);
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
            .layout
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
        self.settings.profiles.retain(|p| p.display != key);
        let picture = self.draft.picture_preference();
        if !zones.is_empty() || picture.is_some() {
            self.settings.profiles.push(Profile {
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
    use crate::{Control, touch::DownReport};
    #[test]
    fn preferences_round_trip_without_touching_game_data_and_corrupt_original_is_retained() {
        let root = std::env::temp_dir().join(format!(
            "ntx-controls-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("card-fixture"), b"ordinary progress").unwrap();
        let settings = Settings {
            dead_zone: 5,
            haptics: false,
            ..Settings::default()
        };
        settings.save(&root).unwrap();
        assert_eq!(Settings::load(&root).unwrap(), settings);
        std::fs::write(root.join("controls.json"), b"broken settings retained").unwrap();
        assert!(Settings::load(&root).is_err());
        assert_eq!(
            std::fs::read(root.join("controls.json")).unwrap(),
            b"broken settings retained"
        );
        assert_eq!(
            std::fs::read(root.join("card-fixture")).unwrap(),
            b"ordinary progress"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn editor_moves_one_target_and_rejects_collision_without_changing_other_controls() {
        let mut e = Editor::new(840., 420., Settings::default());
        let target = Target::Pause;
        let a = e
            .layout
            .zones
            .iter()
            .find(|z| z.target == target)
            .unwrap()
            .area;
        let old = e.layout.clone();
        let (x, y) = a.center();
        e.down(3, x, y);
        e.moved(8, x + 20., y); // other pointer cannot edit
        assert_eq!(
            e.layout
                .zones
                .iter()
                .find(|z| z.target == target)
                .unwrap()
                .area,
            a
        );
        e.moved(3, 420., 220.);
        e.up(3);
        assert!(!e.rejected);
        assert_eq!(
            e.layout
                .zones
                .iter()
                .filter(|z| z.target != target)
                .map(|z| z.area)
                .collect::<Vec<_>>(),
            old.zones
                .iter()
                .filter(|z| z.target != target)
                .map(|z| z.area)
                .collect::<Vec<_>>()
        );
        let good = e
            .layout
            .zones
            .iter()
            .find(|z| z.target == target)
            .unwrap()
            .area;
        e.select(target);
        let other = e
            .layout
            .zones
            .iter()
            .find(|z| z.target == Target::Button(Control::A))
            .unwrap()
            .area;
        e.place(other);
        assert!(e.rejected);
        assert_eq!(
            e.layout
                .zones
                .iter()
                .find(|z| z.target == target)
                .unwrap()
                .area,
            good
        );
    }
    #[test]
    fn saved_positions_are_per_display_and_reset_is_local() {
        let mut e = Editor::new(840., 420., Settings::default());
        e.select(Target::Pause);
        e.place(Area {
            left: 390.,
            top: 190.,
            right: 438.,
            bottom: 238.,
        });
        let settings = e.finish().unwrap();
        assert_eq!(settings.profiles.len(), 1);
        assert_eq!(
            settings
                .layout(840., 420.)
                .zones
                .iter()
                .find(|z| z.target == Target::Pause)
                .unwrap()
                .area
                .left,
            390.
        );
        assert_eq!(
            settings
                .layout(1000., 800.)
                .zones
                .iter()
                .find(|z| z.target == Target::Pause)
                .unwrap()
                .area,
            Layout::new(1000., 800.)
                .zones
                .iter()
                .find(|z| z.target == Target::Pause)
                .unwrap()
                .area
        );
        let mut e = Editor::new(1000., 800., settings);
        e.reset_display();
        assert_eq!(e.finish().unwrap().profiles.len(), 1);
        let mut e = Editor::new(840., 420., e.settings);
        e.reset_display();
        assert!(e.finish().unwrap().profiles.is_empty());
    }
    #[test]
    fn malformed_preferences_and_unsafe_rectangles_are_rejected_or_fall_back_as_one_profile() {
        let mut s = Settings {
            opacity: 0,
            ..Settings::default()
        };
        assert!(s.validate().is_err());
        s.opacity = 85;
        s.dead_zone = 100;
        assert!(s.validate().is_err());
        s.dead_zone = 12;
        s.profiles.push(Profile {
            picture: None,
            display: display_key(840., 420.),
            zones: vec![Placement {
                target: Target::Pause,
                bounds: Area {
                    left: 0.,
                    top: 0.,
                    right: 0.001,
                    bottom: 0.001,
                },
            }],
        });
        assert_eq!(
            s.layout(840., 420.)
                .zones
                .iter()
                .map(|z| z.area)
                .collect::<Vec<_>>(),
            Layout::new(840., 420.)
                .zones
                .iter()
                .map(|z| z.area)
                .collect::<Vec<_>>()
        );
        s.profiles[0].zones[0].bounds.left = f32::NAN;
        assert!(s.validate().is_err());
    }
    #[test]
    fn test_preview_has_an_independent_pad_and_partial_trigger_does_not_click() {
        let mut e = Editor::new(840., 420., Settings::default());
        e.toggle_test();
        let a = e
            .layout
            .zones
            .iter()
            .find(|z| z.target == Target::Travel(Control::LTravel))
            .unwrap()
            .area;
        e.down(1, (a.left + a.right) / 2., (a.top + a.bottom) / 2.);
        assert!((e.pad().values()[Control::LTravel as usize] - 0.5).abs() < 0.01);
        assert_eq!(e.pad().values()[Control::LClick as usize], 0.);
        assert_ne!(
            e.test_pad.down_report(2, a.left, a.top, &e.layout),
            DownReport::Captured
        );
        e.toggle_test();
        assert_eq!(e.pad().values(), &[0.; 18]);
    }
}

#[cfg(test)]
mod picture_tests {
    use super::*;
    use crate::{
        layout_edit::{Selection, denormalized, fit},
        viewport::{Geometry, Insets, WindowGeometry},
    };
    #[test]
    fn game_field_round_trip_reaches_both_renderers_with_full_drawable_and_distinct_profiles() {
        // Existing v1 settings have no picture member and continue to decode.
        let old: Settings =
            serde_json::from_str(r#"{"version":1,"profiles":[],"n64_profiles":[]}"#).unwrap();
        let frame = Area {
            left: -20.,
            top: -12.,
            right: 860.,
            bottom: 432.,
        };
        let mut gc = Editor::new(840., 420., old);
        gc.draft.frame = frame;
        gc.draft.aspect = 1.25;
        gc.draft.selected = Some(Selection::Picture);
        gc.resize(-100.);
        let gc_picture = gc.draft.picture();
        let settings = gc.finish().unwrap();
        assert_eq!(settings.profiles.len(), 1);
        assert!(settings.profiles[0].zones.is_empty());
        let mut n = crate::n64_controls::Editor::new(840., 420., settings);
        n.draft.frame = frame;
        n.draft.selected = Some(Selection::Picture);
        n.resize(-80.);
        let n_picture = n.draft.picture();
        let settings = n.finish().unwrap();
        let bytes = serde_json::to_vec(&settings).unwrap();
        let settings: Settings = serde_json::from_slice(&bytes).unwrap();
        settings.validate().unwrap();
        assert_eq!(settings.profiles.len(), 1);
        assert_eq!(settings.n64_profiles.len(), 1);
        let bounds = Area {
            left: 0.,
            top: 0.,
            right: 1760.,
            bottom: 888.,
        };
        let w = WindowGeometry {
            window: bounds,
            surface: bounds,
            content: bounds,
            input_origin: (0., 0.),
            density: 2.,
            clearance: Insets {
                left: 40.,
                right: 40.,
                top: 24.,
                bottom: 24.,
            },
        };
        let g = Geometry::with_controls(w, 880., 444., Some(&settings)).unwrap();
        let p = g.picture(1.25);
        assert_eq!(
            p,
            Area {
                left: gc_picture.left + 20.,
                top: gc_picture.top + 12.,
                right: gc_picture.right + 20.,
                bottom: gc_picture.bottom + 12.
            }
        );
        let mut view = crate::n64_touch::View::default();
        view.configure(&settings);
        view.measure(Some(w));
        let p = view.picture(880., 444., 4. / 3.).unwrap();
        assert_eq!(
            p,
            Area {
                left: n_picture.left + 20.,
                top: n_picture.top + 12.,
                right: n_picture.right + 20.,
                bottom: n_picture.bottom + 12.
            }
        );
        let reloaded = Editor::new(840., 420., settings.clone());
        assert_eq!(
            reloaded.draft.picture_preference(),
            settings.profiles[0].picture
        );
        assert_eq!(
            fit(
                denormalized(settings.profiles[0].picture.unwrap(), frame),
                1.25
            ),
            gc_picture
        );
        let mut invalid = settings;
        invalid.n64_profiles[0].picture.as_mut().unwrap().right = f32::INFINITY;
        assert!(invalid.validate().is_err());
    }
}

#[cfg(test)]
mod short_editor_tests {
    use super::*;
    #[test]
    fn pixel_short_screen_group_move_round_trips_scaled_default_caps() {
        let mut e = Editor::new(832., 328., Settings::default());
        e.draft.grid = false;
        let original = e.layout.clone();
        let a = original
            .zones
            .iter()
            .find(|z| z.target == Target::Button(crate::Control::A))
            .unwrap()
            .area;
        let (x, y) = a.center();
        e.down(1, x, y);
        e.moved(1, x, y + 12.);
        e.up(1);
        assert!(!e.rejected, "default scaled caps must remain movable");
        let faces = |t| {
            matches!(
                t,
                Target::Button(
                    crate::Control::A | crate::Control::B | crate::Control::X | crate::Control::Y
                )
            )
        };
        for (old, new) in original.zones.iter().zip(&e.layout.zones) {
            assert!(
                (new.area.top - old.area.top - if faces(old.target) { 12. } else { 0. }).abs()
                    < 0.001
            );
        }
        let expected = e.layout.clone();
        let saved = e.finish().unwrap();
        let loaded = saved.layout(832., 328.);
        for (a, b) in loaded.zones.iter().zip(expected.zones) {
            assert!((a.area.top - b.area.top).abs() < 0.001);
        }
        assert_eq!(saved.profiles[0].zones.len(), 4);
    }
}
