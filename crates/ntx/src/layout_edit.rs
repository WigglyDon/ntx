// SPDX-License-Identifier: MIT
//! Console-independent layout gestures. Indices address a draft, never game input.
use crate::{
    controls::{finite, overlaps},
    touch::Area,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    Control(usize),
    Picture,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Move,
    Resize,
}
#[derive(Clone, PartialEq)]
struct State {
    areas: Vec<Area>,
    picture: Option<Area>,
}
struct Drag {
    id: i32,
    x: f32,
    y: f32,
    before: State,
    bounds: Area,
    corner: Option<usize>,
}
pub struct Draft {
    state: State,
    defaults: Vec<Area>,
    groups: Vec<Vec<usize>>,
    pub selected: Option<Selection>,
    pub mode: Mode,
    pub grid: bool,
    pub grouped: bool,
    pub rejected: bool,
    pub guides: (Option<f32>, Option<f32>),
    pub frame: Area,
    pub aspect: f32,
    width: f32,
    height: f32,
    minimum: f32,
    history: Vec<State>,
    drag: Option<Drag>,
}
pub fn fit(a: Area, aspect: f32) -> Area {
    let aspect = if aspect.is_finite() && aspect > 0. {
        aspect
    } else {
        1.
    };
    let w = (a.right - a.left).min((a.bottom - a.top) * aspect).max(0.);
    let h = w / aspect;
    let (x, y) = a.center();
    Area {
        left: x - w / 2.,
        top: y - h / 2.,
        right: x + w / 2.,
        bottom: y + h / 2.,
    }
}
pub fn normalized(a: Area, frame: Area) -> Area {
    let w = frame.right - frame.left;
    let h = frame.bottom - frame.top;
    Area {
        left: (a.left - frame.left) / w,
        top: (a.top - frame.top) / h,
        right: (a.right - frame.left) / w,
        bottom: (a.bottom - frame.top) / h,
    }
}
pub fn denormalized(a: Area, frame: Area) -> Area {
    let w = frame.right - frame.left;
    let h = frame.bottom - frame.top;
    Area {
        left: frame.left + a.left * w,
        top: frame.top + a.top * h,
        right: frame.left + a.right * w,
        bottom: frame.top + a.bottom * h,
    }
}
pub fn valid_picture(a: Area) -> bool {
    finite(a)
        && a.left >= 0.
        && a.top >= 0.
        && a.right <= 1.
        && a.bottom <= 1.
        && a.right > a.left
        && a.bottom > a.top
}
/// Largest centered, uncropped picture clear of actual controls (PSX MIT concept).
pub fn clear_picture<I>(a: Area, zones: I, aspect: f32) -> Area
where
    I: IntoIterator<Item = Area>,
    I::IntoIter: Clone,
{
    let aspect = if aspect.is_finite() && aspect > 0. {
        aspect
    } else {
        1.
    };
    let zones = zones.into_iter();
    let center = a.center().0;
    let mut best = Area {
        left: center,
        top: a.top,
        right: center,
        bottom: a.top,
    };
    for top in std::iter::once(a.top)
        .chain(zones.clone().map(|z| z.bottom))
        .filter(|y| *y >= a.top && *y <= a.bottom)
    {
        let mut width = (a.right - a.left).min((a.bottom - top) * aspect).max(0.);
        for r in zones.clone() {
            if r.bottom <= top + 0.01 {
                continue;
            }
            let beside = (center - r.right).max(r.left - center).max(0.) * 2.;
            width = width.min(beside.max((r.top - top).max(0.) * aspect));
        }
        if width > best.right - best.left {
            best = Area {
                left: center - width / 2.,
                top,
                right: center + width / 2.,
                bottom: top + width / aspect,
            };
        }
    }
    best
}
impl Draft {
    pub fn new(
        width: f32,
        height: f32,
        areas: Vec<Area>,
        defaults: Vec<Area>,
        groups: Vec<Vec<usize>>,
        picture: Option<Area>,
        minimum: f32,
    ) -> Self {
        Self {
            state: State { areas, picture },
            defaults,
            groups,
            selected: None,
            mode: Mode::Move,
            grid: true,
            grouped: true,
            rejected: false,
            guides: (None, None),
            frame: Area {
                left: 0.,
                top: 0.,
                right: width,
                bottom: height,
            },
            aspect: 4. / 3.,
            width,
            height,
            minimum,
            history: Vec::new(),
            drag: None,
        }
    }
    pub fn areas(&self) -> &[Area] {
        &self.state.areas
    }
    pub fn default_area(&self, index: usize) -> Area {
        self.defaults[index]
    }
    #[cfg(test)]
    pub fn set_selected_area(&mut self, area: Area) {
        let Some(from) = self.selection_bounds() else {
            return;
        };
        let before = self.state.clone();
        self.transform(&before, from, area);
        self.remember(before);
    }
    pub fn picture_preference(&self) -> Option<Area> {
        self.state.picture
    }
    pub fn picture(&self) -> Area {
        self.state
            .picture
            .map(|p| fit(denormalized(p, self.frame), self.aspect))
            .unwrap_or_else(|| {
                clear_picture(self.frame, self.state.areas.iter().copied(), self.aspect)
            })
    }
    fn indices(&self) -> Vec<usize> {
        let Some(Selection::Control(i)) = self.selected else {
            return Vec::new();
        };
        if self.grouped
            && let Some(group) = self.groups.iter().find(|g| g.contains(&i))
        {
            return group.clone();
        }
        vec![i]
    }
    pub fn selection_bounds(&self) -> Option<Area> {
        if self.selected == Some(Selection::Picture) {
            return Some(self.picture());
        }
        self.indices()
            .iter()
            .map(|i| self.state.areas[*i])
            .reduce(union)
    }
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }
    pub fn can_undo(&self) -> bool {
        !self.history.is_empty()
    }
    fn remember(&mut self, before: State) {
        if before != self.state {
            if self.history.len() == 32 {
                self.history.remove(0);
            }
            self.history.push(before);
        }
    }
    pub fn down(&mut self, id: i32, x: f32, y: f32) -> bool {
        if self.drag.is_some() || !x.is_finite() || !y.is_finite() {
            return false;
        }
        let corner = if self.mode == Mode::Resize {
            self.selection_bounds().and_then(|a| {
                corners(a)
                    .iter()
                    .position(|(cx, cy)| (x - cx).abs() <= 24. && (y - cy).abs() <= 24.)
            })
        } else {
            None
        };
        if corner.is_none() {
            // Actual caps win before expanded targets, then the game field.
            self.selected = self
                .state
                .areas
                .iter()
                .rposition(|a| a.contains(x, y))
                .or_else(|| {
                    self.state.areas.iter().rposition(|a| {
                        let (cx, cy) = a.center();
                        (x - cx).abs() <= ((a.right - a.left) / 2.).max(24.)
                            && (y - cy).abs() <= ((a.bottom - a.top) / 2.).max(24.)
                    })
                })
                .map(Selection::Control)
                .or_else(|| self.picture().contains(x, y).then_some(Selection::Picture));
        }
        let Some(bounds) = self.selection_bounds() else {
            return false;
        };
        self.drag = Some(Drag {
            id,
            x,
            y,
            before: self.state.clone(),
            bounds,
            corner,
        });
        self.rejected = false;
        true
    }
    pub fn moved(&mut self, id: i32, x: f32, y: f32) {
        if !x.is_finite() || !y.is_finite() {
            return;
        }
        let Some(d) = self.drag.as_ref().filter(|d| d.id == id) else {
            return;
        };
        let (a, before, dx, dy, corner) = (d.bounds, d.before.clone(), x - d.x, y - d.y, d.corner);
        let frame = if self.selected == Some(Selection::Picture) {
            self.frame
        } else {
            Area {
                left: 0.,
                top: 0.,
                right: self.width,
                bottom: self.height,
            }
        };
        let mut next = a;
        self.guides = (None, None);
        if let Some(c) = corner {
            let points = corners(a);
            let anchor = points[(c + 2) % 4];
            let original = points[c];
            let vx = original.0 - anchor.0;
            let vy = original.1 - anchor.1;
            let scale = ((vx * (vx + dx) + vy * (vy + dy)) / (vx * vx + vy * vy)).max(0.05);
            let (ex, ey) = (anchor.0 + vx * scale, anchor.1 + vy * scale);
            next = Area {
                left: anchor.0.min(ex),
                top: anchor.1.min(ey),
                right: anchor.0.max(ex),
                bottom: anchor.1.max(ey),
            };
        } else if self.mode == Mode::Move {
            let mut dx = dx.clamp(frame.left - a.left, frame.right - a.right);
            let mut dy = dy.clamp(frame.top - a.top, frame.bottom - a.bottom);
            if self.grid {
                // Always derive from the unsnapped gesture origin so slow drags escape.
                let ids = self.indices();
                let mut xs = vec![
                    frame.left,
                    frame.left + 16.,
                    frame.center().0,
                    frame.right - 16.,
                    frame.right,
                ];
                let mut ys = vec![
                    frame.top,
                    frame.top + 16.,
                    frame.center().1,
                    frame.bottom - 16.,
                    frame.bottom,
                ];
                for (i, r) in before
                    .areas
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| !ids.contains(i))
                {
                    let _ = i;
                    xs.extend([r.left, r.center().0, r.right, self.width - r.center().0]);
                    ys.extend([r.top, r.center().1, r.bottom]);
                }
                let (sx, gx) = snap(
                    [a.left + dx, a.center().0 + dx, a.right + dx],
                    &xs,
                    frame.center().0,
                );
                let (sy, gy) = snap([a.top + dy, a.center().1 + dy, a.bottom + dy], &ys, 0.);
                dx = (dx + sx).clamp(frame.left - a.left, frame.right - a.right);
                dy = (dy + sy).clamp(frame.top - a.top, frame.bottom - a.bottom);
                self.guides = (Some(gx), Some(gy));
            }
            next = Area {
                left: a.left + dx,
                top: a.top + dy,
                right: a.right + dx,
                bottom: a.bottom + dy,
            };
        }
        self.transform(&before, a, next);
    }
    fn transform(&mut self, before: &State, from: Area, to: Area) {
        if from.right <= from.left || from.bottom <= from.top {
            return;
        }
        let mut candidate = before.clone();
        if self.selected == Some(Selection::Picture) {
            let p = fit(to, self.aspect);
            if !inside(p, self.frame, 48.) || candidate.areas.iter().any(|a| overlaps(*a, p)) {
                self.rejected = true;
                return;
            }
            candidate.picture = Some(normalized(p, self.frame));
        } else {
            let ids = self.indices();
            for i in &ids {
                let a = before.areas[*i];
                let sx = (to.right - to.left) / (from.right - from.left);
                let sy = (to.bottom - to.top) / (from.bottom - from.top);
                candidate.areas[*i] = Area {
                    left: to.left + (a.left - from.left) * sx,
                    top: to.top + (a.top - from.top) * sy,
                    right: to.left + (a.right - from.left) * sx,
                    bottom: to.top + (a.bottom - from.top) * sy,
                };
            }
            let bounds = Area {
                left: 0.,
                top: 0.,
                right: self.width,
                bottom: self.height,
            };
            let invalid = ids.iter().any(|i| {
                let a = candidate.areas[*i];
                !inside(a, bounds, self.minimum)
                    || candidate
                        .areas
                        .iter()
                        .enumerate()
                        .any(|(j, b)| j != *i && overlaps(a, *b))
                    || candidate
                        .picture
                        .is_some_and(|p| overlaps(a, fit(denormalized(p, self.frame), self.aspect)))
            });
            if invalid {
                self.rejected = true;
                return;
            }
        }
        self.state = candidate;
        self.rejected = false;
    }
    pub fn up(&mut self, id: i32) {
        if self.drag.as_ref().is_some_and(|d| d.id == id) {
            let d = self.drag.take().unwrap();
            self.remember(d.before);
            self.guides = (None, None);
        }
    }
    pub fn cancel(&mut self) {
        if let Some(d) = self.drag.take() {
            self.state = d.before;
        }
        self.guides = (None, None);
        self.rejected = false;
    }
    pub fn resize(&mut self, delta: f32) {
        self.cancel();
        let Some(a) = self.selection_bounds() else {
            return;
        };
        let scale = ((a.right - a.left + delta) / (a.right - a.left)).max(0.05);
        let (x, y) = a.center();
        let (w, h) = (
            (a.right - a.left) * scale / 2.,
            (a.bottom - a.top) * scale / 2.,
        );
        let before = self.state.clone();
        self.transform(
            &before,
            a,
            Area {
                left: x - w,
                top: y - h,
                right: x + w,
                bottom: y + h,
            },
        );
        self.remember(before);
    }
    pub fn undo(&mut self) {
        self.cancel();
        if let Some(s) = self.history.pop() {
            self.state = s;
        }
    }
    pub fn reset_selected(&mut self) {
        self.cancel();
        let before = self.state.clone();
        if self.selected == Some(Selection::Picture) {
            self.state.picture = None;
        } else {
            let ids = self.indices();
            let mut candidate = self.state.clone();
            for i in &ids {
                candidate.areas[*i] = self.defaults[*i];
            }
            if ids.iter().any(|i| {
                candidate
                    .areas
                    .iter()
                    .enumerate()
                    .any(|(j, b)| j != *i && overlaps(candidate.areas[*i], *b))
                    || candidate.picture.is_some_and(|p| {
                        overlaps(
                            candidate.areas[*i],
                            fit(denormalized(p, self.frame), self.aspect),
                        )
                    })
            }) {
                self.rejected = true;
                return;
            }
            self.state = candidate;
        }
        self.remember(before);
    }
    pub fn reset_display(&mut self) {
        self.cancel();
        let before = self.state.clone();
        self.state = State {
            areas: self.defaults.clone(),
            picture: None,
        };
        self.remember(before);
    }
    pub fn mirror(&mut self, partner: Option<usize>) {
        self.cancel();
        let Some(a) = self.selection_bounds() else {
            return;
        };
        if let Some(i) = partner {
            self.selected = Some(Selection::Control(i));
        }
        let from = self.selection_bounds().unwrap();
        let center = if self.selected == Some(Selection::Picture) {
            self.frame.center().0
        } else {
            self.width / 2.
        };
        let before = self.state.clone();
        self.transform(
            &before,
            from,
            Area {
                left: 2. * center - a.right,
                right: 2. * center - a.left,
                ..a
            },
        );
        self.remember(before);
    }
    pub fn spacing(&mut self, delta: f32) {
        self.cancel();
        let ids = self.indices();
        if ids.len() < 2 {
            return;
        }
        let a = self.selection_bounds().unwrap();
        let (x, y) = a.center();
        let scale = ((a.right - a.left + delta) / (a.right - a.left)).max(0.1);
        let before = self.state.clone();
        let mut candidate = before.clone();
        for i in &ids {
            let b = before.areas[*i];
            let (cx, cy) = b.center();
            let (dx, dy) = ((cx - x) * (scale - 1.), (cy - y) * (scale - 1.));
            candidate.areas[*i] = Area {
                left: b.left + dx,
                right: b.right + dx,
                top: b.top + dy,
                bottom: b.bottom + dy,
            };
        }
        // Use the same admission as resizing, without scaling cap sizes.
        self.state = candidate;
        let valid = ids.iter().all(|i| {
            inside(
                self.state.areas[*i],
                Area {
                    left: 0.,
                    top: 0.,
                    right: self.width,
                    bottom: self.height,
                },
                self.minimum,
            ) && self
                .state
                .areas
                .iter()
                .enumerate()
                .all(|(j, b)| j == *i || !overlaps(self.state.areas[*i], *b))
                && self.state.picture.is_none_or(|p| {
                    !overlaps(
                        self.state.areas[*i],
                        fit(denormalized(p, self.frame), self.aspect),
                    )
                })
        });
        if !valid {
            self.state = before;
            self.rejected = true;
        } else {
            self.rejected = false;
            self.remember(before);
        }
    }
}
fn inside(a: Area, b: Area, min: f32) -> bool {
    finite(a)
        && a.left >= b.left - 0.01
        && a.top >= b.top - 0.01
        && a.right <= b.right + 0.01
        && a.bottom <= b.bottom + 0.01
        && a.right - a.left + 0.01 >= min
        && a.bottom - a.top + 0.01 >= min
}
fn union(a: Area, b: Area) -> Area {
    Area {
        left: a.left.min(b.left),
        top: a.top.min(b.top),
        right: a.right.max(b.right),
        bottom: a.bottom.max(b.bottom),
    }
}
pub fn corners(a: Area) -> [(f32, f32); 4] {
    [
        (a.left, a.top),
        (a.right, a.top),
        (a.right, a.bottom),
        (a.left, a.bottom),
    ]
}

