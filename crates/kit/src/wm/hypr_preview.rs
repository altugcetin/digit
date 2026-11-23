use super::{Zone, hypr_float_rect, hypr_json, hypr_monitor, hypr_work_area};
use serde_json::Value;

type Rect = [f64; 4];
type Point = (f64, f64);
type Gaps = [f64; 4]; // top, right, bottom, left

pub struct HyprPreview {
    work: Rect,
    tiled: Rect,
    gaps: Gaps,
    layout: Layout,
}

enum Layout {
    Full,
    /// A tile that doesn't depend on where the app is dropped.
    Fixed(Rect),
    Dwindle {
        windows: Vec<(Rect, bool)>,
        active_split: bool,
        hover_focus: bool,
        smart: bool,
        force: i64,
        multiplier: f64,
        ratio: f64,
        bias: bool,
    },
    Scrolling {
        anchor: Option<Rect>,
        axis: usize,
        reversed: bool,
        width: f64,
        single: bool,
        center: bool,
    },
    Unavailable,
}

impl HyprPreview {
    pub fn read(output: &str) -> Option<Self> {
        let monitor = hypr_monitor(output)?;
        let work = hypr_work_area(&monitor)?;
        let ox = monitor["x"].as_f64()?;
        let oy = monitor["y"].as_f64()?;
        let workspace = monitor["specialWorkspace"]["address"].as_str().filter(|name| !name.is_empty())
            .or_else(|| monitor["activeWorkspace"]["address"].as_str())
            .or_else(|| monitor["activeWorkspace"]["name"].as_str())?;
        let workspaces = hypr_json("workspaces")?;
        let info = workspaces.as_array()?.iter().find(|w| {
            w["monitor"].as_str() == Some(output)
                && w["address"].as_str().or_else(|| w["name"].as_str()) == Some(workspace)
        })?;
        let rules = hypr_json("workspacerules");
        let rule = rules.as_ref().and_then(Value::as_array).and_then(|rules| rules.iter().find(|rule| {
            let selector = rule["workspaceString"].as_str();
            rule["enabled"].as_bool() != Some(false) && (selector == Some(workspace) || selector == info["name"].as_str())
        }));
        let gaps = |rule_key: &str, name: &str| {
            rule.and_then(|r| four(r[rule_key].as_array()?.iter().filter_map(Value::as_f64)))
                .or_else(|| four(option(name)?["css"].as_str()?.split_whitespace().filter_map(|v| v.parse().ok())))
        };
        let tiled = inset(work, gaps("gapsOut", "general:gaps_out")?);
        let inner = gaps("gapsIn", "general:gaps_in")?;
        let border = option("general:border_size").and_then(|v| v["int"].as_f64()).unwrap_or(0.0);
        let clients = hypr_json("clients")?;
        let mut windows: Vec<_> = clients.as_array()?.iter().filter(|w| {
            w["monitor"].as_i64() == monitor["id"].as_i64()
                && w["workspace"]["address"].as_str().or_else(|| w["workspace"]["name"].as_str()) == Some(workspace)
                && w["mapped"].as_bool() != Some(false) && w["hidden"].as_bool() != Some(true)
                && w["floating"].as_bool() != Some(true)
        }).filter_map(|w| {
            let at = w["at"].as_array()?;
            let size = w["size"].as_array()?;
            let rect = [at.first()?.as_f64()? - ox, at.get(1)?.as_f64()? - oy, size.first()?.as_f64()?, size.get(1)?.as_f64()?];
            Some((rect, w["focusHistoryID"].as_i64() == Some(0), w["address"].as_str() == info["lastwindow"].as_str(), w["fullscreen"].as_i64().unwrap_or(0)))
        }).collect();
        let layout_name = info["tiledLayout"].as_str().map(str::to_owned)
            .or_else(|| Some(option("general:layout")?["str"].as_str()?.to_owned()))?;
        let fullscreen = info["hasfullscreen"].as_bool() == Some(true);
        let layout = if tiled[2] <= 0.0 || tiled[3] <= 0.0 || (fullscreen && layout_name != "dwindle") {
            Layout::Unavailable
        } else if layout_name == "monocle" || (!fullscreen && windows.is_empty() && layout_name == "dwindle") {
            Layout::Full
        } else if layout_name == "dwindle" {
            for (rect, _, _, mode) in &mut windows {
                if *mode == 0 {
                    *rect = tile_box(*rect, tiled, inner, border);
                }
            }
            let recovered = if fullscreen {
                match windows.as_mut_slice() {
                    [(rect, _, _, 1)] => { *rect = tiled; true }
                    [(rect, _, _, 1), (other, _, _, 0)] | [(other, _, _, 0), (rect, _, _, 1)] => {
                        if let Some(hidden) = tile_complement(tiled, *other) {
                            *rect = hidden;
                            true
                        } else {
                            false
                        }
                    }
                    _ => false,
                }
            } else {
                true
            };
            if recovered {
                Layout::Dwindle {
                    windows: windows.into_iter().map(|(rect, active, _, _)| (rect, active)).collect(),
                    active_split: flag("dwindle:use_active_for_splits", true),
                    hover_focus: integer("input:follow_mouse", 1) == 1,
                    smart: flag("dwindle:smart_split", false),
                    force: integer("dwindle:force_split", 0),
                    multiplier: number("dwindle:split_width_multiplier", 1.0).max(0.01),
                    ratio: number("dwindle:default_split_ratio", 1.0).clamp(0.1, 1.9),
                    bias: integer("dwindle:split_bias", 0) != 0,
                }
            } else {
                Layout::Unavailable
            }
        } else if layout_name == "scrolling" {
            let direction = option("scrolling:direction").and_then(|v| v["str"].as_str().map(str::to_owned));
            let (axis, reversed) = match direction.as_deref() {
                Some("up") => (1, true),
                Some("down") => (1, false),
                Some("left") => (0, true),
                _ => (0, false),
            };
            let anchor = windows.iter().find(|(_, active, _, _)| *active)
                .or_else(|| windows.iter().find(|(_, _, last, _)| *last))
                .or_else(|| windows.last()).map(|(rect, _, _, _)| {
                    let [top, right, bottom, left] = inner.map(|gap| -gap - border);
                    inset(*rect, if axis == 0 { [0.0, right, 0.0, left] } else { [top, 0.0, bottom, 0.0] })
                });
            Layout::Scrolling {
                anchor, axis, reversed,
                width: number("scrolling:column_width", 0.5).clamp(0.1, 1.0),
                single: flag("scrolling:fullscreen_on_one_column", true),
                center: integer("scrolling:focus_fit_method", 1) != 1,
            }
        } else if layout_name == "master" {
            let tiles: Vec<_> = windows.iter().map(|(rect, active, _, _)| (tile_box(*rect, tiled, inner, border), *active)).collect();
            let master = Master {
                orientation: text("master:orientation", "left"),
                status: text("master:new_status", "slave"),
                on_active: text("master:new_on_active", "none") != "none",
                on_top: flag("master:new_on_top", false),
                mfact: number("master:mfact", 0.55),
                smart: flag("master:smart_resizing", true),
                keep: flag("master:always_keep_position", false),
            };
            master.rect(&tiles, tiled).map_or(Layout::Unavailable, Layout::Fixed)
        } else {
            // A custom layout can choose a tile we cannot infer from its clients.
            Layout::Unavailable
        };
        Some(Self { work, tiled, gaps: inner, layout })
    }

