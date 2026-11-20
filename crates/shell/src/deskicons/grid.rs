//! Where icons sit: the cells of a screen's grid, filling them, and rubber-band selection.

use super::*;

pub(super) const CELL_W: i32 = 112;
pub(super) const CELL_H: i32 = 116;
pub(super) const PAD: i32 = 24;
pub(super) const ICON: i32 = 64;
pub(super) const LABEL_INSET: i32 = 6;
pub(super) type Positions = BTreeMap<String, FreePos>;

/// Logical pixels of the icon cell's top-left on the desktop surface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct FreePos {
    pub(super) x: f64,
    pub(super) y: f64,
    #[serde(default)]
    pub(super) page: u32,
}
impl FreePos {
    pub(super) fn rect(self) -> Rect {
        Rect {
            x: self.x,
            y: self.y,
            w: CELL_W as f64,
            h: CELL_H as f64,
        }
    }
    pub(super) fn overlaps(self, other: Self) -> bool {
        self.page == other.page && self.rect().intersects(other.rect())
    }
}

#[derive(Clone, Copy)]
pub(super) struct Grid {
    pub(super) cols: u32,
    pub(super) rows: u32,
    pub(super) width: i32,
    pub(super) height: i32,
    pub(super) top: i32,
    /// Where desktop widgets sit (the clock, the batteries and the corner widgets), which
    /// icons keep out of.
    /// Where the desktop widgets on this screen sit: the clock, the batteries and the four cards.
    pub(super) reserved: [Option<Rect>; 6],
}
impl Grid {
    pub(super) fn new(width: i32, height: i32, top: i32) -> Self {
        let top = top.max(PAD).min((height - CELL_H).max(0));
        Self {
            cols: ((width - 2 * PAD) / CELL_W).max(1) as u32,
            rows: ((height - top - 88) / CELL_H).max(1) as u32,
            width,
            height,
            top,
            reserved: [None; 6],
        }
    }
    pub(super) fn clamp(self, p: FreePos) -> FreePos {
        FreePos {
            x: p.x.clamp(0.0, (self.width - CELL_W).max(0) as f64),
            y: p.y
                .clamp(self.top as f64, (self.height - CELL_H).max(self.top) as f64),
            page: p.page.min(u32::MAX - 1),
        }
    }
    // The grid is only an automatic-placement policy; manual positions never snap.
    pub(super) fn nth(self, n: usize) -> FreePos {
        let n = n as u32;
        self.clamp(FreePos {
            x: (self.width - PAD - CELL_W - ((n / self.rows) % self.cols) as i32 * CELL_W) as f64,
            y: (self.top + (n % self.rows) as i32 * CELL_H) as f64,
            page: n / (self.rows * self.cols),
        })
    }
    pub(super) fn reserved(self, p: FreePos) -> bool {
        self.reserved.iter().flatten().any(|r| r.intersects(p.rect()))
    }
    pub(super) fn automatic_cells(self) -> Vec<FreePos> {
        (0..(self.cols * self.rows) as usize)
            .map(|n| self.nth(n))
            .filter(|p| !self.reserved(*p))
            .collect()
    }
}

/// Restore free positions, including overlaps, before assigning automatic cells.
/// Persisting the result makes later additions independent of the current name order.
///
/// Widgets win over icons: an icon saved where a widget now sits (one dropped there, or a
/// widget turned on over it) moves just outside that widget, to whichever side is nearest,
/// or to the nearest free cell when no side has room.
pub(super) fn place(names: &[String], saved: &Positions, grid: Grid) -> Positions {
    let mut cells = grid.automatic_cells();
    // On very small displays there may be no room beside the card. Keep icons reachable.
    if cells.is_empty() {
        cells.extend((0..(grid.cols * grid.rows) as usize).map(|n| grid.nth(n)));
    }
    let mut out = Positions::new();
    let mut displaced = Vec::new();
    for name in names {
        if let Some(&p) = saved.get(name)
            && p.x.is_finite()
            && p.y.is_finite()
        {
            let p = grid.clamp(p);
            if grid.reserved(p) {
                displaced.push((name.clone(), p));
            } else {
                out.insert(name.clone(), p);
            }
        }
    }
    for (name, was) in displaced {
        let far = |c: &FreePos| (c.x - was.x).powi(2) + (c.y - was.y).powi(2);
        let free = |c: &FreePos| !grid.reserved(*c) && !out.values().any(|other| c.overlaps(*other));
        // Beside each widget it overlaps, keeping the other coordinate, with a little air.
        let (cw, ch, air) = (CELL_W as f64, CELL_H as f64, 4.0);
        let beside: Vec<FreePos> = grid
            .reserved
            .iter()
            .flatten()
            .filter(|r| r.intersects(was.rect()))
            .flat_map(|r| {
                [
                    FreePos { x: r.x - cw - air, ..was },
                    FreePos { x: r.x + r.w + air, ..was },
                    FreePos { y: r.y - ch - air, ..was },
                    FreePos { y: r.y + r.h + air, ..was },
                ]
            })
            .map(|c| grid.clamp(c))
            .collect();
        let nearest = beside
            .into_iter()
            .filter(free)
            .min_by(|a, b| far(a).total_cmp(&far(b)))
            .or_else(|| {
                cells
                    .iter()
                    .map(|c| FreePos { page: was.page, ..*c })
                    .filter(free)
                    .min_by(|a, b| far(a).total_cmp(&far(b)))
            });
        // A full page leaves it to the automatic placement below.
        if let Some(p) = nearest {
            out.insert(name, p);
        }
    }
    let mut next = 0;
    for name in names {
        if !out.contains_key(name) {
            // Rejected cells stay blocked as icons are added. Scan each candidate once.
            // With at most out.len() occupied pages, one additional page is always free.
            let (n, p) = (next..(out.len() + 1) * cells.len())
                .map(|n| {
                    (
                        n,
                        FreePos {
                            page: (n / cells.len()) as u32,
                            ..cells[n % cells.len()]
                        },
                    )
                })
                .find(|(_, p)| !out.values().any(|other| p.overlaps(*other)))
                .expect("one more page than occupied icons");
            next = n + 1;
            out.insert(name.clone(), p);
        }
    }
    out
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Rect {
    pub(super) x: f64,
    pub(super) y: f64,
    pub(super) w: f64,
    pub(super) h: f64,
}
impl Rect {
    pub(super) fn between(a: (f64, f64), b: (f64, f64)) -> Self {
        Self {
            x: a.0.min(b.0),
            y: a.1.min(b.1),
            w: (a.0 - b.0).abs(),
            h: (a.1 - b.1).abs(),
        }
    }
    pub(super) fn intersects(self, b: Self) -> bool {
        self.x < b.x + b.w && self.x + self.w > b.x && self.y < b.y + b.h && self.y + self.h > b.y
    }
    pub(super) fn contains(self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}
pub(super) fn rubber_select(
    rect: Rect,
    items: &[(String, Rect)],
    initial: &BTreeSet<String>,
    additive: bool,
) -> BTreeSet<String> {
    let mut selected = if additive {
        initial.clone()
    } else {
        BTreeSet::new()
    };
    selected.extend(
        items
            .iter()
            .filter(|(_, r)| rect.intersects(*r))
            .map(|(n, _)| n.clone()),
    );
    selected
}
