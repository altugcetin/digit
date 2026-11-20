use super::*;
#[test]
fn double_click_survives_layer_replacement_and_rejects_other_icons_or_distant_presses() {
    let press = ClickRecord {
        name: "file".into(),
        time: 1_000_000,
        point: (100.5, 200.25),
    };
    assert!(press.follows("file", 1_200_000, (102.0, 201.0), 400_000, 5.0));
    assert!(!press.follows("other", 1_200_000, (102.0, 201.0), 400_000, 5.0));
    assert!(!press.follows("file", 1_500_000, (102.0, 201.0), 400_000, 5.0));
    assert!(!press.follows("file", 1_200_000, (110.0, 201.0), 400_000, 5.0));
}

#[test]
fn focus_correction_targets_new_window_without_overriding_a_newer_focus() {
    assert!(needs_focus_correction(20, Some(10), Some(10)));
    assert!(needs_focus_correction(20, Some(10), None));
    assert!(!needs_focus_correction(20, Some(10), Some(20)));
    assert!(!needs_focus_correction(20, Some(10), Some(30)));
}

#[test]
fn keyboard_is_requested_by_selection_or_a_desktop_click_and_revoked_by_windows_or_overlays() {
    for selected in [false, true] {
        assert_eq!(
            keyboard_requested(selected, KeyboardEvent::Select),
            selected
        );
        for event in [
            KeyboardEvent::Clear,
            KeyboardEvent::Window,
            KeyboardEvent::Overlay,
        ] {
            assert!(!keyboard_requested(selected, event));
        }
    }
    // Clicking the desktop makes it active even with nothing selected (Ctrl+A).
    assert!(keyboard_requested(false, KeyboardEvent::Clicked));
    // Visible selection is not permission to reacquire after a window event.
    let selected = true;
    assert!(!keyboard_requested(selected, KeyboardEvent::Window));
    assert!(keyboard_requested(selected, KeyboardEvent::Select));
}

#[test]
fn connected_connector_owns_an_item_and_missing_or_legacy_items_use_primary() {
    let connected = vec!["DP-1".into(), "eDP-1".into()];
    assert_eq!(item_output(Some("DP-1"), &connected, "eDP-1"), "DP-1");
    assert_eq!(item_output(Some("HDMI-A-1"), &connected, "eDP-1"), "eDP-1");
    assert_eq!(item_output(None, &connected, "eDP-1"), "eDP-1");
}

#[test]
fn outputs_place_independently_and_new_items_belong_to_primary() {
    let names = vec!["left".into(), "right".into(), "new".into()];
    let connected = vec!["DP-1".into(), "eDP-1".into()];
    let mut outputs = BTreeMap::from([
        ("left".into(), "DP-1".into()),
        ("right".into(), "eDP-1".into()),
    ]);
    let mut saved = Positions::new();
    let left = place_output(
        &names,
        &saved,
        &outputs,
        &connected,
        "eDP-1",
        "DP-1",
        grid(),
    );
    let right = place_output(
        &names,
        &saved,
        &outputs,
        &connected,
        "eDP-1",
        "eDP-1",
        grid(),
    );
    assert_eq!(left.keys().cloned().collect::<Vec<_>>(), vec!["left"]);
    assert_eq!(right.len(), 2);
    assert_eq!(left["left"], right["right"]); // Same coordinates on different outputs are fine.
    assert_separate(&right);
    remember_output(&right, &mut saved, &mut outputs, "eDP-1");
    assert_eq!(outputs["new"], "eDP-1");
}

#[test]
fn unplugged_icons_get_free_guest_spots_and_replug_restores_exact_home() {
    let names = vec!["home".into(), "guest".into()];
    let mut outputs = BTreeMap::from([
        ("home".into(), "eDP-1".into()),
        ("guest".into(), "DP-1".into()),
    ]);
    let home = FreePos {
        x: 650.5,
        y: 350.25,
        page: 2,
    };
    let mut saved = Positions::from([("guest".into(), home)]);
    let before = saved.clone();
    let fallback = place_output(
        &names,
        &saved,
        &outputs,
        &["eDP-1".into()],
        "eDP-1",
        "eDP-1",
        grid(),
    );
    assert_separate(&fallback);
    assert_eq!(fallback["guest"].page, 0);
    remember_output(&fallback, &mut saved, &mut outputs, "eDP-1");
    assert_eq!(saved["guest"], before["guest"]);
    assert_eq!(outputs["guest"], "DP-1");
    let restored = place_output(
        &names,
        &saved,
        &outputs,
        &["eDP-1".into(), "DP-1".into()],
        "eDP-1",
        "DP-1",
        grid(),
    );
    assert_eq!(restored, Positions::from([("guest".into(), home)]));
}

#[test]
fn output_geometry_clamps_only_its_own_icons_and_preserves_other_homes() {
    let names = vec!["small".into(), "large".into()];
    let connected = vec!["DP-1".into(), "eDP-1".into()];
    let outputs = BTreeMap::from([
        ("small".into(), "eDP-1".into()),
        ("large".into(), "DP-1".into()),
    ]);
    let far = FreePos {
        x: 1700.75,
        y: 900.5,
        page: 0,
    };
    let saved = Positions::from([("small".into(), far), ("large".into(), far)]);
    let small_grid = Grid::new(800, 600, 72);
    let small = place_output(
        &names, &saved, &outputs, &connected, "eDP-1", "eDP-1", small_grid,
    );
    let large = place_output(
        &names,
        &saved,
        &outputs,
        &connected,
        "eDP-1",
        "DP-1",
        Grid::new(2560, 1440, 40),
    );
    assert_eq!(small["small"], small_grid.clamp(far));
    assert_eq!(large["large"], far);
}

