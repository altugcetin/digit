//! Shared spacing and corner values, so sibling panels line up. Radii here are at the default
//! Roundness; multiply by `app.round()` (or `app.notch_round()` in the notch) when drawing.

/// Corner radius of an edge-docked panel (Control Center, Workspaces, calendar).
pub const PANEL_RADIUS: f64 = 28.0;
/// The concave fillet where a docked panel meets the screen edge.
/// How far a card under the island drops in as it opens.
pub const CARD_DROP: f64 = 16.0;
pub const FILLET: f64 = 14.0;
/// Space between a panel's edge and its content.
pub const PANEL_PAD: f64 = 18.0;
/// Corner radius of a card inside a panel.
pub const CARD_RADIUS: f64 = 20.0;
/// Space between neighbouring controls: buttons in a row, tiles in a grid.
pub const GAP: f64 = 10.0;
/// Space between larger cards in a grid (workspace previews, layout tiles).
pub const GRID_GAP: f64 = 12.0;
/// How far a highlight or nested shape sits inside its container.
pub const INSET: f64 = 8.0;

/// A corner for something nested `inset` inside a shape with radius `outer`, so both curves
/// stay concentric.
pub fn nested(outer: f64, inset: f64) -> f64 {
    (outer - inset).max(0.0)
}

/// A radius at the default Roundness scaled by `round`, never more than a pill of height `h`.
pub fn scaled(radius: f64, round: f64, h: f64) -> f64 {
    (radius * round).min(h / 2.0).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_corners_stay_concentric_and_never_negative() {
        assert_eq!(nested(PANEL_RADIUS, INSET), 20.0);
        assert_eq!(nested(6.0, INSET), 0.0);
    }

    #[test]
    fn scaled_radius_follows_roundness_but_stops_at_a_pill() {
        assert_eq!(scaled(CARD_RADIUS, 1.0, 120.0), 20.0);
        assert_eq!(scaled(CARD_RADIUS, 0.55, 120.0), 11.0);
        assert_eq!(scaled(CARD_RADIUS, 1.6, 30.0), 15.0);
    }
}
