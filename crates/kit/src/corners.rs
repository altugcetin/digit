//! Screen-corner assignments, independent of the shell and GTK.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Corner {
    pub const ALL: [Self; 4] = [Self::TopLeft, Self::TopRight, Self::BottomLeft, Self::BottomRight];

    pub const fn key(self) -> &'static str {
        match self {
            Self::TopLeft => "corner_top_left",
            Self::TopRight => "corner_top_right",
            Self::BottomLeft => "corner_bottom_left",
            Self::BottomRight => "corner_bottom_right",
        }
    }

    pub const fn is_top(self) -> bool {
        matches!(self, Self::TopLeft | Self::TopRight)
    }

    pub const fn is_left(self) -> bool {
        matches!(self, Self::TopLeft | Self::BottomLeft)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Panel {
    None,
    Workspaces,
    Control,
}

impl Panel {
    pub const fn value(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Workspaces => "workspaces",
            Self::Control => "control",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "workspaces" => Self::Workspaces,
            "control" => Self::Control,
            _ => Self::None,
        }
    }
}

pub fn read(prefs: &toml::Table) -> [Panel; 4] {
    let defaults = [Panel::Workspaces, Panel::None, Panel::None, Panel::Control];
    let mut assignment = [Panel::None; 4];
    for (i, corner) in Corner::ALL.into_iter().enumerate() {
        let panel = prefs.get(corner.key()).map_or(defaults[i], |v| v.as_str().map_or(Panel::None, Panel::parse));
        if panel == Panel::None || !assignment[..i].contains(&panel) {
            assignment[i] = panel;
        }
    }
    assignment
}

pub fn corner_of(assignment: &[Panel; 4], panel: Panel) -> Option<Corner> {
    if panel == Panel::None {
        return None;
    }
    Corner::ALL.into_iter().zip(assignment).find_map(|(corner, &at)| (at == panel).then_some(corner))
}

pub fn assign(mut assignment: [Panel; 4], corner: Corner, panel: Panel) -> [Panel; 4] {
    for (at, value) in Corner::ALL.into_iter().zip(&mut assignment) {
        if at == corner {
            *value = panel;
        } else if panel != Panel::None && *value == panel {
            *value = Panel::None;
        }
    }
    assignment
}

pub fn values(assignment: &[Panel; 4]) -> Vec<(&'static str, toml::Value)> {
    Corner::ALL.into_iter().zip(assignment).map(|(c, p)| (c.key(), toml::Value::String(p.value().into()))).collect()
}

/// The setting that holds the screens that have corners of their own: a table of tables, by
/// connector, each with the same four keys as the shared corners.
pub const MONITORS: &str = "monitor_corners";

/// The corners of the screen `output`: its own when it has them, the shared ones otherwise.
pub fn read_on(prefs: &toml::Table, output: &str) -> [Panel; 4] {
    match own(prefs).and_then(|t| t.get(output)).and_then(toml::Value::as_table) {
        Some(table) => read(table),
        None => read(prefs),
    }
}

/// The screens with corners of their own, and what each has.
pub fn monitors(prefs: &toml::Table) -> Vec<(String, [Panel; 4])> {
    own(prefs).map(|t| t.iter().filter_map(|(k, v)| Some((k.clone(), read(v.as_table()?)))).collect()).unwrap_or_default()
}

fn own(prefs: &toml::Table) -> Option<&toml::Table> {
    prefs.get(MONITORS).and_then(toml::Value::as_table)
}

/// The value of [`MONITORS`] once `output` has `assignment` for its own, the other screens kept.
pub fn with_monitor(prefs: &toml::Table, output: &str, assignment: &[Panel; 4]) -> toml::Value {
    let mut all = own(prefs).cloned().unwrap_or_default();
    all.insert(output.to_string(), toml::Value::Table(values(assignment).into_iter().map(|(k, v)| (k.to_string(), v)).collect()));
    toml::Value::Table(all)
}

#[cfg(test)]
mod tests {
    use super::*;
    use Corner::*;
    use Panel::*;