#[test]
fn state_keeps_connector_names_and_migrates_positions_without_them() {
    let positions = Positions::from([(
        "file".into(),
        FreePos {
            x: 50.5,
            y: 100.25,
            page: 1,
        },
    )]);
    let outputs = BTreeMap::from([("file".into(), "DP-3".into())]);
    let (restored, connectors) =
        decode_state(&encode_state(&positions, &outputs).unwrap()).unwrap();
    assert_eq!(restored, positions);
    assert_eq!(connectors, outputs);
    let (legacy, outputs) = decode_state(&encode(&positions).unwrap()).unwrap();
    assert_eq!(legacy, positions);
    assert!(outputs.is_empty());
}

#[test]
fn cross_output_group_drop_uses_target_logical_coordinates_and_original_grab() {
    let drag = Drag {
        start: (540.25, 220.5),
        origins: BTreeMap::from([("a".into(), (500.0, 200.0)), ("b".into(), (620.0, 250.0))]),
    };
    let dropped = drag.dropped(Grid::new(1280, 720, 50), 240.75, 160.25, 0);
    assert_eq!(dropped["a"].x, 200.5);
    assert_eq!(dropped["a"].y, 139.75);
    assert_eq!(dropped["b"].x - dropped["a"].x, 120.0);
    assert_eq!(dropped["b"].y - dropped["a"].y, 50.0);
}

#[test]
fn local_visual_follows_desktop_reentry_but_never_restores_the_source_on_leave() {
    assert!(drag_items_visible(false, true, false, DragEnd::Active));
    for over in [true, false, true, false] {
        assert_eq!(drag_items_visible(true, true, over, DragEnd::Active), over);
    }
    assert!(!drag_items_visible(true, true, false, DragEnd::Performed));
    assert!(drag_items_visible(true, true, false, DragEnd::Local));
    // On niri the compositor owns the visual even over the desktop.
    assert!(!drag_items_visible(true, false, true, DragEnd::Active));
    // A terminal restoration removes the system drag before showing items.
    assert!(drag_items_visible(false, true, false, DragEnd::Ended));
}

#[test]
fn frozen_preview_keeps_the_press_across_windows_and_desktop_reentry() {
    let press = (337.75, 229.25);
    let drag = Drag {
        start: press,
        origins: [("a".into(), (300.25, 200.5)), ("b".into(), (20.5, 60.25))]
            .into_iter()
            .collect(),
    };
    let (_, hot) = preview_bounds(&drag.origins, press);
    for pointer in [(351.0, 237.5), (951.0, 537.5), (401.0, 317.5)] {
        // The same frozen surface follows the pointer on either target;
        // neither leaving nor re-entering may rebase it to the old cell.
        let moved = drag.at(pointer.0, pointer.1);
        for (name, &(x, y)) in &drag.origins {
            let in_preview = (x + hot.0 as f64 - press.0, y + hot.1 as f64 - press.1);
            assert_eq!(
                moved[name],
                (
                    pointer.0 - hot.0 as f64 + in_preview.0,
                    pointer.1 - hot.1 as f64 + in_preview.1
                )
            );
        }
    }
}