fn snap(edges: [f32; 3], guides: &[f32], origin: f32) -> (f32, f32) {
    let candidate = guides
        .iter()
        .flat_map(|g| edges.iter().map(move |e| (*g - *e, *g)))
        .filter(|(delta, _)| delta.abs() <= 6.)
        .min_by(|a, b| a.0.abs().total_cmp(&b.0.abs()));
    candidate.unwrap_or_else(|| {
        let point = origin + ((edges[1] - origin) / 8.).round() * 8.;
        (point - edges[1], point)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rect(x: f32, y: f32, w: f32, h: f32) -> Area {
        Area {
            left: x,
            top: y,
            right: x + w,
            bottom: y + h,
        }
    }
    fn draft() -> Draft {
        let a = vec![
            rect(20., 300., 48., 48.),
            rect(80., 300., 48., 48.),
            rect(700., 300., 48., 48.),
        ];
        Draft::new(840., 420., a.clone(), a, vec![vec![0, 1]], None, 48.)
    }
    #[test]
    fn grouped_gesture_is_one_undo_corner_keeps_anchor_and_cancel_restores_draft() {
        let mut d = draft();
        d.grid = false;
        let original = d.areas().to_vec();
        assert!(d.down(1, 40., 320.));
        d.moved(2, 100., 320.);
        assert_eq!(d.areas(), original);
        d.moved(1, 100., 260.);
        d.up(1);
        assert_eq!(d.areas()[0], rect(80., 240., 48., 48.));
        assert_eq!(d.areas()[1], rect(140., 240., 48., 48.));
        assert_eq!(d.areas()[2], original[2]);
        assert_eq!(d.history.len(), 1);
        d.mode = Mode::Resize;
        let a = d.selection_bounds().unwrap();
        d.down(3, a.right, a.bottom);
        d.moved(3, a.right + 60., a.bottom + 20.);
        d.up(3);
        let b = d.selection_bounds().unwrap();
        assert_eq!((a.left, a.top), (b.left, b.top));
        assert!(
            (d.areas()[0].right - d.areas()[0].left - (d.areas()[1].right - d.areas()[1].left))
                .abs()
                < 0.01
        );
        d.undo();
        assert_eq!(d.selection_bounds(), Some(a));
        d.mode = Mode::Move;
        d.down(4, 90., 250.);
        d.moved(4, 180., 210.);
        d.cancel();
        assert_eq!(d.selection_bounds(), Some(a));
        d.undo();
        assert_eq!(d.areas(), original);
    }
    #[test]
    fn game_field_has_equal_collision_priority_and_preserves_aspect_and_cancel() {
        let mut d = draft();
        d.selected = Some(Selection::Picture);
        let original = d.picture();
        d.resize(-100.);
        let p = d.picture();
        assert!(p.right - p.left < original.right - original.left);
        assert!(((p.right - p.left) / (p.bottom - p.top) - 4. / 3.).abs() < 0.0001);
        let before = d.state.clone();
        d.mode = Mode::Resize;
        assert!(d.down(1, p.left, p.top));
        d.moved(1, p.left + 30., p.top + 30.);
        d.up(1);
        assert_eq!((d.picture().right, d.picture().bottom), (p.right, p.bottom));
        d.undo();
        assert!(d.state == before);
        d.selected = Some(Selection::Control(2));
        d.grouped = false;
        let old = d.areas()[2];
        d.set_selected_area(p);
        assert!(d.rejected);
        assert_eq!(d.areas()[2], old);
        d.selected = Some(Selection::Picture);
        let old = d.picture();
        d.set_selected_area(rect(0., 280., 300., 225.));
        assert!(d.rejected);
        assert_eq!(d.picture(), old);
        d.reset_selected();
        assert!(d.picture_preference().is_none());
        d.undo();
        assert_eq!(d.picture(), old);
    }
    #[test]
    fn snapping_escapes_slowly_group_spacing_and_mirror_do_not_push_other_controls() {
        let mut d = draft();
        let original = d.areas().to_vec();
        d.down(1, 44., 324.);
        d.moved(1, 45., 324.);
        let a = d.areas()[0];
        for x in 46..65 {
            d.moved(1, x as f32, 324.);
        }
        assert!(d.areas()[0].left > a.left + 8.);
        d.up(1);
        assert_eq!(d.history.len(), 1);
        d.undo();
        assert_eq!(d.areas(), original);
        d.selected = Some(Selection::Control(0));
        d.spacing(16.);
        assert_eq!(d.areas()[0].right - d.areas()[0].left, 48.);
        assert!(d.areas()[1].left - d.areas()[0].left > 60.);
        d.undo();
        d.grouped = false;
        d.mirror(Some(2));
        assert!(!d.rejected);
        assert_eq!(d.areas()[2], rect(772., 300., 48., 48.));
        assert_eq!(&d.areas()[..2], &original[..2]);
        d.undo();
        assert_eq!(d.areas(), original);
    }
}
