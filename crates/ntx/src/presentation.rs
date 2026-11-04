// SPDX-License-Identifier: MIT
//! Shared ntx presentation values; no native handles or emulator state.
pub const BACKGROUND: u32 = 0xff111418;
pub const SURFACE: u32 = 0xff22282c;
pub const TEXT: u32 = 0xffeef1eb;
pub const MUTED: u32 = 0xffa9b2b2;
pub const ACCENT: u32 = 0xff85ca72;
pub const INK: u32 = 0xff173322;
pub const DANGER: u32 = 0xffdca7a0;

/// Keep a following stick's visible well inside its generous touch target.
/// Input keeps its continuous following origin; drawing cannot drift over D-pad,
/// Start or another independently captured control.
pub fn stick_visual(area: crate::touch::Area, origin: Option<(f32, f32)>) -> (f32, f32, f32) {
    let radius = (area.right - area.left).min(area.bottom - area.top) * 0.4;
    let (x, y) = origin
        .filter(|(x, y)| x.is_finite() && y.is_finite())
        .unwrap_or(area.center());
    (
        x.clamp(area.left + radius, area.right - radius),
        y.clamp(area.top + radius, area.bottom - radius),
        radius,
    )
}

/// Keep decisions outside scrollable content. An error may replace Resume with
/// Restart; a stored-place confirmation must keep cancellation beside Close.
pub fn menu_footer_action(
    editing: bool,
    stop_available: bool,
    confirming_place: bool,
    action: i32,
) -> bool {
    if editing {
        matches!(action, 10 | 11)
    } else if confirming_place {
        matches!(action, 18 | 41)
    } else {
        stop_available && matches!(action, 2..=4)
    }
}

/// File-provider names remain intact in storage. Present only their title;
/// region/revision/file-container labels are not a game's public identity.
pub fn game_title(name: &str) -> String {
    let mut title = name.trim().to_owned();
    for suffix in [".rvz", ".iso", ".gcm"] {
        if title.to_ascii_lowercase().ends_with(suffix) {
            title.truncate(title.len() - suffix.len());
            break;
        }
    }
    while let Some(start) = title.rfind(" (") {
        if !title.ends_with(')') {
            break;
        }
        let tag = &title[start + 2..title.len() - 1];
        if [
            "USA",
            "USA, Canada",
            "Europe",
            "Japan",
            "Australia",
            "Korea",
            "World",
        ]
        .contains(&tag)
            || tag.starts_with("Rev ")
        {
            title.truncate(start);
        } else {
            break;
        }
    }
    if let Some((name, rest)) = title.split_once(", The - ") {
        format!("The {name} — {rest}")
    } else {
        title
    }
}
/// Adjust decorative alpha only; never filters, buffers or delays logical input.
pub fn opacity(abgr: u32, percent: u8) -> u32 {
    let alpha = (abgr >> 24) * u32::from(percent.clamp(30, 100)) / 100;
    (abgr & 0x00ff_ffff) | (alpha << 24)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn following_stick_graphics_stay_inside_the_captured_target_even_after_long_drags() {
        use crate::{
            Control,
            touch::{Layout, Target},
        };
        for (w, h) in [(840., 420.), (800., 700.), (390., 844.)] {
            let layout = Layout::new(w, h);
            for z in layout
                .zones
                .iter()
                .filter(|z| matches!(z.target, Target::Stick(..)))
            {
                for origin in [
                    None,
                    Some((-10000., 10000.)),
                    Some((10000., -10000.)),
                    Some((f32::NAN, 0.)),
                ] {
                    let (x, y, r) = stick_visual(z.area, origin);
                    assert!(x - r >= z.area.left - 0.01 && x + r <= z.area.right + 0.01);
                    assert!(y - r >= z.area.top - 0.01 && y + r <= z.area.bottom + 0.01);
                    assert!(r > 0.);
                }
            }
            assert!(
                layout
                    .zones
                    .iter()
                    .any(|z| z.target == Target::Stick(Control::CX, Control::CY))
            );
        }
    }
    #[test]
    fn pause_recovery_and_library_keep_stop_beside_the_primary_action() {
        for actions in [
            vec![3, 4, 7, 6, 8],
            vec![2, 4, 7, 6, 8],
            vec![8, 9, 3, 4, 1],
        ] {
            let fixed: Vec<_> = actions
                .iter()
                .copied()
                .filter(|a| menu_footer_action(false, true, false, *a))
                .collect();
            assert!(fixed.contains(&4));
            assert_eq!(fixed.len(), 2);
            assert!(fixed.contains(&3) || fixed.contains(&2));
        }
        // An unloaded game's Play/Restart and the editor retain their own flow.
        assert!(!menu_footer_action(false, false, false, 2));
        assert!(!menu_footer_action(false, false, false, 5));
        let editor: Vec<_> = [20, 21, 23, 24, 25, 22, 10, 11]
            .into_iter()
            .filter(|a| menu_footer_action(true, false, false, *a))
            .collect();
        assert_eq!(editor, [10, 11]);
        for stop_available in [false, true] {
            let confirmation: Vec<_> = [2, 3, 4, 6, 8, 18, 41, 100]
                .into_iter()
                .filter(|a| menu_footer_action(false, stop_available, true, *a))
                .collect();
            assert_eq!(confirmation, [18, 41]);
        }
        // The same action opens storage elsewhere; it is not globally pinned.
        assert!(!menu_footer_action(false, false, false, 18));
    }
    #[test]
    fn library_names_keep_disc_and_subtitle_but_drop_known_file_metadata() {
        assert_eq!(
            game_title("Legend of Zelda, The - The Wind Waker (USA, Canada).rvz"),
            "The Legend of Zelda — The Wind Waker"
        );
        assert_eq!(
            game_title("Example (Disc 2) (USA) (Rev 1).ISO"),
            "Example (Disc 2)"
        );
        assert_eq!(
            game_title("My game (Director's Cut).gcm"),
            "My game (Director's Cut)"
        );
    }
    #[test]
    fn opacity_preserves_gamecube_color_and_existing_translucency() {
        assert_eq!(opacity(0xff68b65a, 50), 0x7f68b65a);
        assert_eq!(opacity(0x703c3b3a, 50), 0x383c3b3a);
    }
}