    #[test]
    fn defaults_and_corner_properties() {
        assert_eq!(read(&toml::Table::new()), [Workspaces, None, None, Control]);
        assert_eq!(Corner::ALL.map(Corner::key), ["corner_top_left", "corner_top_right", "corner_bottom_left", "corner_bottom_right"]);
        assert_eq!(Corner::ALL.map(Corner::is_top), [true, true, false, false]);
        assert_eq!(Corner::ALL.map(Corner::is_left), [true, false, true, false]);
        assert_eq!(corner_of(&read(&toml::Table::new()), Workspaces), Some(TopLeft));
        assert_eq!(corner_of(&[None; 4], Control), Option::None);
        assert_eq!(corner_of(&[None; 4], None), Option::None);
    }

    #[test]
    fn first_duplicate_wins_including_defaults() {
        let prefs = "corner_top_right = 'workspaces'\ncorner_bottom_left = 'control'".parse().unwrap();
        assert_eq!(read(&prefs), [Workspaces, None, Control, None]);
        let prefs = "corner_top_left = 'control'\ncorner_top_right = 'control'\ncorner_bottom_left = 'workspaces'".parse().unwrap();
        assert_eq!(read(&prefs), [Control, None, Workspaces, None]);
    }

    #[test]
    fn a_screen_keeps_corners_of_its_own_and_the_rest_share_theirs() {
        let shared: toml::Table = "corner_top_left = 'control'\ncorner_bottom_right = 'none'".parse().unwrap();
        assert_eq!(read_on(&shared, "HDMI-A-1"), [Control, None, None, None]);
        let mut prefs = shared.clone();
        prefs.insert(MONITORS.into(), with_monitor(&prefs, "HDMI-A-1", &[None, Workspaces, None, Control]));
        assert_eq!(read_on(&prefs, "HDMI-A-1"), [None, Workspaces, None, Control]);
        assert_eq!(read_on(&prefs, "DP-2"), [Control, None, None, None]);
        prefs.insert(MONITORS.into(), with_monitor(&prefs, "DP-2", &[Workspaces, None, None, None]));
        assert_eq!(read_on(&prefs, "HDMI-A-1"), [None, Workspaces, None, Control], "another screen's corners stay");
        assert_eq!(monitors(&prefs).len(), 2);
    }

    #[test]
    fn assign_moves_and_replaces_panels() {
        let a = [Workspaces, None, None, Control];
        assert_eq!(assign(a, BottomRight, Workspaces), [None, None, None, Workspaces]);
        assert_eq!(assign(a, TopRight, Control), [Workspaces, Control, None, None]);
        assert_eq!(assign(a, TopLeft, Workspaces), a);
    }

    #[test]
    fn clearing_only_changes_the_selected_corner() {
        assert_eq!(assign([Workspaces, None, None, Control], TopLeft, None), [None, None, None, Control]);
    }

    #[test]
    fn values_round_trip_every_valid_assignment() {
        for workspace in Corner::ALL.into_iter().map(Some).chain([Option::None]) {
            for control in Corner::ALL.into_iter().map(Some).chain([Option::None]) {
                if workspace.is_some() && workspace == control {
                    continue;
                }
                let mut a = [None; 4];
                if let Some(c) = workspace {
                    a = assign(a, c, Workspaces);
                }
                if let Some(c) = control {
                    a = assign(a, c, Control);
                }
                let prefs = values(&a).into_iter().map(|(k, v)| (k.into(), v)).collect();
                assert_eq!(read(&prefs), a);
                assert_eq!(corner_of(&a, Workspaces), workspace);
                assert_eq!(corner_of(&a, Control), control);
            }
        }
    }

    #[test]
    fn invalid_values_disable_a_corner_and_legacy_keys_are_ignored() {
        for p in [None, Workspaces, Control] {
            assert_eq!(Panel::parse(p.value()), p);
        }
        assert_eq!(Panel::parse("unknown"), None);
        let prefs = "hot_corner = false\ncontrol_corner = false".parse().unwrap();
        assert_eq!(read(&prefs), [Workspaces, None, None, Control]);
        let prefs = "corner_top_left = false\ncorner_bottom_right = 'unknown'".parse().unwrap();
        assert_eq!(read(&prefs), [None; 4]);
    }
}
