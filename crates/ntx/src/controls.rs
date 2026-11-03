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
                let a = placement.bounds;
                zone.area = Area {
                    left: a.left * width,
                    top: a.top * height,
                    right: a.right * width,
                    bottom: a.bottom * height,
                };
                if !usable(zone.area, width, height) {
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
fn usable(a: Area, w: f32, h: f32) -> bool {
    finite(a)
        && a.left >= 0.
        && a.top >= 0.
        && a.right <= w
        && a.bottom <= h
        && a.right - a.left + 0.01 >= 48_f32.min(w)
        && a.bottom - a.top + 0.01 >= 48_f32.min(h)
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
    drag: Option<(i32, f32, f32, Area)>,
    test_pad: TouchPad,
}
impl Editor {
    pub fn new(width: f32, height: f32, settings: Settings) -> Self {
        let mut test_pad = TouchPad::new();
        test_pad.configure(settings.dead_zone, settings.floating_sticks);
        Self {
            layout: settings.layout(width, height),
            settings,
            width,
            height,
            selected: None,
            testing: false,
            rejected: false,
            drag: None,
            test_pad,
        }
    }
    pub fn select(&mut self, target: Target) {
        self.selected = Some(target);
        self.drag = None;
    }
    pub fn down(&mut self, pointer: i32, x: f32, y: f32) -> bool {
        if self.testing {
            return matches!(
                self.test_pad.down_report(pointer, x, y, &self.layout),
                crate::touch::DownReport::Captured | crate::touch::DownReport::Pause
            );
        }
        if self.drag.is_some() || !x.is_finite() || !y.is_finite() {
            return false;
        }
        if let Some(zone) = self
            .layout
            .zones
            .iter()
            .rev()
            .find(|z| z.area.contains(x, y))
        {
            self.selected = Some(zone.target);
            self.drag = Some((pointer, x, y, zone.area));
            self.rejected = false;
            return true;
        }
        false
    }
    pub fn moved(&mut self, pointer: i32, x: f32, y: f32) {
        if self.testing {
            self.test_pad.moved(pointer, x, y);
            return;
        }
        if let Some((id, ox, oy, a)) = self.drag.filter(|(id, ..)| *id == pointer) {
            let _ = id;
            if !x.is_finite() || !y.is_finite() {
                return;
            }
            let dx = (x - ox).clamp(-a.left, self.width - a.right);
            let dy = (y - oy).clamp(-a.top, self.height - a.bottom);
            self.place(Area {
                left: a.left + dx,
                top: a.top + dy,
                right: a.right + dx,
                bottom: a.bottom + dy,
            });
        }
    }
    pub fn up(&mut self, pointer: i32) {
        self.test_pad.up(pointer);
        if self.drag.is_some_and(|(id, ..)| id == pointer) {
            self.drag = None;
        }
    }
    pub fn cancel(&mut self) {
        self.drag = None;
        self.test_pad.cancel();
    }
    pub fn toggle_test(&mut self) {
        self.cancel();
        self.testing = !self.testing;
    }
    pub fn pad(&self) -> crate::Pad {
        self.test_pad.pad()
    }
    pub fn configure_test(&mut self) {
        self.test_pad
            .configure(self.settings.dead_zone, self.settings.floating_sticks);
    }
    pub fn resize(&mut self, delta: f32) {
        let Some(zone) = self
            .layout
            .zones
            .iter()
            .find(|z| Some(z.target) == self.selected)
        else {
            return;
        };
        let a = zone.area;
        let (cx, cy) = a.center();
        let w = (a.right - a.left + delta).max(48.).min(self.width);
        let h = (a.bottom - a.top + delta).max(48.).min(self.height);
        self.place(Area {
            left: cx - w / 2.,
            top: cy - h / 2.,
            right: cx + w / 2.,
            bottom: cy + h / 2.,
        });
    }
    fn place(&mut self, area: Area) {
        let Some(target) = self.selected else {
            return;
        };
        if !usable(area, self.width, self.height)
            || self
                .layout
                .zones
                .iter()
                .any(|z| z.target != target && overlaps(area, z.area))
        {
            self.rejected = true;
            return;
        }
        if let Some(zone) = self.layout.zones.iter_mut().find(|z| z.target == target) {
            zone.area = area;
            self.rejected = false;
        }
    }
    pub fn reset_display(&mut self) {
        self.cancel();
        self.layout = Layout::new(self.width, self.height);
        self.selected = None;
        self.settings
            .profiles
            .retain(|p| p.display != display_key(self.width, self.height));
        self.rejected = false;
    }
    pub fn finish(&mut self) -> io::Result<Settings> {
        self.cancel();
        let original = Layout::new(self.width, self.height);
        let zones = self
            .layout
            .zones
            .iter()
            .zip(&original.zones)
            .filter(|(a, b)| a.area != b.area)
            .map(|(z, _)| {
                let a = z.area;
                Placement {
                    target: z.target,
                    bounds: Area {
                        left: a.left / self.width,
                        top: a.top / self.height,
                        right: a.right / self.width,
                        bottom: a.bottom / self.height,
                    },
                }
            })
            .collect::<Vec<_>>();
        let key = display_key(self.width, self.height);
        self.settings.profiles.retain(|p| p.display != key);
        if !zones.is_empty() {
            self.settings.profiles.push(Profile {
                display: key,
                zones,
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