    pub fn rect(&self, zone: Zone, pointer: Point) -> Option<Rect> {
        if zone == Zone::Float {
            return Some(hypr_float_rect(self.work));
        }
        let area = self.tiled;
        let rect = match &self.layout {
            Layout::Full => area,
            Layout::Fixed(rect) => inset(*rect, inner_gaps(*rect, area, self.gaps)),
            Layout::Dwindle { windows, active_split, hover_focus, smart, force, multiplier, ratio, bias } => {
                let opening = dwindle_target(windows, pointer, *active_split, *hover_focus)?;
                let [x, y, w, h] = opening.0;
                let (dx, dy) = (pointer.0 - x - w / 2.0, pointer.1 - y - h / 2.0);
                let axis = if *smart { usize::from(dy.abs() * w >= dx.abs() * h) } else { usize::from(w < h * multiplier) };
                let first = if !*smart && *force != 0 { *force == 1 } else { [dx, dy][axis] < 0.0 };
                let ratio = if *bias && first { 2.0 - ratio } else { *ratio };
                let mut rect = opening.0;
                let extent = rect[axis + 2] * ratio / 2.0;
                if first {
                    rect[axis + 2] = extent;
                } else {
                    rect[axis] += extent;
                    rect[axis + 2] -= extent;
                }
                inset(rect, inner_gaps(rect, area, self.gaps))
            }
            Layout::Scrolling { anchor, axis, reversed, width, single, center } => {
                let axis = *axis;
                let size = axis + 2;
                if anchor.is_none() && *single {
                    return Some(area);
                }
                let mut rect = area;
                let full = area[size];
                rect[size] = full * width;
                let (leading, trailing) = if axis == 0 { (self.gaps[3], self.gaps[1]) } else { (self.gaps[0], self.gaps[2]) };
                let start = if let Some(anchor) = anchor {
                    if *center {
                        area[axis] + (full - rect[size]) / 2.0
                    } else if *reversed {
                        if anchor[size] >= full - 1.0 {
                            area[axis] + full - 2.0 * rect[size]
                        } else {
                            anchor[axis] - rect[size]
                        }
                    } else if anchor[size] >= full - 1.0 {
                        area[axis] + rect[size]
                    } else {
                        anchor[axis] + anchor[size]
                    }
                } else if *reversed {
                    area[axis] + full - rect[size]
                } else {
                    area[axis]
                };
                rect[axis] = start.clamp(area[axis], area[axis] + full - rect[size]);
                rect[axis] += leading;
                rect[size] -= leading + trailing;
                rect
            }
            Layout::Unavailable => return None,
        };
        (rect[2] > 0.0 && rect[3] > 0.0).then_some(rect)
    }
}