#[test]
fn preview_hotspot_is_the_press_minus_bounds_origin_for_one_item_and_groups() {
    let cell = (300.0, 200.0);
    for grab in [(0.0, 0.0), (37.0, 29.0), (CELL_W as f64 - 1.0, 80.0)] {
        let press = (cell.0 + grab.0, cell.1 + grab.1);
        for grouped in [false, true] {
            let mut origins = BTreeMap::from([("grabbed".into(), cell)]);
            if grouped {
                origins.insert("above-left".into(), (20.0, 60.0));
                origins.insert("below-right".into(), (480.0, 350.0));
            }
            let (bounds, hot) = preview_bounds(&origins, press);
            assert_eq!(hot.0 as f64, press.0 - bounds.x);
            assert_eq!(hot.1 as f64, press.1 - bounds.y);
            assert_eq!(hot.0 as f64 - (cell.0 - bounds.x), grab.0);
            assert_eq!(hot.1 as f64 - (cell.1 - bounds.y), grab.1);

            let drag = Drag {
                start: press,
                origins,
            };
            // Crossing the threshold moves both the cells and their bounds;
            // it must not rebase the grab to the threshold event's position.
            let pointer = (press.0 + 19.0, press.1 - 13.0);
            let moved = drag.at(pointer.0, pointer.1);
            let (moved_bounds, moved_hot) = preview_bounds(&moved, pointer);
            assert_eq!(moved_hot, hot);
            assert_eq!(moved_hot.0 as f64, pointer.0 - moved_bounds.x);
            assert_eq!(moved_hot.1 as f64, pointer.1 - moved_bounds.y);
        }
    }
}
#[test]
fn preview_fits_one_icon_and_its_shadow_with_the_exact_fractional_grab() {
    let origins = [("a".into(), (200.25, 100.5))].into_iter().collect();
    let pointer = (237.75, 129.25);
    let (bounds, hot) = preview_bounds(&origins, pointer);
    assert_eq!(
        (bounds.x, bounds.y, bounds.w, bounds.h),
        (192.0, 92.0, 129.0, 133.0)
    );
    assert_eq!(hot, (46, 37));
    assert!(bounds.contains(pointer.0, pointer.1));
    // The surface is positioned at pointer - integer hotspot. Its contents
    // compensate the fractional remainder, so no grab distance is rounded.
    let in_preview = (
        origins["a"].0 + hot.0 as f64 - pointer.0,
        origins["a"].1 + hot.1 as f64 - pointer.1,
    );
    assert_eq!(pointer.0 - hot.0 as f64 + in_preview.0, origins["a"].0);
    assert_eq!(pointer.1 - hot.1 as f64 + in_preview.1, origins["a"].1);
}
#[test]
fn preview_group_includes_icons_left_and_above_the_grab_and_keeps_its_first_frame() {
    let drag = Drag {
        start: (337.75, 229.25),
        origins: [("a".into(), (300.25, 200.5)), ("b".into(), (20.5, 60.25))]
            .into_iter()
            .collect(),
    };
    let pointer = (351.0, 237.5); // Already moved before the DND threshold.
    let moved = drag.at(pointer.0, pointer.1);
    let (bounds, hot) = preview_bounds(&moved, pointer);
    for &(x, y) in moved.values() {
        assert!(bounds.contains(x - 6.0, y - 6.0));
        assert!(bounds.contains(x + CELL_W as f64 + 5.0, y + CELL_H as f64 + 5.0));
        let in_preview = (
            x.round() + hot.0 as f64 - pointer.0,
            y.round() + hot.1 as f64 - pointer.1,
        );
        assert_eq!(pointer.0 - hot.0 as f64 + in_preview.0, x.round());
        assert_eq!(pointer.1 - hot.1 as f64 + in_preview.1, y.round());
    }
    let dropped = drag.dropped(grid(), pointer.0, pointer.1, 0);
    for (name, &(x, y)) in &moved {
        assert_eq!((dropped[name].x, dropped[name].y), (x, y));
    }
}
#[test]
fn external_drop_cascade_is_centered_clamped_and_wins_over_automatic_placement() {
    let positions = drop_cascade(grid(), (250.5, 200.25), 1, 3);
    assert_eq!(
        positions[0],
        FreePos {
            x: 194.5,
            y: 142.25,
            page: 1
        }
    );
    assert_eq!(
        positions[2],
        FreePos {
            x: 242.5,
            y: 190.25,
            page: 1
        }
    );
    for pointer in [(0.0, 0.0), (800.0, 600.0)] {
        assert!(
            drop_cascade(grid(), pointer, 0, 5)
                .iter()
                .all(|p| grid().clamp(*p) == *p)
        );
    }
    assert!(drop_cascade(grid(), (0.0, 0.0), 0, 0).is_empty());
    let saved = [("incoming".into(), positions[0])].into_iter().collect();
    let placed = place(&["incoming".into(), "other".into()], &saved, grid());
    assert_eq!(placed["incoming"], positions[0]);
}
#[test]
fn failed_import_removes_its_reservation_but_preserves_existing_or_newer_placements() {
    let seeded = drop_cascade(grid(), (250.0, 200.0), 0, 1)[0];
    let old = grid().nth(0);
    for previous in [None, Some(old)] {
        let mut saved: Positions = [("incoming".into(), seeded)].into_iter().collect();
        let mut positions = saved.clone();
        let mut pending = saved.clone();
        rollback_import(
            &mut saved,
            &mut positions,
            &mut pending,
            "incoming".into(),
            seeded,
            previous,
        );
        assert!(pending.is_empty());
        assert_eq!(saved.get("incoming").copied(), previous);
        assert_eq!(positions, saved);
    }
    let mut saved: Positions = [("incoming".into(), old)].into_iter().collect();
    let mut positions = saved.clone();
    let mut pending = [("incoming".into(), seeded)].into_iter().collect();
    rollback_import(
        &mut saved,
        &mut positions,
        &mut pending,
        "incoming".into(),
        seeded,
        None,
    );
    assert!(pending.is_empty());
    assert_eq!(saved["incoming"], old);
    assert_eq!(positions["incoming"], old);
}
#[test]
fn collision_names_preserve_suffixes_directories_and_non_utf8_names() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    assert_eq!(
        collision_name(OsStr::new("a.txt"), false, |_| false),
        "a.txt"
    );
    assert_eq!(
        collision_name(OsStr::new("a.txt"), false, |n| n == "a.txt"
            || n == "a (2).txt"),
        "a (3).txt"
    );
    assert_eq!(
        collision_name(OsStr::new("folder.ext"), true, |n| n == "folder.ext"),
        "folder.ext (2)"
    );
    assert_eq!(
        collision_name(OsStr::new(".hidden"), false, |n| n == ".hidden"),
        ".hidden (2)"
    );
    let name = OsStr::from_bytes(b"a\xff.txt");
    assert_eq!(
        collision_name(name, false, |n| n == name),
        OsStr::from_bytes(b"a\xff (2).txt")
    );
}
#[test]
fn import_plans_reserve_final_names_before_any_transfer_and_keep_existing_desktop_files() {
    let dir = temp();
    let desktop = dir.join("Desktop");
    std::fs::create_dir_all(&desktop).unwrap();
    std::fs::create_dir(dir.join("first")).unwrap();
    std::fs::create_dir(dir.join("second")).unwrap();
    std::fs::write(desktop.join("a.txt"), b"desktop").unwrap();
    std::fs::write(dir.join("first/a.txt"), b"first").unwrap();
    std::fs::write(dir.join("second/a.txt"), b"second").unwrap();
    // Dangling symlinks also occupy names.
    std::os::unix::fs::symlink("missing", desktop.join("a (2).txt")).unwrap();
    let paths = [
        dir.join("first/a.txt"),
        dir.join("second/a.txt"),
        desktop.join("a.txt"),
    ];
    let uris = paths
        .iter()
        .map(|p| gio::File::for_path(p).uri().to_string())
        .collect();
    let plans = plan_imports(uris, &desktop, BTreeSet::new(), false, false);
    assert_eq!(
        plans.iter().map(|p| p.dest.clone()).collect::<Vec<_>>(),
        [
            desktop.join("a (3).txt"),
            desktop.join("a (4).txt"),
            desktop.join("a.txt"),
        ]
    );
    assert!(!desktop.join("a (3).txt").exists());
    assert!(!desktop.join("a (4).txt").exists());
    for plan in plans {
        import(&gio::File::for_uri(&plan.uri), &plan.dest, false, false).unwrap();
    }
    assert_eq!(std::fs::read(desktop.join("a.txt")).unwrap(), b"desktop");
    assert_eq!(std::fs::read(desktop.join("a (3).txt")).unwrap(), b"first");
    assert_eq!(std::fs::read(desktop.join("a (4).txt")).unwrap(), b"second");
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn drag_end_waits_for_transfer_and_finishes_once_in_either_signal_order() {
    for moved in [false, true] {
        for source_first in [false, true] {
            let mut state = DragEnd::default();
            assert_eq!(state.event(DragEvent::Performed), None);
            if source_first {
                assert_eq!(state.event(DragEvent::SourceEnd), None);
            }
            assert_eq!(
                state.event(DragEvent::Finished { moved }),
                Some(if moved {
                    DragResult::AwaitRemoval
                } else {
                    DragResult::Restore
                })
            );
            assert_eq!(state.event(DragEvent::SourceEnd), None);
            assert_eq!(state.event(DragEvent::Cancel), None);
            assert_eq!(state.event(DragEvent::Finished { moved }), None);
        }
    }
    for performed in [false, true] {
        let mut state = DragEnd::default();
        if performed {
            state.event(DragEvent::Performed);
        }
        assert_eq!(state.event(DragEvent::Cancel), Some(DragResult::Restore));
        assert_eq!(state.event(DragEvent::SourceEnd), None);
        assert_eq!(state.event(DragEvent::Cancel), None);
    }
}
#[test]
fn local_drop_can_precede_or_follow_performed_and_is_never_an_external_move() {
    for performed_first in [false, true] {
        for cancelled in [false, true] {
            let mut state = DragEnd::default();
            if performed_first {
                state.event(DragEvent::Performed);
            }
            assert_eq!(state.event(DragEvent::LocalDrop), None);
            if !performed_first {
                state.event(DragEvent::Performed);
            }
            assert_eq!(state, DragEnd::Local);
            assert_eq!(
                state.event(if cancelled {
                    DragEvent::Cancel
                } else {
                    DragEvent::Finished { moved: true }
                }),
                Some(DragResult::KeepLocal)
            );
            assert_eq!(state.event(DragEvent::SourceEnd), None);
        }
    }
}
fn grid() -> Grid {
    Grid::new(800, 600, 50)
}
fn names(count: usize) -> Vec<String> {
    (0..count).map(|n| format!("file{n:03}")).collect()
}
fn temp() -> PathBuf {
    std::env::temp_dir().join(format!(
        "digit-deskicons-{}-{}",
        std::process::id(),
        glib::random_int()
    ))
}
fn assert_separate(positions: &Positions) {
    let positions: Vec<_> = positions.values().collect();
    for (n, p) in positions.iter().enumerate() {
        assert!(positions[n + 1..].iter().all(|other| !p.overlaps(**other)));
    }
}
#[test]
fn new_items_fill_down_from_top_right_without_displacing_saved_items() {
    let g = grid();
    let names = names(8);
    let saved = [(names[2].clone(), g.nth(0))].into_iter().collect();
    let p = place(&names, &saved, g);
    assert_eq!(p[&names[2]], g.nth(0));
    assert_eq!(p[&names[0]], g.nth(1));
    assert!(g.nth(0).x > g.nth(g.rows as usize).x);
    assert_separate(&p);
}
fn battery_grid(placement: &str, clock: &str) -> Grid {
    let mut g = grid();
    g.reserved[0] = crate::batterywidget::card_rect(
        placement,
        crate::batterywidget::SHOWN,
        (800, 600),
        26,
        if clock == "left" { 214 } else { 0 },
        [0; 3],
    )
    .map(|r| Rect {
        x: r.x,
        y: r.y,
        w: r.w,
        h: r.h,
    });
    g
}
#[test]
fn automatic_placement_skips_battery_cells_in_every_corner_and_page() {
    let names = names(100);
    for corner in ["top_left", "top_right", "bottom_left", "bottom_right"] {
        for clock in ["off", "left"] {
            let g = battery_grid(corner, clock);
            assert!(
                !corner.starts_with("top")
                    || (0..(g.cols * g.rows) as usize).any(|n| g.reserved(g.nth(n)))
            );
            let p = place(&names, &Positions::new(), g);
            assert!(p.values().all(|p| !g.reserved(*p)), "{corner}, {clock}");
            assert_separate(&p);
            assert_eq!(place(&names, &p, g), p);
        }
    }
}
#[test]
fn automatic_placement_skips_free_saved_rectangles_and_the_battery_card() {
    let g = battery_grid("top_left", "off");
    let names = names(20);
    let saved_pos = FreePos {
        x: g.nth(0).x - 10.5,
        y: 65.25,
        page: 0,
    };
    let saved = [(names[10].clone(), saved_pos)].into_iter().collect();
    let p = place(&names, &saved, g);
    assert_eq!(p[&names[10]], saved_pos);
    assert!(!p.values().any(|p| g.reserved(*p)));
    assert_separate(&p);
    assert_ne!(p[&names[0]], g.nth(0));
    assert_ne!(p[&names[1]], g.nth(1));
}
#[test]
fn saved_positions_under_a_widget_move_out_beside_it() {
    let names = names(3);
    let g = battery_grid("top_right", "off");
    let saved = [(names[0].clone(), g.nth(0)), (names[1].clone(), g.nth(0))]
        .into_iter()
        .collect();
    assert!(g.reserved(g.nth(0)));
    let p = place(&names, &saved, g);
    assert!(!p.values().any(|p| g.reserved(*p)));
    assert_separate(&p);
    // Just outside the card, never farther than the nearest free grid cell.
    let far = |c: FreePos| (c.x - g.nth(0).x).powi(2) + (c.y - g.nth(0).y).powi(2);
    let cell = g.automatic_cells().into_iter().map(far).fold(f64::INFINITY, f64::min);
    assert!(far(p[&names[0]]) <= cell);
    let card = g.reserved.iter().flatten().next().copied().unwrap();
    let gap = |c: FreePos| {
        let r = c.rect();
        (card.x - (r.x + r.w)).max(r.x - (card.x + card.w)).max(card.y - (r.y + r.h)).max(r.y - (card.y + card.h))
    };
    assert!((0.0..=8.0).contains(&gap(p[&names[0]])), "{}", gap(p[&names[0]]));
    // Overlapping saved positions stay overlapping only where nothing reserves them.
    let free = g.automatic_cells()[3];
    let p = place(&names, &[(names[0].clone(), free), (names[1].clone(), free)].into_iter().collect(), g);
    assert_eq!((p[&names[0]], p[&names[1]]), (free, free));
}
#[test]
fn smaller_screen_pulls_restored_positions_to_the_nearest_edge_without_regridding() {
    let names = names(3);
    let old = Grid::new(1920, 1080, 50);
    let edge = FreePos {
        x: 1700.5,
        y: 900.25,
        page: 2,
    };
    let untouched = FreePos {
        x: 223.75,
        y: 101.5,
        page: 0,
    };
    let saved = [
        (names[0].clone(), edge),
        (names[1].clone(), untouched),
        (names[2].clone(), edge),
    ]
    .into_iter()
    .collect();
    assert_eq!(place(&names, &saved, old), saved);
    let p = place(&names, &saved, grid());
    assert_eq!(
        p[&names[0]],
        FreePos {
            x: 688.0,
            y: 484.0,
            page: 2
        }
    );
    assert_eq!(p[&names[1]], untouched);
    assert_eq!(p[&names[2]], p[&names[0]]);
}
#[test]
fn automatic_results_stay_stable_when_earlier_names_arrive_or_card_changes() {
    let old_names = vec!["b".into(), "c".into()];
    let saved = place(&old_names, &Positions::new(), grid());
    let new_names = vec!["a".into(), "b".into(), "c".into()];
    let card = battery_grid("top_right", "off");
    let p = place(&new_names, &saved, card);
    // Icons the card now covers move aside; the rest stay where they were.
    for name in old_names {
        if card.reserved(saved[&name]) {
            assert!(!card.reserved(p[&name]));
        } else {
            assert_eq!(p[&name], saved[&name]);
        }
    }
    assert_eq!(place(&new_names, &p, grid()), p);
}
#[test]
fn five_hundred_items_get_separate_reachable_pages() {
    let names = names(500);
    let g = grid();
    let p = place(&names, &Positions::new(), g);
    assert_eq!(p.len(), 500);
    assert_separate(&p);
    assert!(p.values().all(|p| g.clamp(*p) == *p));
    assert!(p.values().any(|p| p.page > 0));
    assert_eq!(place(&names, &p, g), p);
}
#[test]
fn drag_follows_first_motion_and_dnd_handoff_without_rebasing_the_grab() {
    let drag = Drag {
        start: (237.25, 129.5),
        origins: [("a".into(), (200.0, 100.0)), ("b".into(), (323.5, 211.25))]
            .into_iter()
            .collect(),
    };
    // Sub-threshold motion is visible immediately. The first DND motion
    // continues the same translation rather than recapturing moved origins.
    for (dx, dy) in [(0.0, 0.0), (0.75, 1.25), (12.0, 3.0), (54.5, 68.25)] {
        let pointer = (drag.start.0 + dx, drag.start.1 + dy);
        let p = drag.at(pointer.0, pointer.1);
        assert_eq!(p["a"], (200.0 + dx, 100.0 + dy));
        assert_eq!(pointer.0 - p["a"].0, 37.25);
        assert_eq!(pointer.1 - p["a"].1, 29.5);
        assert_eq!(p["b"].0 - p["a"].0, 123.5);
        assert_eq!(p["b"].1 - p["a"].1, 111.25);
        let dropped = drag.dropped(grid(), pointer.0, pointer.1, 0);
        assert_eq!((dropped["a"].x, dropped["a"].y), p["a"]);
    }
}
#[test]
fn drag_leave_reentry_and_cancel_keep_the_original_group_immutable() {
    let drag = Drag {
        start: (237.25, 129.5),
        origins: [("a".into(), (200.0, 100.0)), ("b".into(), (323.5, 211.25))]
            .into_iter()
            .collect(),
    };
    let origins = drag.origins.clone();
    let before_leave = drag.at(412.5, 310.25);
    assert_ne!(before_leave, origins);
    assert_eq!(drag.restored(), origins);
    assert_eq!(drag.at(412.5, 310.25), before_leave);
    let _ = drag.dropped(grid(), -9999.0, 9999.0, 0);
    // Cancellation, including a release before DND, never uses the last
    // pointer position or a clamped drop as its return destination.
    assert_eq!(drag.restored(), origins);
}
#[test]
fn dragging_preserves_a_selected_group_and_exports_a_toggled_off_anchor_alone() {
    let selected: BTreeSet<_> = ["a".into(), "b".into()].into_iter().collect();
    assert_eq!(drag_selection(&selected, "b"), selected);
    assert_eq!(
        drag_selection(&selected, "c"),
        ["c".into()].into_iter().collect()
    );
    assert_eq!(selected.len(), 2);
}
#[test]
fn drop_keeps_exact_fractional_coordinates_and_selection_offsets() {
    let drag = Drag {
        start: (230.0, 120.0),
        origins: [("a".into(), (200.0, 100.0)), ("b".into(), (323.5, 211.25))]
            .into_iter()
            .collect(),
    };
    let p = drag.dropped(grid(), 247.25, 149.5, 1);
    assert_eq!(
        p["a"],
        FreePos {
            x: 217.25,
            y: 129.5,
            page: 1
        }
    );
    assert_eq!(
        p["b"],
        FreePos {
            x: 340.75,
            y: 240.75,
            page: 1
        }
    );
    // Dropping onto an existing icon is allowed; restoration keeps both there.
    let mut saved = p.clone();
    saved.insert("c".into(), p["a"]);
    assert_eq!(
        place(&["a".into(), "b".into(), "c".into()], &saved, grid()),
        saved
    );
}
#[test]
fn clamping_keeps_icons_below_reserve_and_group_offsets_at_edges() {
    let g = grid();
    assert_eq!(
        g.clamp(FreePos {
            x: -20.5,
            y: -10.0,
            page: 0
        }),
        FreePos {
            x: 0.0,
            y: 50.0,
            page: 0
        }
    );
    assert_eq!(
        g.clamp(FreePos {
            x: 9999.0,
            y: 9999.0,
            page: 0
        }),
        FreePos {
            x: 688.0,
            y: 484.0,
            page: 0
        }
    );
    let drag = Drag {
        start: (230.0, 120.0),
        origins: [("a".into(), (200.0, 100.0)), ("b".into(), (323.5, 211.25))]
            .into_iter()
            .collect(),
    };
    for pointer in [(-9999.0, -9999.0), (9999.0, 9999.0)] {
        let p = drag.dropped(g, pointer.0, pointer.1, 0);
        assert_eq!(p["b"].x - p["a"].x, 123.5);
        assert_eq!(p["b"].y - p["a"].y, 111.25);
        assert!(p.values().all(|p| g.clamp(*p) == *p));
    }
}
#[test]
fn sort_by_name_discards_free_positions_and_restores_grid_order() {
    let names = names(30);
    let saved = names
        .iter()
        .map(|name| {
            (
                name.clone(),
                FreePos {
                    x: 123.25,
                    y: 111.5,
                    page: 5,
                },
            )
        })
        .collect();
    assert_eq!(place(&names, &saved, grid()), saved);
    // Sort clears saved positions before using the same name-ordered placement.
    let sorted = place(&names, &Positions::new(), grid());
    for (n, name) in names.iter().enumerate() {
        assert_eq!(sorted[name], grid().nth(n));
    }
    assert_eq!(place(&names, &sorted, grid()), sorted);
}
#[test]
fn rubber_band_works_in_both_directions_and_preserves_ctrl_selection() {
    let r = Rect {
        x: 10.0,
        y: 10.0,
        w: 100.0,
        h: 100.0,
    };
    let items = vec![("one".into(), r), ("two".into(), Rect { x: 200.0, ..r })];
    let initial = ["two".into()].into_iter().collect();
    for band in [
        Rect::between((0.0, 0.0), (50.0, 50.0)),
        Rect::between((50.0, 50.0), (0.0, 0.0)),
    ] {
        assert_eq!(
            rubber_select(band, &items, &initial, false),
            ["one".into()].into_iter().collect()
        );
        assert_eq!(rubber_select(band, &items, &initial, true).len(), 2);
    }
    assert!(
        rubber_select(
            Rect::between((0.0, 0.0), (0.0, 0.0)),
            &items,
            &initial,
            false
        )
        .is_empty()
    );
}
#[test]
fn positions_roundtrip_names_and_reject_corrupt_data() {
    let p = [("notes\n\"日本語.txt".into(), grid().nth(3))]
        .into_iter()
        .collect();
    assert_eq!(decode(&encode(&p).unwrap()).unwrap(), p);
    assert!(decode(b"truncated {").is_err());
    assert!(decode(br#"{"bad":{"col":-1,"row":0}}"#).unwrap().is_empty());
}
#[test]
fn old_format_entries_are_ignored_without_losing_new_positions() {
    let old = br#"{"a":{"col":1,"row":2,"page":0},"b":{"col":0,"row":0}}"#;
    assert!(decode(old).unwrap().is_empty());
    let mixed = br#"{"old":{"col":1,"row":2},"new":{"x":123.5,"y":99.25}}"#;
    assert_eq!(
        decode(mixed).unwrap(),
        [(
            "new".into(),
            FreePos {
                x: 123.5,
                y: 99.25,
                page: 0
            }
        )]
        .into_iter()
        .collect()
    );
}
#[test]
fn unreadable_or_corrupt_state_uses_defaults() {
    let dir = temp();
    std::fs::create_dir_all(&dir).unwrap();
    assert!(read_positions(&dir).unwrap().is_empty());
    assert!(read_positions(&dir.join("missing")).unwrap().is_empty());
    std::fs::write(dir.join("broken.json"), b"truncated {").unwrap();
    assert!(read_positions(&dir.join("broken.json")).unwrap().is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn atomic_save_replaces_complete_state_and_reports_failure() {
    let dir = temp();
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("icons.json");
    let p = [("a".into(), grid().nth(0))].into_iter().collect();
    write_positions(&path, &p).unwrap();
    assert_eq!(read_positions(&path).unwrap(), p);
    write_positions(&path, &Positions::new()).unwrap();
    assert!(read_positions(&path).unwrap().is_empty());
    assert!(write_positions(&path.join("impossible.json"), &p).is_err());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn copy_rejects_descendants_including_symlinked_parents() {
    let dir = temp();
    let src = dir.join("src");
    std::fs::create_dir_all(src.join("Desktop")).unwrap();
    std::os::unix::fs::symlink(src.join("Desktop"), dir.join("alias")).unwrap();
    for dest in [src.clone(), src.join("Desktop/copy"), dir.join("alias/copy")] {
        assert!(copy_tree(&src, &dest).is_err());
    }
    assert_eq!(std::fs::read_dir(src.join("Desktop")).unwrap().count(), 0);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn cut_cleanup_preserves_new_changed_and_replaced_entries() {
    let dir = temp();
    let src = dir.join("src");
    std::fs::create_dir_all(src.join("sub")).unwrap();
    for name in ["unchanged", "changed", "mtime", "replaced"] {
        std::fs::write(src.join(name), b"original").unwrap();
    }
    std::os::unix::fs::symlink("unchanged", src.join("link")).unwrap();
    let copied = copy_tree(&src, &dir.join("dest")).unwrap();
    std::fs::write(src.join("new"), b"new").unwrap();
    std::fs::write(src.join("changed"), b"changed size").unwrap();
    let file = std::fs::OpenOptions::new().write(true).open(src.join("mtime")).unwrap();
    file.set_modified(std::time::SystemTime::UNIX_EPOCH).unwrap();
    std::fs::rename(src.join("replaced"), dir.join("old")).unwrap();
    std::fs::write(src.join("replaced"), b"original").unwrap();
    assert!(remove_import_source(&copied).is_err());
    for name in ["new", "changed", "mtime", "replaced"] { assert!(src.join(name).exists()); }
    assert!(!src.join("unchanged").exists());
    assert!(src.join("link").symlink_metadata().is_err());
    assert!(!src.join("sub").exists());
    assert_eq!(std::fs::read(dir.join("dest/changed")).unwrap(), b"original");
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn copy_preserves_original_symlinks_and_never_overwrites() {
    let dir = temp();
    std::fs::create_dir_all(dir.join("src/sub")).unwrap();
    std::fs::write(dir.join("src/sub/a"), b"original").unwrap();
    std::os::unix::fs::symlink("sub/a", dir.join("src/link")).unwrap();
    copy_tree(&dir.join("src"), &dir.join("dest")).unwrap();
    assert_eq!(std::fs::read(dir.join("dest/link")).unwrap(), b"original");
    assert_eq!(
        std::fs::read_link(dir.join("dest/link")).unwrap(),
        PathBuf::from("sub/a")
    );
    assert!(copy_tree(&dir.join("src/sub/a"), &dir.join("dest/sub/a")).is_err());
    assert!(dir.join("src/sub/a").exists());
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn scan_hides_dotfiles_and_reads_desktop_entry_metadata() {
    let dir = temp();
    std::fs::create_dir_all(dir.join("Folder")).unwrap();
    std::fs::write(dir.join(".hidden"), b"hidden").unwrap();
    std::fs::write(
        dir.join("launch.desktop"),
        "[Desktop Entry]\nType=Application\nName=Desktop test\nIcon=folder\nExec=true\n",
    )
    .unwrap();
    let items = scan(&dir).unwrap();
    assert_eq!(items.len(), 2);
    let launcher = items.iter().find(|f| f.launcher).unwrap();
    assert_eq!(launcher.label, "Desktop test");
    assert_eq!(launcher.icon.as_deref(), Some("folder"));
    assert!(items.iter().any(|f| f.directory));
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn cross_device_cut_removes_only_fully_copied_sources() {
    let dir = temp();
    let desktop = dir.join("Desktop");
    std::fs::create_dir_all(&desktop).unwrap();
    let source = dir.join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("file"), "contents").unwrap();
    // A FIFO is unsupported: a partially copied folder must leave its source intact.
    use std::os::unix::ffi::OsStrExt;
    let fifo = std::ffi::CString::new(source.join("fifo").as_os_str().as_bytes()).unwrap();
    // SAFETY: fifo is a valid, NUL-terminated path for the duration of the call.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    let file = gio::File::for_path(&source);
    let dest = desktop.join("source");
    assert!(import_with_devices(&file, &dest, false, true, |_, _| Ok(false)).is_err());
    assert!(source.join("file").exists());
    assert!(source.join("fifo").exists());
    assert!(!dest.exists());
    std::fs::remove_file(source.join("fifo")).unwrap();
    import_with_devices(&file, &dest, false, false, |_, _| Ok(false)).unwrap();
    assert!(source.join("file").exists(), "a normal cross-device drop stays a copy");
    assert!(import_with_devices(&file, &dest, false, true, |_, _| Ok(false)).is_err());
    assert!(source.join("file").exists(), "a collision must preserve the source");
    std::fs::remove_dir_all(&dest).unwrap();
    let plans = plan_imports(vec![file.uri().to_string()], &desktop, BTreeSet::new(), false, true);
    assert!(plans[0].cut);
    import_with_devices(&file, &plans[0].dest, false, plans[0].cut, |_, _| Ok(false)).unwrap();
    assert!(!source.exists());
    assert_eq!(std::fs::read_to_string(dest.join("file")).unwrap(), "contents");
    let link = dir.join("link");
    std::os::unix::fs::symlink(dest.join("file"), &link).unwrap();
    import_with_devices(&gio::File::for_path(&link), &desktop.join("link"), false, true, |_, _| Ok(false)).unwrap();
    assert!(link.symlink_metadata().is_err());
    assert!(desktop.join("link").symlink_metadata().unwrap().is_symlink());
    assert!(dest.join("file").exists(), "moving a symlink must preserve its target");
    assert!(imports_succeeded(&Ok::<_, ()>(vec![Ok(()), Ok(())])));
    assert!(!imports_succeeded(&Ok::<_, ()>(vec![Ok(()), Err("Failed".into())])));
    assert!(!imports_succeeded(&Err::<Vec<Result<(), String>>, _>(())));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn import_moves_on_same_filesystem_and_refuses_collisions() {
    let dir = temp();
    std::fs::create_dir_all(dir.join("Desktop")).unwrap();
    let source = dir.join("a");
    let dest = dir.join("Desktop/a");
    std::fs::write(&source, b"original").unwrap();
    import(&gio::File::for_path(&source), &dest, false, false).unwrap();
    assert!(!source.exists());
    std::fs::write(&source, b"replacement").unwrap();
    assert!(import(&gio::File::for_path(&source), &dest, false, false).is_err());
    assert_eq!(std::fs::read(&dest).unwrap(), b"original");
    assert_eq!(std::fs::read(&source).unwrap(), b"replacement");
    import(&gio::File::for_path(&dest), &dest, false, false).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn invalid_utf8_filenames_have_distinct_persistence_keys() {
    use std::os::unix::ffi::OsStrExt;
    let a = name_key(std::ffi::OsStr::from_bytes(b"a\xff"));
    let b = name_key(std::ffi::OsStr::from_bytes(b"a\xfe"));
    assert_ne!(a, b);
    assert_ne!(a, name_key(std::ffi::OsStr::new("a�")));
    let positions = [(a, grid().nth(0)), (b, grid().nth(1))]
        .into_iter()
        .collect();
    assert_eq!(decode(&encode(&positions).unwrap()).unwrap(), positions);
}
#[test]
fn desktop_icons_default_on_with_explicit_off_switch() {
    let default: crate::config::Config = toml::from_str("").unwrap();
    assert!(crate::config::Prefs::from(&default).desktop_icons);
    let disabled: crate::config::Config = toml::from_str("desktop_icons = false").unwrap();
    assert!(!crate::config::Prefs::from(&disabled).desktop_icons);
}
#[test]
fn a_name_in_use_gets_the_next_number_and_copies_leave_the_source() {
    let root = std::env::temp_dir().join(format!("digit-desk-names-{}", std::process::id()));
    std::fs::create_dir_all(root.join("test")).unwrap();
    std::fs::create_dir_all(root.join("test (2)")).unwrap();
    let free = collision_name(std::ffi::OsStr::new("test"), true, |c| root.join(c).symlink_metadata().is_ok());
    assert_eq!(free, "test (3)");
    std::fs::write(root.join("a.txt"), "x").unwrap();
    let uri = gio::File::for_path(root.join("a.txt")).uri().to_string();
    let plans = plan_imports(vec![uri], &root, BTreeSet::new(), true, false);
    assert_eq!(plans[0].dest, root.join("a (2).txt"), "a copy onto itself is a duplicate");
    import(&gio::File::for_uri(&plans[0].uri), &plans[0].dest, true, false).unwrap();
    assert!(root.join("a.txt").exists() && root.join("a (2).txt").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn menus_follow_files_order_and_undo_shows_only_when_possible() {
    use MenuAction::*;
    let empty = menu_entries(false, 0, false);
    assert_eq!(&empty[..3], &[NewFolder, Paste, SelectAll]);
    assert!(!empty.contains(&Undo));
    assert!(menu_entries(false, 3, true).contains(&Undo));
    assert_eq!(menu_entries(true, 1, true), vec![Open, Reveal, Separator, Cut, Copy, CopyPath, Separator, Rename, Duplicate, Separator, Trash]);
    for count in [2, 20] {
        assert_eq!(menu_entries(true, count, false), vec![Open, Separator, Cut, Copy, Duplicate, Separator, Trash]);
    }
    for action in menu_entries(true, 1, false).into_iter().chain(menu_entries(false, 0, true)) {
        assert_eq!(action.glyph().is_empty(), action == Separator);
    }
}
#[test]
fn next_folder_name_starts_at_two_and_fills_gaps() {
    assert_eq!(next_folder_name(|_| false).as_deref(), Some("New Folder"));
    let occupied = ["New Folder", "New Folder 2", "New Folder 4"];
    assert_eq!(
        next_folder_name(|n| occupied.contains(&n)).as_deref(),
        Some("New Folder 3")
    );
    assert_eq!(
        next_folder_name(|n| n == "New Folder").as_deref(),
        Some("New Folder 2")
    );
    assert!(next_folder_name(|_| true).is_none());
}
#[test]
fn denied_empty_point_drag_and_duplicate_end_leave_no_rubber_state() {
    let selected = BTreeSet::from(["kept".to_string()]);
    // Empty space, but a non-primary gesture or an active file drag denies the band.
    for (primary, dragging) in [(false, false), (true, true)] {
        let model = RefCell::new(begin_band(
            false,
            primary,
            dragging,
            (10.0, 20.0),
            selected.clone(),
            false,
        ));
        assert!(model.borrow().is_none());
        // Model the synchronous drag-end followed by cancel after set_state(Denied).
        assert!(finish_band(&mut model.borrow_mut(), false).is_none());
        assert!(finish_band(&mut model.borrow_mut(), true).is_none());
        assert!(model.borrow().is_none());
        assert_eq!(selected, BTreeSet::from(["kept".to_string()]));
    }
    let mut band = begin_band(false, true, false, (10.0, 20.0), selected.clone(), true);
    assert!(band.is_some());
    assert_eq!(finish_band(&mut band, true), Some(selected));
    assert!(band.is_none());
    assert!(finish_band(&mut band, false).is_none());
    assert!(begin_band(true, true, false, (10.0, 20.0), BTreeSet::new(), false).is_none());
}
#[test]
fn new_folder_uses_a_free_name() {
    let dir = temp();
    std::fs::create_dir_all(&dir).unwrap();
    assert_eq!(new_folder(&dir).unwrap(), dir.join("New Folder"));
    std::fs::write(dir.join("New Folder 2"), b"also occupied").unwrap();
    std::os::unix::fs::symlink("missing", dir.join("New Folder 3")).unwrap();
    assert_eq!(new_folder(&dir).unwrap(), dir.join("New Folder 4"));
    std::fs::remove_dir_all(dir).unwrap();
}
