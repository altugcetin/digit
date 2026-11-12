//! The open sheet's Tray tab: dropped files, sharing and devices.

use super::*;

impl View {
    pub(super) fn files_rect(&self) -> Rect {
        Rect { x: LEFT, y: self.body_top(), w: FILES_W, h: 120.0 }
    }

    pub(super) fn share_rect(&self) -> Rect {
        Rect { x: LEFT + FILES_W + 14.0, y: self.body_top(), w: RIGHT - LEFT - FILES_W - 14.0, h: 120.0 }
    }

    pub(super) fn tile_rect(&self, i: usize) -> Rect {
        let f = self.files_rect();
        Rect { x: f.x + 14.0 + i as f64 * TILE, y: f.y + 12.0, w: TILE - 6.0, h: 96.0 }
    }

    pub(super) fn remove_rect(&self, i: usize) -> Rect {
        let t = self.tile_rect(i);
        Rect { x: t.x + t.w - 22.0, y: t.y + 2.0, w: 20.0, h: 20.0 }
    }

    pub(super) fn device_rect(&self, i: usize) -> Rect {
        let z = self.share_rect();
        Rect { x: z.x + 8.0, y: z.y + 38.0 + i as f64 * 26.0, w: z.w - 16.0, h: 24.0 }
    }

    pub(super) fn chip_rect(&self, i: usize) -> Rect {
        let z = self.share_rect();
        Rect { x: z.x + 12.0 + i as f64 * (CHIP + 6.0), y: z.y + z.h - CHIP - 8.0, w: CHIP, h: CHIP }
    }