fn dwindle_target(windows: &[(Rect, bool)], pointer: Point, active_split: bool, hover_focus: bool) -> Option<&(Rect, bool)> {
    // Hyprland refocuses the window under the pointer after the Wayland drop ends.
    if (!active_split || hover_focus)
        && let Some(hovered) = windows.iter().find(|(rect, _)| contains(*rect, pointer))
    {
        return Some(hovered);
    }
    if active_split {
        windows.iter().find(|(_, active)| *active).or_else(|| windows.iter().min_by(|(a, _), (b, _)| {
            distance(*a, pointer).total_cmp(&distance(*b, pointer))
        }))
    } else {
        windows.first()
    }
}

/// Hyprland's master layout options that decide where a new window's tile goes.
struct Master {
    orientation: String,
    status: String,
    on_active: bool,
    on_top: bool,
    mfact: f64,
    smart: bool,
    keep: bool,
}

impl Master {
    /// The tile `area` gives a new window, as master.cpp's addTarget and calculateWorkspace
    /// place it. `tiles` are the workspace's tiles and whether each has focus.
    fn rect(&self, tiles: &[(Rect, bool)], area: Rect) -> Option<Rect> {
        let (axis, far) = match self.orientation.as_str() {
            "left" => (0, false),
            "right" => (0, true),
            "top" => (1, false),
            "bottom" => (1, true),
            // ponytail: center splits the stack into two columns; add it if someone uses it.
            _ => return None,
        };
        let (size, cross) = (axis + 2, 1 - axis);
        let near = |a: f64, b: f64| (a - b).abs() <= 2.0;
        // Where along the axis a master part `len` long starts.
        let master_at = |len: f64| if far { area[axis] + area[size] - len } else { area[axis] };
        let mut rect = area;
        if tiles.is_empty() {
            if self.keep {
                rect[size] = area[size] * self.mfact;
                rect[axis] = master_at(rect[size]);
            }
            return Some(rect);
        }
        // One master, at the master side and across the whole area; more masters aren't predicted.
        let side = if far { area[axis] + area[size] } else { area[axis] };
        let edge = |r: &Rect| if far { r[axis] + r[size] } else { r[axis] };
        let masters: Vec<_> = tiles.iter().filter(|(r, _)| near(edge(r), side)).collect();
        let &[&(master, master_focused)] = masters.as_slice() else { return None };
        if !near(master[cross], area[cross]) || !near(master[cross + 2], area[cross + 2]) {
            return None;
        }
        let new_master = match self.status.as_str() {
            "master" => true,
            "slave" => false,
            // Hyprland opens on the focused tile, or on the master when no tile here has focus.
            "inherit" => master_focused || !tiles.iter().any(|(_, f)| *f),
            _ => return None,
        };
        if tiles.len() == 1 {
            // ponytail: the master's share is mfact; a share resized by hand isn't visible while it's alone.
            let len = area[size] * self.mfact;
            if new_master {
                rect[size] = len;
                rect[axis] = master_at(len);
            } else {
                rect[size] -= len;
                if !far {
                    rect[axis] += len;
                }
            }
            return Some(rect);
        }
        if new_master {
            // The new window takes the master's tile; the old master joins the stack.
            return Some(master);
        }
        if self.on_active {
            return None;
        }
        let slaves: Vec<_> = tiles.iter().map(|(r, _)| r).filter(|r| **r != master).collect();
        // Without smart resizing, the last tile takes what's left, so only an even stack is known.
        if !self.smart && slaves.iter().any(|r| !near(r[cross + 2], slaves[0][cross + 2])) {
            return None;
        }
        rect[size] -= master[size];
        if !far {
            rect[axis] += master[size];
        }
        let slot = area[cross + 2] / (slaves.len() + 1) as f64;
        rect[cross + 2] = slot;
        if !self.on_top {
            rect[cross] += area[cross + 2] - slot;
        }
        Some(rect)
    }
}

