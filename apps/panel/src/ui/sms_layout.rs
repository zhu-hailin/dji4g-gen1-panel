//! Pure geometry for the SMS workspace: no device access, no screen-global coordinates, no
//! commands.  Every value is an egui logical point; the previous implementation estimated the
//! list height from `Context::screen_rect()` and then subtracted margins again per nesting level,
//! which capped the message list at 340 points no matter how tall the window was.  The layout is
//! now computed once from the height the parent container actually left over.

/// Below this inner width the list and the reader share one column and the reader replaces the
/// list.  The value is measured on the workspace inside the page frame, not on the window.
pub(crate) const WIDE_MIN_WIDTH: f32 = 720.0;

/// Gap between the list column, the divider and the reader column.  Charged exactly once.
pub(crate) const COLUMN_GAP: f32 = 26.0;

/// Narrowest and widest the list column may become in the wide layout.
const LIST_MIN_WIDTH: f32 = 270.0;
const LIST_MAX_WIDTH: f32 = 380.0;
const LIST_WIDTH_RATIO: f32 = 0.38;

/// Height reserved for the workspace footer note (`发送接受 ≠ 收到` line).  This is the footer's
/// own height, not a screen-coordinate guess: it is subtracted once, from the frame's real
/// remaining height.
pub(crate) const FOOTER_RESERVE: f32 = 44.0;

/// Below this much free height the bounded workspace cannot show anything useful, so the page
/// falls back to the shared whole-page scroll area.  A normal window at the 800x600 logical
/// minimum is far above it, so the list never competes with the page for the wheel.
pub(crate) const MIN_BOUNDED_HEIGHT: f32 = 360.0;

fn finite_non_negative(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SmsWorkspaceLayout {
    /// Wide means side-by-side list and reader; narrow means the reader replaces the list.
    pub wide: bool,
    /// Height handed to the message list / reader.  No upper bound, no forced minimum: a short
    /// window yields a short workspace and the caller decides whether it still fits.
    pub body_height: f32,
    pub list_width: f32,
    pub detail_width: f32,
}

/// Split `width` x `remaining_height` into the workspace regions.
///
/// Non-finite or negative inputs degrade to zero instead of propagating NaN into egui's
/// `max_rect`.  In the wide layout both columns receive the same height and the gap (divider plus
/// spacing) is charged once, so `list_width + gap + detail_width <= width` always holds.
pub(crate) fn workspace_layout(width: f32, remaining_height: f32, gap: f32) -> SmsWorkspaceLayout {
    let width = finite_non_negative(width);
    let body_height = finite_non_negative(remaining_height);
    let gap = finite_non_negative(gap);
    if width < WIDE_MIN_WIDTH {
        return SmsWorkspaceLayout {
            wide: false,
            body_height,
            list_width: width,
            detail_width: width,
        };
    }
    let list_width = (width * LIST_WIDTH_RATIO)
        .clamp(LIST_MIN_WIDTH, LIST_MAX_WIDTH)
        .min(width);
    let detail_width = finite_non_negative(width - list_width - gap);
    SmsWorkspaceLayout {
        wide: true,
        body_height,
        list_width,
        detail_width,
    }
}

/// The list column inside the workspace: the search field sits on top and the message list gets
/// whatever height is left over.  Returns the list viewport height, never negative.
pub(crate) fn list_viewport_height(column_height: f32, search_height: f32) -> f32 {
    finite_non_negative(column_height - finite_non_negative(search_height))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A returned region must never push the frame wider than the space it was given. Compares
    /// with a small tolerance so layout rounding does not turn into a failure.
    fn within_parent(child: f32, parent: f32) -> bool {
        child <= parent + 0.5
    }

    #[test]
    fn wide_layout_starts_at_720_inner_width() {
        let narrow = workspace_layout(719.0, 400.0, COLUMN_GAP);
        assert!(!narrow.wide, "719 is still the single-column layout");
        assert_eq!(narrow.list_width, 719.0);
        assert_eq!(narrow.detail_width, 719.0);

        let wide = workspace_layout(720.0, 400.0, COLUMN_GAP);
        assert!(wide.wide, "720 is the first wide width");
    }

    #[test]
    fn wide_columns_never_exceed_the_parent_width() {
        for width in [720.0_f32, 800.0, 1000.0, 1100.0, 1440.0, 4000.0] {
            let layout = workspace_layout(width, 600.0, COLUMN_GAP);
            assert!(layout.wide);
            assert!(within_parent(layout.list_width, width));
            assert!(within_parent(
                layout.list_width + COLUMN_GAP + layout.detail_width,
                width
            ));
            assert!(layout.list_width >= LIST_MIN_WIDTH - 0.5);
            assert!(layout.list_width <= LIST_MAX_WIDTH + 0.5);
        }
    }

    #[test]
    fn body_height_follows_the_remaining_height_without_a_cap() {
        for height in [0.0_f32, 120.0, 300.0, 650.0, 2000.0] {
            let layout = workspace_layout(1100.0, height, COLUMN_GAP);
            assert!(
                (layout.body_height - height).abs() < 0.001,
                "height {height}"
            );
        }
        // The regression this replaces: the old cap froze the workspace at 340.
        let tall = workspace_layout(1100.0, 650.0, COLUMN_GAP);
        assert!(tall.body_height > 340.0);
    }

    #[test]
    fn growing_the_window_grows_the_workspace() {
        let short = workspace_layout(1100.0, 300.0, COLUMN_GAP);
        let tall = workspace_layout(1100.0, 650.0, COLUMN_GAP);
        assert!(tall.body_height > short.body_height);
        assert!(tall.body_height - short.body_height >= 349.0);
    }

    #[test]
    fn non_finite_and_negative_inputs_normalize_to_zero() {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -50.0] {
            let layout = workspace_layout(bad, bad, bad);
            assert_eq!(layout.body_height, 0.0);
            assert!(layout.list_width.is_finite());
            assert!(layout.detail_width.is_finite());
            assert!(layout.list_width >= 0.0);
            assert!(layout.detail_width >= 0.0);
        }
        let height = workspace_layout(1100.0, f32::NAN, COLUMN_GAP);
        assert_eq!(height.body_height, 0.0);
        let positive = workspace_layout(1100.0, -10.0, COLUMN_GAP);
        assert_eq!(positive.body_height, 0.0);
    }

    #[test]
    fn the_bounded_workspace_has_room_at_the_minimum_window() {
        // 600 logical points of window height, minus the top bar, footer bar and page margins.
        let frame_free = 600.0 - 40.0 - 28.0 - 48.0 - 90.0;
        assert!(frame_free >= MIN_BOUNDED_HEIGHT);
    }

    #[test]
    fn list_viewport_never_goes_negative() {
        assert_eq!(list_viewport_height(200.0, 60.0), 140.0);
        assert_eq!(list_viewport_height(40.0, 60.0), 0.0);
        assert_eq!(list_viewport_height(f32::NAN, 60.0), 0.0);
        // A non-finite consumption normalizes to zero rather than poisoning the subtraction, so
        // the column keeps its full height.
        assert_eq!(list_viewport_height(200.0, f32::INFINITY), 200.0);
        assert_eq!(list_viewport_height(200.0, f32::NAN), 200.0);
    }
}