    pub(super) fn draw_tray(&self, s: &gtk::Snapshot, now: i64) {
        let pal = self.pal.get();
        let start = pango::Alignment::Left;
        let center = pango::Alignment::Center;
        let f = self.files_rect();
        let over_files = self.landing == Some(Landing::Files);
        s.push_rounded_clip(&gsk::RoundedRect::from_rect(f.graphene(), self.corner(CARD_RADIUS, f.h)));
        s.append_color(&pal.ink_main.alpha(if over_files { 0.1 } else { 0.04 }).rgba(), &f.graphene());
        s.pop();
        let outline = gsk::PathBuilder::new();
        outline.add_rounded_rect(&gsk::RoundedRect::from_rect(graphene::Rect::new((f.x + 1.0) as f32, (f.y + 1.0) as f32, (f.w - 2.0) as f32, (f.h - 2.0) as f32), crate::metrics::nested(self.corner(CARD_RADIUS, f.h) as f64, 1.0) as f32));
        let stroke = gsk::Stroke::new(1.5);
        stroke.set_dash(&[7.0, 6.0]);
        s.append_stroke(&outline.to_path(), &stroke, &(if over_files { pal.accent } else { pal.ink_meta.alpha(0.45) }).rgba());
        let count = crate::tray::len();
        if count == 0 {
            self.icon(s, "inbox", 28.0, true, pal.accent, f.x + 38.0, f.y + 44.0);
            self.text(s, "Files Tray", 15.0, POINT, pal.accent, f.x + 24.0, f.y + 78.0, 200.0, start);
            self.text(s, "Drop files here to keep them close", 12.0, REST, pal.ink_faint, f.x + 24.0, f.y + 98.0, f.w - 48.0, start);
        } else {
            let glow = self.glow[9].value(now).clamp(0.0, 1.0);
            crate::tray::each(|i, item| {
                if i >= TILES {
                    return;
                }
                let t = self.tile_rect(i);
                let hot = matches!(self.hot, Some(Hot::Tile(j) | Hot::Remove(j)) if j == i);
                if hot {
                    s.push_rounded_clip(&gsk::RoundedRect::from_rect(t.graphene(), self.corner(14.0, t.h)));
                    s.append_color(&pal.ink_main.alpha(0.08 * glow).rgba(), &t.graphene());
                    s.pop();
                }
                match &item.icon {
                    Some(p) => self.draw_paintable(s, p, t.x + (t.w - 48.0) / 2.0, t.y + 10.0, 48.0, 10.0),
                    None => self.icon(s, "draft", 40.0, false, pal.ink_meta, t.x + t.w / 2.0, t.y + 34.0),
                }
                self.text(s, &item.name, 11.0, REST, pal.ink_main, t.x + 4.0, t.y + 76.0, t.w - 8.0, center);
                if hot {
                    let x = self.remove_rect(i);
                    let on = self.hot == Some(Hot::Remove(i));
                    s.push_rounded_clip(&gsk::RoundedRect::from_rect(x.graphene(), 10.0));
                    s.append_color(&(if on { pal.risk } else { pal.ink_main.alpha(0.85) }).rgba(), &x.graphene());
                    s.pop();
                    self.icon(s, "close", 13.0, false, pal.body.alpha(1.0), x.x + x.w / 2.0, x.y + x.h / 2.0);
                }
            });
            if count > TILES {
                let t = self.tile_rect(TILES);
                self.text(s, &format!("+{}", count - TILES), 13.0, Face::Number(500), pal.ink_meta, t.x - 8.0, t.y + 34.0, 40.0, start);
            }
        }
        let z = self.share_rect();
        let over_share = matches!(self.landing, Some(Landing::Share | Landing::Device(_) | Landing::Sender(_)));
        s.push_rounded_clip(&gsk::RoundedRect::from_rect(z.graphene(), self.corner(CARD_RADIUS, z.h)));
        s.append_color(&pal.accent.alpha(if over_share { 0.26 } else { 0.15 }).rgba(), &z.graphene());
        self.icon(s, "share", 18.0, true, pal.accent, z.x + 24.0, z.y + 21.0);
        self.text(s, "Share", 15.0, POINT, pal.ink_main, z.x + 40.0, z.y + 21.0, 80.0, start);
        let senders = crate::share::senders();
        let named = match (self.hot, self.landing) {
            (Some(Hot::Sender(i)), _) | (_, Some(Landing::Sender(i))) => senders.get(i).map(|s| format!("Send with {}", s.name)),
            _ => None,
        };
        let status = crate::share::status().map(|status| match status {
            crate::share::Status::Asking(n) => (format!("Waiting for {n}…"), pal.ink_meta),
            crate::share::Status::Sending(f) => (format!("Sending {f}…"), pal.ink_meta),
            crate::share::Status::Sent(n) => (format!("Sent to {n}"), GREEN),
            crate::share::Status::Failed(e) => (e, pal.risk),
        });
        let (label, ink) = match (named, status) {
            (Some(n), _) => (n, pal.ink_main),
            (None, Some(st)) => st,
            (None, None) => ("LocalSend · KDE Connect".to_string(), pal.ink_faint),
        };
        self.text(s, &label, 11.0, REST, ink, z.x + 100.0, z.y + 21.0, z.w - 116.0, pango::Alignment::Right);
        let devices = crate::share::devices();
        if devices.is_empty() {
            let msg = if crate::share::scanning() { "Looking for devices…" } else { "No devices nearby" };
            self.text(s, msg, 13.0, REST, pal.ink_meta, z.x, z.y + 54.0, z.w, center);
        }
        for (i, sender) in senders.iter().take(CHIPS).enumerate() {
            let r = self.chip_rect(i);
            let on = self.hot == Some(Hot::Sender(i)) || self.landing == Some(Landing::Sender(i));
            s.push_rounded_clip(&gsk::RoundedRect::from_rect(r.graphene(), 9.0));
            s.append_color(&pal.ink_main.alpha(if on { 0.2 } else { 0.08 }).rgba(), &r.graphene());
            s.pop();
            match &sender.icon {
                Some(p) => self.draw_paintable(s, p, r.x + 4.0, r.y + 4.0, r.w - 8.0, 4.0),
                None => self.icon(s, "language", 17.0, false, pal.ink_main, r.x + r.w / 2.0, r.y + r.h / 2.0),
            }
        }
        let glow = self.glow[9].value(now).clamp(0.0, 1.0);
        for (i, d) in devices.iter().take(DEVICES).enumerate() {
            let r = self.device_rect(i);
            let fill = if self.landing == Some(Landing::Device(i)) { 0.3 } else if self.hot == Some(Hot::Device(i)) { 0.12 * glow } else { 0.0 };
            if fill > 0.005 {
                s.push_rounded_clip(&gsk::RoundedRect::from_rect(r.graphene(), 10.0));
                s.append_color(&pal.accent.alpha(fill).rgba(), &r.graphene());
                s.pop();
            }
            let mid = r.y + r.h / 2.0;
            self.icon(s, d.icon(), 16.0, false, pal.ink_main, r.x + 16.0, mid);
            self.text(s, &d.name, 13.0, REST, pal.ink_main, r.x + 30.0, mid, r.w - 120.0, start);
            self.text(s, d.service(), 11.0, REST, pal.ink_faint, r.x + r.w - 92.0, mid, 84.0, pango::Alignment::Right);
        }
        s.pop();
    }
}