fn option(name: &str) -> Option<Value> {
    hypr_json(&format!("getoption {name}"))
}

fn text(name: &str, default: &str) -> String {
    option(name).and_then(|v| Some(v["str"].as_str()?.to_owned())).unwrap_or_else(|| default.to_owned())
}

fn flag(name: &str, default: bool) -> bool {
    option(name).and_then(|v| v["bool"].as_bool().or_else(|| Some(v["int"].as_i64()? != 0))).unwrap_or(default)
}

fn integer(name: &str, default: i64) -> i64 {
    option(name).and_then(|v| v["int"].as_i64()).unwrap_or(default)
}

fn number(name: &str, default: f64) -> f64 {
    option(name).and_then(|v| v["float"].as_f64()).unwrap_or(default)
}

fn four(values: impl Iterator<Item = f64>) -> Option<Gaps> {
    let values: Vec<_> = values.collect();
    values.try_into().ok()
}

fn inset([x, y, w, h]: Rect, [top, right, bottom, left]: Gaps) -> Rect {
    [x + left, y + top, w - left - right, h - top - bottom]
}

fn distance([x, y, w, h]: Rect, (px, py): Point) -> f64 {
    (x + w / 2.0 - px).hypot(y + h / 2.0 - py)
}

fn contains([x, y, w, h]: Rect, (px, py): Point) -> bool {
    (x..x+w).contains(&px) && (y..y+h).contains(&py)
}

fn inner_gaps([x, y, w, h]: Rect, [ax, ay, aw, ah]: Rect, [top, right, bottom, left]: Gaps) -> Gaps {
    [if y > ay + 1.0 { top } else { 0.0 },
     if x+w < ax+aw - 1.0 { right } else { 0.0 },
     if y+h < ay+ah - 1.0 { bottom } else { 0.0 },
     if x > ax + 1.0 { left } else { 0.0 }]
}

fn tile_box(rect: Rect, area: Rect, gaps: Gaps, border: f64) -> Rect {
    let border_box = inset(rect, [-border; 4]);
    inset(border_box, inner_gaps(border_box, area, gaps).map(|g| -g))
}

// In a two-tile workspace, the other tile reveals the maximized tile's normal geometry.
fn tile_complement(area: Rect, other: Rect) -> Option<Rect> {
    let near = |a: f64, b: f64| (a - b).abs() <= 1.0;
    for axis in 0..2 {
        let cross = 1 - axis;
        let size = axis + 2;
        if !near(area[cross], other[cross]) || !near(area[cross+2], other[cross+2])
            || !(0.0 < other[size] && other[size] < area[size])
        {
            continue;
        }
        let mut missing = area;
        missing[size] -= other[size];
        if near(area[axis], other[axis]) {
            missing[axis] += other[size];
            return Some(missing);
        }
        if near(area[axis] + area[size], other[axis] + other[size]) {
            return Some(missing);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dwindle_uses_the_tile_hyprland_focuses_after_drop() {
        let area = [20.0, 60.0, 1240.0, 564.0];
        let preview = HyprPreview { work: area, tiled: area, gaps: [5.0; 4], layout: Layout::Dwindle {
            windows: vec![([20.0, 342.0, 1240.0, 282.0], true), ([20.0, 60.0, 1240.0, 282.0], false)],
            active_split: true, hover_focus: true, smart: false, force: 0, multiplier: 1.0, ratio: 1.0, bias: false,
        }};
        assert_eq!(preview.rect(Zone::Tiled, (1240.0, 85.0)), Some([645.0, 60.0, 615.0, 277.0]));
        let mut preview = preview;
        if let Layout::Dwindle { hover_focus, .. } = &mut preview.layout {
            *hover_focus = false;
        }
        assert_eq!(preview.rect(Zone::Tiled, (1240.0, 85.0)), Some([645.0, 347.0, 615.0, 277.0]));
    }

    #[test]
    fn a_maximized_tile_uses_the_space_not_occupied_by_its_peer() {
        let area = [20.0, 60.0, 1240.0, 564.0];
        let peer = tile_box([645.0, 60.0, 615.0, 564.0], area, [5.0; 4], 0.0);
        assert_eq!(tile_complement(area, peer), Some([20.0, 60.0, 620.0, 564.0]));
        assert_eq!(tile_complement(area, [20.0, 60.0, 1240.0, 282.0]), Some([20.0, 342.0, 1240.0, 282.0]));
        assert_eq!(tile_complement(area, [20.0, 60.0, 620.0, 282.0]), None);
    }

    #[test]
    fn dwindle_splits_the_active_tile_instead_of_the_screen() {
        let area = [20.0, 60.0, 1240.0, 564.0];
        let old = tile_box([25.0, 60.0, 610.0, 564.0], area, [5.0; 4], 0.0);
        let preview = HyprPreview { work: [0.0, 40.0, 1280.0, 604.0], tiled: area, gaps: [5.0; 4], layout: Layout::Dwindle {
            windows: vec![(old, true)], active_split: true, hover_focus: true, smart: false, force: 0, multiplier: 1.0, ratio: 1.0, bias: false,
        }};
        assert_eq!(preview.rect(Zone::Tiled, (1240.0, 375.0)), Some([335.0, 60.0, 300.0, 564.0]));
        assert_eq!(preview.rect(Zone::Float, (640.0, 320.0)), Some([294.0, 155.0, 691.0, 374.0]));
    }

    #[test]
    fn master_places_a_new_tile_like_hyprland() {
        let area = [0.0, 0.0, 1000.0, 600.0];
        let mut m = Master { orientation: "left".into(), status: "slave".into(), on_active: false, on_top: false, mfact: 0.55, smart: true, keep: false };
        assert_eq!(m.rect(&[], area), Some(area));
        let one = [(area, true)];
        assert_eq!(m.rect(&one, area), Some([550.0, 0.0, 450.0, 600.0]));
        let three = [([0.0, 0.0, 550.0, 600.0], false), ([550.0, 0.0, 450.0, 400.0], true), ([550.0, 400.0, 450.0, 200.0], false)];
        assert_eq!(m.rect(&three, area), Some([550.0, 400.0, 450.0, 200.0]));
        m.on_top = true;
        assert_eq!(m.rect(&three, area), Some([550.0, 0.0, 450.0, 200.0]));
        // An uneven stack without smart resizing, and new_on_active, aren't predicted.
        m.smart = false;
        assert_eq!(m.rect(&three, area), None);
        m.smart = true;
        m.on_active = true;
        assert_eq!(m.rect(&three, area), None);
        m.status = "master".into();
        assert_eq!(m.rect(&one, area), Some([0.0, 0.0, 550.0, 600.0]));
        assert_eq!(m.rect(&three, area), Some([0.0, 0.0, 550.0, 600.0]));
        // inherit follows the focused tile: a slave here, the master with nothing focused.
        m.status = "inherit".into();
        m.on_active = false;
        m.on_top = false;
        assert_eq!(m.rect(&three, area), Some([550.0, 400.0, 450.0, 200.0]));
        let unfocused = three.map(|(r, _)| (r, false));
        assert_eq!(m.rect(&unfocused, area), Some([0.0, 0.0, 550.0, 600.0]));
        m.status = "slave".into();
        m.orientation = "bottom".into();
        assert_eq!(m.rect(&one, area), Some([0.0, 0.0, 1000.0, 270.0]));
        m.orientation = "center".into();
        assert_eq!(m.rect(&one, area), None);
        // Two masters side by side.
        m.orientation = "left".into();
        assert_eq!(m.rect(&[([0.0, 0.0, 550.0, 300.0], false), ([0.0, 300.0, 550.0, 300.0], false), ([550.0, 0.0, 450.0, 600.0], false)], area), None);
    }

    #[test]
    fn scrolling_uses_column_width_and_direction_not_the_drop_edge() {
        let mut preview = HyprPreview { work: [0.0, 40.0, 1280.0, 604.0], tiled: [20.0, 60.0, 1240.0, 564.0], gaps: [5.0; 4], layout: Layout::Scrolling {
            anchor: Some([20.0, 60.0, 1240.0, 564.0]), axis: 0, reversed: false, width: 0.5, single: true, center: false,
        }};
        for pointer in [(30.0, 375.0), (1240.0, 85.0)] {
            assert_eq!(preview.rect(Zone::Tiled, pointer), Some([645.0, 60.0, 610.0, 564.0]));
        }
        preview.layout = Layout::Scrolling {
            anchor: None, axis: 0, reversed: false, width: 0.5, single: true, center: false,
        };
        assert_eq!(preview.rect(Zone::Tiled, (30.0, 375.0)), Some(preview.tiled));
    }
}
