//! The island's head: clock, privacy lights, volume, notifications and the call/media peeks.

use super::*;

impl View {
    pub(super) fn draw_clock(&self, s: &gtk::Snapshot, now: i64) {
        let pal = self.pal.get();
        let hush = self.hush.value(now).clamp(0.0, 1.0);
        if hush < 0.01 {
            return;
        }
        let mid = self.header() / 2.0;
        // While switching workspaces the pill takes the clock's place. The two overlap rather
        // than swap: the clock sinks back and fades out while the pill grows from the middle and
        // fades in, so neither cuts in or out.
        let open = self.pill.openness(now);
        let ease = |x: f64| { let x = x.clamp(0.0, 1.0); x * x * (3.0 - 2.0 * x) };
        let full = self.pill.width_at(now);
        if open > 0.002 && full > 0.0 {
            let pw = (full * (0.35 + 0.65 * ease(open))).max(1.0);
            let x = CX - pw / 2.0;
            let lift = (4.0 * (1.0 - ease(open))).round();
            s.save();
            s.translate(&graphene::Point::new(0.0, lift as f32));
            s.push_rounded_clip(&gsk::RoundedRect::from_rect(graphene::Rect::new(x as f32, (mid - 14.0) as f32, pw as f32, 28.0), 14.0_f64.min(pw / 2.0) as f32));
            s.push_opacity(ease((open - 0.2) / 0.8) * hush);
            // Drawn at full width and centred, so the dots stay put while the clip opens.
            self.pill.draw(s, (CX - full / 2.0).round(), mid, now, pal.ink_main, pal.accent);
            s.pop();
            s.pop();
            s.restore();
        }
        let hush = hush * (1.0 - ease(open / 0.7));
        if hush < 0.01 {
            return;
        }
        // The clock shrinks a little and drops back as it gives way.
        let give = ease(open / 0.7);
        let k = 1.0 - 0.12 * give;
        s.save();
        s.translate(&graphene::Point::new(CX as f32, (mid - 3.0 * give) as f32));
        s.scale(k as f32, k as f32);
        s.translate(&graphene::Point::new(-CX as f32, -mid as f32));
        let color = pal.ink_main.mix(pal.ink_meta, 1.0 - self.back.value(now).clamp(0.0, 1.0)).alpha(hush);
        let face = Face::Number(500);
        let cells: Vec<(pango::Layout, f64)> = self
            .shown
            .iter()
            .map(|c| {
                let l = self.layout(&c.to_string(), 16.0, face);
                let w = l.pixel_extents().1.width() as f64;
                (l, w)
            })
            .collect();
        let total: f64 = cells.iter().map(|c| c.1).sum();
        let mut x = (CX - total / 2.0).round();
        let line = cells.first().map(|c| c.0.pixel_extents().1.height() as f64).unwrap_or(20.0);
        let y = (mid - line / 2.0).round();
        for (i, (l, w)) in cells.iter().enumerate() {
            let e = match self.roll.get(i).copied().flatten() {
                Some(t) if !motion::reduced() && now < t + ROLL => motion::IN_OUT_CUBIC.at(((now - t).max(0)) as f64 / ROLL as f64),
                _ => 1.0,
            };
            if e < 1.0 {
                s.push_clip(&graphene::Rect::new(x as f32, y as f32, *w as f32, line as f32));
                if let Some(old) = self.was.get(i) {
                    let ol = self.layout(&old.to_string(), 16.0, face);
                    s.save();
                    s.translate(&graphene::Point::new(x as f32, (y - line * e) as f32));
                    s.append_layout(&ol, &color.rgba());
                    s.restore();
                }
                s.save();
                s.translate(&graphene::Point::new(x as f32, (y + line * (1.0 - e)) as f32));
                s.append_layout(l, &color.rgba());
                s.restore();
                s.pop();
            } else {
                s.save();
                s.translate(&graphene::Point::new(x as f32, y as f32));
                s.append_layout(l, &color.rgba());
                s.restore();
            }
            x += w;
        }
        s.restore();
    }

    pub(super) fn wings(&self, head: &Head, now: i64) -> (f64, f64, f64, f64) {
        let (w, _, _) = self.geometry(head, now);
        let half = self.clock_width() / 2.0;
        (CX - w / 2.0 + 12.0, CX - half - 10.0, CX + half + 10.0, CX + w / 2.0 - 12.0 - self.room())
    }

    pub(super) fn draw_lights(&self, s: &gtk::Snapshot, now: i64) {
        let (screen, capture) = (self.dots[0].value(now).clamp(0.0, 1.0), self.dots[1].value(now).clamp(0.0, 1.0));
        if screen < 0.01 && capture < 0.01 {
            return;
        }
        let mid = self.header() / 2.0;
        // The outermost light sits as far from the right edge as from the top and bottom.
        let edge = CX + self.w.value(now) / 2.0 - mid;
        let mut x = edge;
        for (alpha, color) in [(capture, self.capture.get()), (screen, RED)] {
            if alpha < 0.01 {
                continue;
            }
            for (r, a) in [(6.0 * (0.6 + 0.4 * alpha), 0.22 * alpha), (3.5 * (0.5 + 0.5 * alpha), alpha)] {
                let dot = graphene::Rect::new((x - r) as f32, (mid - r) as f32, (2.0 * r) as f32, (2.0 * r) as f32);
                s.push_rounded_clip(&gsk::RoundedRect::from_rect(dot, r as f32));
                s.append_color(&color.alpha(a).rgba(), &dot);
                s.pop();
            }
            x -= 12.0 * alpha;
        }
    }

    pub(super) fn vol_right(&self, now: i64) -> f64 {
        self.wings(&Head::Open, now).3 - 38.0
    }

    pub(super) fn volume_zone(&self, now: i64) -> Option<Rect> {
        let (level, muted) = self.app.volume.get()?;
        let (_, _, r0, _) = self.wings(&Head::Open, now);
        let r1 = self.vol_right(now);
        let h = self.header();
        if self.tuning {
            return Some(Rect { x: r0 - 4.0, y: 0.0, w: r1 - r0 + 10.0, h });
        }
        let text = if muted { String::new() } else { format!("{}%", (level * 100.0).round()) };
        let tw = self.text_width(&text, 15.0, Face::Number(450));
        let x = r1 - tw - 30.0;
        Some(Rect { x, y: 4.0, w: r1 + 8.0 - x, h: h - 8.0 })
    }

    pub(super) fn tuner(&self, now: i64) -> Option<Tuner> {
        if !self.open {
            return None;
        }
        let (_, _, r0, _) = self.wings(&Head::Open, now);
        let r1 = self.vol_right(now);
        let mid = self.header() / 2.0;
        let label = self.text_width("100%", 15.0, Face::Number(560)).ceil() + 2.0;
        let x = r0 + 28.0;
        Some(Tuner {
            icon: Rect { x: r0 - 2.0, y: mid - 13.0, w: 26.0, h: 26.0 },
            track: Rect { x, y: mid - 2.0, w: (r1 - label - 10.0 - x).max(20.0), h: 4.0 },
            label,
        })
    }

    pub(super) fn draw_volume(&self, s: &gtk::Snapshot, now: i64) {
        let Some((level, muted)) = self.app.volume.get() else { return };
        let pal = self.pal.get();
        let mid = self.header() / 2.0;
        let r1 = self.vol_right(now);
        let t = self.tune.value(now).clamp(0.0, 1.0);
        let glow = self.glow[4].value(now).clamp(0.0, 1.0);
        let icon = if muted { "volume_off" } else if level < 0.5 { "volume_down" } else { "volume_up" };
        if t < 0.99 {
            s.push_opacity(1.0 - t);
            if let Some(z) = self.volume_zone(now).filter(|_| !self.tuning) {
                if glow > 0.01 {
                    let pill = graphene::Rect::new(z.x as f32, (mid - 13.0) as f32, z.w as f32, 26.0);
                    s.push_rounded_clip(&gsk::RoundedRect::from_rect(pill, 13.0));
                    s.append_color(&pal.ink_main.alpha(0.09 * glow).rgba(), &pill);
                    s.pop();
                }
            }
            let text = if muted { String::new() } else { format!("{}%", (level * 100.0).round()) };
            let tw = self.text_width(&text, 15.0, Face::Number(450));
            let ink = pal.ink_meta.mix(pal.ink_main, 1.0 - glow);
            self.text(s, &text, 15.0, Face::Number(450), ink, r1 - tw - 2.0, mid, tw + 2.0, pango::Alignment::Right);
            self.icon(s, icon, 18.0, false, ink, r1 - tw - 14.0, mid);
            s.pop();
        }
        let Some(tuner) = self.tuner(now) else { return };
        if t > 0.01 {
            s.push_opacity(t);
            let ink = if muted { pal.ink_meta } else { pal.ink_main };
            self.icon(s, icon, 18.0, false, ink, tuner.icon.x + tuner.icon.w / 2.0, mid);
            let end = tuner.track.x + tuner.track.w;
            let w = tuner.track.w * (0.3 + 0.7 * t);
            let track = Rect { x: end - w, w, ..tuner.track };
            let value = level.clamp(0.0, 1.0);
            self.bar(s, &track, value, pal.ink_main.alpha(0.15), ink);
            let r = if self.sliding { 6.5 } else { 5.0 + glow };
            let knob = graphene::Rect::new((track.x + track.w * value - r) as f32, (mid - r) as f32, (2.0 * r) as f32, (2.0 * r) as f32);
            s.push_rounded_clip(&gsk::RoundedRect::from_rect(knob, r as f32));
            s.append_color(&ink.rgba(), &knob);
            s.pop();
            let percent = format!("{}%", (value * 100.0).round());
            self.text(s, &percent, 15.0, Face::Number(560), ink, r1 - tuner.label, mid, tuner.label, pango::Alignment::Right);
            s.pop();
        }
    }

    pub(super) fn draw_head(&self, s: &gtk::Snapshot, now: i64) {
        let pal = self.pal.get();
        let mid = self.header() / 2.0;
        let (l0, l1, r0, r1) = self.wings(&self.head, now);
        let (left, right) = (l1 - l0, r1 - r0);
        let start = pango::Alignment::Left;
        let end = pango::Alignment::Right;
        match &self.head {
            Head::Idle(media) => {
                if *media {
                    let (rect, r) = self.cover(l0, mid);
                    self.draw_art(s, &rect, r, 1.0);
                }
            }
            Head::Call => {
                let tx = self.badge(s, l0, mid, 22.0, GREEN, "call", 14.0);
                self.text(s, &self.call_time(now), 15.0, Face::Number(560), GREEN, tx, mid, (l0 + left - tx).max(0.0), start);
            }
            Head::Open => {
                let _ = (l0, left, r0, right);
                for (i, (r, icon, label)) in self.tab_rects().iter().enumerate() {
                    let on = TABS[i].0 == self.tab;
                    let glow = self.glow[Hot::Tab(i).index()].value(now).clamp(0.0, 1.0);
                    let fill = if on { 0.14 } else { 0.0 } + 0.07 * glow;
                    if fill > 0.005 {
                        s.push_rounded_clip(&gsk::RoundedRect::from_rect(r.graphene(), 13.0));
                        s.append_color(&pal.ink_main.alpha(fill).rgba(), &r.graphene());
                        s.pop();
                    }
                    let ink = if on { pal.ink_main } else { pal.ink_meta.mix(pal.ink_main, 1.0 - glow) };
                    self.icon(s, icon, 17.0, on, ink, r.x + 21.0, mid);
                    self.text(s, label, 14.0, POINT, ink, r.x + 34.0, mid, r.w - 38.0, start);
                }
                let g = self.gear_rect(now);
                let glow = self.glow[7].value(now).clamp(0.0, 1.0);
                if glow > 0.01 {
                    s.push_rounded_clip(&gsk::RoundedRect::from_rect(g.graphene(), 13.0));
                    s.append_color(&pal.ink_main.alpha(0.1 * glow).rgba(), &g.graphene());
                    s.pop();
                }
                self.icon(s, "settings", 18.0, true, pal.ink_meta.mix(pal.ink_main, 1.0 - glow), g.x + g.w / 2.0, mid);
                self.draw_volume(s, now);
            }
            Head::Peek(Peek::Volume { level, muted }) => {
                let icon = if *muted { "volume_off" } else if *level < 0.5 { "volume_down" } else { "volume_up" };
                self.icon(s, icon, 18.0, false, pal.ink_main, self.lead(l0, 18.0) + 9.0, mid);
                self.text(s, if *muted { "Muted" } else { "Volume" }, 15.0, REST, pal.ink_meta, l0 + 24.0, mid, left - 24.0, start);
                let value = self.meter.value(now).clamp(0.0, 1.0);
                let percent = format!("{}%", (value * 100.0).round());
                // Size the label for the widest value so "100%" never ellipsizes and the meter
                // does not change length while the number animates.
                let label = self.text_width("100%", 15.0, Face::Number(560)).ceil() + 2.0;
                self.text(s, &percent, 15.0, Face::Number(560), pal.ink_main, r1 - label, mid, label, end);
                let track = Rect { x: r0, y: mid - 1.5, w: (right - label - 8.0).max(0.0), h: 3.0 };
                self.bar(s, &track, value, pal.ink_main.alpha(0.15), pal.ink_main);
            }
            Head::Peek(Peek::Track { title, artist }) => {
                let (rect, r) = self.cover(l0, mid);
                self.draw_art(s, &rect, r, 1.0);
                let text = rect.x + rect.w + 9.0;
                self.text(s, title, 15.0, POINT, pal.ink_main, text, mid, (l0 + left - text).max(0.0), start);
                self.text(s, artist, 15.0, REST, pal.ink_meta, r0, mid, right, end);
            }
            Head::Peek(peek @ Peek::Notification { summary, body, app, icon, avatar, actions, .. }) => {
                let (top, height) = match self.app.dock() {
                    Dock::Screen => (0.0, peek.height()),
                    Dock::Bar => (self.header(), peek.height()),
                };
                let content = if actions.is_empty() { height } else { height - 60.0 };
                self.draw_note(s, top + content / 2.0, summary, body, app, icon.as_ref(), avatar.as_ref());
                for (i, (r, label)) in self.note_buttons(peek).into_iter().enumerate() {
                    let last = i + 1 == actions.len();
                    let glow = if self.hot == Some(Hot::Action(i)) { 1.0 } else { 0.0 };
                    let fill = if last { ACTION_BLUE.alpha(0.9 + 0.1 * glow) } else { pal.ink_main.alpha(0.14 + 0.08 * glow) };
                    s.push_rounded_clip(&gsk::RoundedRect::from_rect(r.graphene(), (r.h / 2.0) as f32));
                    s.append_color(&fill.rgba(), &r.graphene());
                    s.pop();
                    self.text(s, &label, 14.0, POINT, Rgb::new(1.0, 1.0, 1.0), r.x, r.y + r.h / 2.0, r.w, pango::Alignment::Center);
                }
            }
            Head::Peek(Peek::Osd { icon, text, level }) => {
                self.icon(s, icon, 18.0, false, pal.ink_main, self.lead(l0, 18.0) + 9.0, mid);
                match level {
                    Some(_) => {
                        self.text(s, text, 15.0, REST, pal.ink_meta, l0 + 24.0, mid, left - 24.0, start);
                        let value = self.meter.value(now).clamp(0.0, 1.0);
                        let percent = format!("{}%", (value * 100.0).round());
                        let label = self.text_width("100%", 15.0, Face::Number(560)).ceil() + 2.0;
                        self.text(s, &percent, 15.0, Face::Number(560), pal.ink_main, r1 - label, mid, label, end);
                        let track = Rect { x: r0, y: mid - 1.5, w: (right - label - 8.0).max(0.0), h: 3.0 };
                        self.bar(s, &track, value, pal.ink_main.alpha(0.15), pal.ink_main);
                    }
                    None => self.text(s, text, 15.0, POINT, pal.ink_main, r0 - 60.0, mid, right + 60.0, end),
                }
            }
            Head::Peek(Peek::Mode { icon, label, on }) => {
                let (glyph, tint) = match *label {
                    "Do not disturb" => ("bedtime", PURPLE),
                    "Night Light" => ("nightlight", ORANGE),
                    "Keep awake" => ("coffee", ORANGE),
                    _ => (*icon, pal.accent),
                };
                let tint = if *on { tint } else { pal.ink_meta };
                self.icon(s, glyph, 20.0, true, tint, self.lead(l0, 20.0) + 10.0, mid);
                self.text(s, if *on { "On" } else { "Off" }, 15.0, Face::Text(620), tint, r0, mid, right, end);
            }
            Head::Peek(Peek::Battery { level, device }) => {
                match device {
                    Some((name, glyph)) => {
                        self.icon(s, glyph, 20.0, true, pal.ink_main, self.lead(l0, 20.0) + 10.0, mid);
                        self.text(s, name, 14.0, REST, pal.ink_meta, l0 + 28.0, mid, left - 28.0, start);
                    }
                    None => self.text(s, "Low Battery", 15.0, Face::Text(620), pal.ink_main, l0, mid, left, start),
                }
                let bw = 30.0;
                let percent = format!("{level}%");
                let pw = self.text_width(&percent, 15.0, Face::Number(600)).ceil();
                let bx = r1 - bw - 4.0;
                self.text(s, &percent, 15.0, Face::Number(600), RED, bx - pw - 8.0, mid, pw + 2.0, start);
                let shell = Rect { x: bx, y: mid - 7.0, w: bw, h: 14.0 };
                s.push_rounded_clip(&gsk::RoundedRect::from_rect(shell.graphene(), 4.0));
                s.append_color(&RED.alpha(0.35).rgba(), &shell.graphene());
                let fill = Rect { x: bx, y: mid - 7.0, w: bw * (*level as f64 / 100.0).clamp(0.08, 1.0), h: 14.0 };
                s.append_color(&RED.rgba(), &fill.graphene());
                s.pop();
                let nub = Rect { x: bx + bw + 1.5, y: mid - 3.0, w: 2.5, h: 6.0 };
                s.push_rounded_clip(&gsk::RoundedRect::from_rect(nub.graphene(), 1.2));
                s.append_color(&RED.alpha(0.6).rgba(), &nub.graphene());
                s.pop();
            }
            Head::Peek(Peek::Device { name, icon, battery, on }) => {
                self.icon(s, icon, 20.0, true, pal.ink_main, self.lead(l0, 20.0) + 10.0, mid);
                self.text(s, name, 14.0, REST, pal.ink_meta, l0 + 28.0, mid, left - 28.0, start);
                match (battery, on) {
                    (Some(b), true) => {
                        let ring = if *b <= 20 { RED } else { GREEN };
                        let cx = r1 - 11.0;
                        let path = |frac: f64| {
                            let pb = gsk::PathBuilder::new();
                            let r = 9.0f32;
                            let start = -std::f64::consts::FRAC_PI_2;
                            let steps = (48.0 * frac).ceil().max(1.0) as usize;
                            for k in 0..=steps {
                                let a = start + frac * std::f64::consts::TAU * k as f64 / steps as f64;
                                let (x, y) = ((cx + r as f64 * a.cos()) as f32, (mid + r as f64 * a.sin()) as f32);
                                if k == 0 { pb.move_to(x, y) } else { pb.line_to(x, y) }
                            }
                            pb.to_path()
                        };
                        let stroke = gsk::Stroke::new(3.0);
                        stroke.set_line_cap(gsk::LineCap::Round);
                        s.append_stroke(&path(1.0), &stroke, &ring.alpha(0.25).rgba());
                        s.append_stroke(&path((*b as f64 / 100.0).clamp(0.02, 1.0)), &stroke, &ring.rgba());
                    }
                    (_, true) => self.text(s, "Connected", 14.0, POINT, GREEN, r0, mid, right, end),
                    (_, false) => self.text(s, "Disconnected", 14.0, REST, pal.ink_faint, r0, mid, right, end),
                }
            }
            Head::Peek(Peek::Output { name, icon }) => {
                self.icon(s, icon, 20.0, true, ACTION_BLUE, self.lead(l0, 20.0) + 10.0, mid);
                self.text(s, name, 14.0, POINT, pal.ink_main, r0 - 60.0, mid, right + 60.0, end);
            }
            Head::Peek(Peek::Privacy { light, who }) => {
                let color = match light {
                    crate::privacy::Light::Camera => GREEN,
                    crate::privacy::Light::Mic => ORANGE,
                    crate::privacy::Light::Screen | crate::privacy::Light::Record => RED,
                };
                let tx = self.badge(s, l0, mid, 22.0, color, light.icon(), 14.0);
                self.text(s, light.label(), 15.0, REST, pal.ink_main, tx, mid, (l0 + left - tx).max(0.0), start);
                self.text(s, who, 15.0, POINT, color.mix(pal.ink_main, 0.55), r0, mid, right, end);
            }
            Head::Peek(Peek::Note { text }) => {
                self.text(s, text, 15.0, POINT, pal.ink_main, l0, mid, left, start);
            }
        }
    }

    /// A tinted disc with an icon at the island's left edge, as far from it as from the top and
    /// bottom and rounded to match the island's corner (calls, recording, camera and mic).
    /// Returns where the text beside it starts.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn badge(&self, s: &gtk::Snapshot, l0: f64, mid: f64, size: f64, color: Rgb, icon: &str, px: f64) -> f64 {
        let x = self.lead(l0, size);
        let r = (self.rounded(self.idle_r(), 0.0) - (x - (l0 - 12.0))).clamp(4.0, size / 2.0);
        let disc = graphene::Rect::new(x as f32, (mid - size / 2.0) as f32, size as f32, size as f32);
        s.push_rounded_clip(&gsk::RoundedRect::from_rect(disc, r as f32));
        s.append_color(&color.alpha(0.2).rgba(), &disc);
        s.pop();
        self.icon(s, icon, px, true, color, x + size / 2.0, mid);
        x + size + 10.0
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_note(&self, s: &gtk::Snapshot, mid: f64, summary: &str, body: &str, app: &str, logo: Option<&gdk::Paintable>, face: Option<&gdk::Paintable>) {
        let pal = self.pal.get();
        let half = self.w.target() / 2.0;
        let left = CX - half + 18.0;
        let right = CX + half - 22.0 - self.room();
        let rich = !body.is_empty();
        let size = if rich { 46.0 } else { 36.0 };
        let (ax, ay) = (left, mid - size / 2.0);
        match (face, logo) {
            (Some(f), _) => self.draw_paintable(s, f, ax, ay, size, size / 2.0),
            (None, Some(l)) => self.draw_paintable(s, l, ax, ay, size, size * 0.26),
            (None, None) => self.draw_monogram(s, app, ax, mid, size),
        }
        if let (Some(_), Some(l)) = (face, logo) {
            let b = if rich { 20.0 } else { 16.0 };
            let (bx, by) = (ax + size - b + 4.0, ay + size - b + 4.0);
            let ring = graphene::Rect::new((bx - 2.0) as f32, (by - 2.0) as f32, (b + 4.0) as f32, (b + 4.0) as f32);
            s.push_rounded_clip(&gsk::RoundedRect::from_rect(ring, ((b + 4.0) * 0.3) as f32));
            s.append_color(&pal.body.alpha(1.0).rgba(), &ring);
            s.pop();
            self.draw_paintable(s, l, bx, by, b, b * 0.26);
        }
        let start = pango::Alignment::Left;
        let tx = ax + size + 14.0;
        let stamp = self.text_width("now", 14.0, REST);
        let line = if rich { mid - 11.0 } else { mid };
        self.text(s, summary, 16.0, Face::Text(600), pal.ink_main, tx, line, (right - stamp - 12.0 - tx).max(0.0), start);
        self.text(s, "now", 14.0, REST, pal.ink_faint, right - stamp, line, stamp + 2.0, start);
        if rich {
            self.text(s, body, 15.0, REST, pal.ink_meta, tx, mid + 11.0, (right - tx).max(0.0), start);
        }
    }

    /// Fallback logo for apps that ship no icon: their initial on a quiet rounded square.
    pub(super) fn draw_monogram(&self, s: &gtk::Snapshot, app: &str, x: f64, mid: f64, size: f64) {
        let pal = self.pal.get();
        let rect = graphene::Rect::new(x as f32, (mid - size / 2.0) as f32, size as f32, size as f32);
        s.push_rounded_clip(&gsk::RoundedRect::from_rect(rect, (size * 0.24) as f32));
        s.append_color(&pal.ink_meta.alpha(0.22).rgba(), &rect);
        s.pop();
        match app.chars().find(|c| c.is_alphanumeric()) {
            Some(c) => {
                let letter: String = c.to_uppercase().collect();
                self.text(s, &letter, (size * 0.5).round(), Face::Text(650), pal.ink_main, x, mid, size, pango::Alignment::Center);
            }
            None => self.icon(s, "notifications", (size * 0.7).round(), false, pal.ink_main, x + size / 2.0, mid),
        }
    }

    /// Draws a square, slightly rounded paintable; non-square images are centre-cropped.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_paintable(&self, s: &gtk::Snapshot, p: &gdk::Paintable, x: f64, y: f64, size: f64, radius: f64) {
        let (iw, ih) = (p.intrinsic_width().max(1) as f64, p.intrinsic_height().max(1) as f64);
        let k = (size / iw).max(size / ih);
        let (w, h) = if p.intrinsic_width() > 0 && p.intrinsic_height() > 0 { (iw * k, ih * k) } else { (size, size) };
        let clip = graphene::Rect::new(x as f32, y as f32, size as f32, size as f32);
        s.push_rounded_clip(&gsk::RoundedRect::from_rect(clip, radius as f32));
        s.save();
        s.translate(&graphene::Point::new((x + (size - w) / 2.0) as f32, (y + (size - h) / 2.0) as f32));
        p.snapshot(s, w, h);
        s.restore();
        s.pop();
    }

    pub(super) fn bar(&self, s: &gtk::Snapshot, r: &Rect, value: f64, track: Rgb, fill: Rgb) {
        if r.w <= 0.0 {
            return;
        }
        let radius = (r.h / 2.0) as f32;
        s.push_rounded_clip(&gsk::RoundedRect::from_rect(r.graphene(), radius));
        s.append_color(&track.rgba(), &r.graphene());
        let filled = Rect { w: r.w * value.clamp(0.0, 1.0), ..*r };
        s.append_color(&fill.rgba(), &filled.graphene());
        s.pop();
    }

    /// Where the cover sits on the island's left: as far from the left edge as from the top and
    /// bottom, with its corner inset from the island's own so the two stay concentric at any
    /// roundness. `l0` is the left wing's start (12 px in from the edge).
    pub(super) fn cover(&self, l0: f64, mid: f64) -> (Rect, f64) {
        const COVER: f64 = 20.0;
        let inset = (self.header() - COVER) / 2.0;
        let outer = self.rounded(self.idle_r(), 0.0);
        let r = (outer - inset).clamp(1.5, COVER / 2.0);
        (Rect { x: l0 - 12.0 + inset, y: mid - COVER / 2.0, w: COVER, h: COVER }, r)
    }

    pub(super) fn draw_art(&self, s: &gtk::Snapshot, r: &Rect, radius: f64, scale: f64) {
        let pal = self.pal.get();
        let (w, h) = (r.w * scale, r.h * scale);
        let rect = graphene::Rect::new((r.x + (r.w - w) / 2.0) as f32, (r.y + (r.h - h) / 2.0) as f32, w as f32, h as f32);
        s.push_rounded_clip(&gsk::RoundedRect::from_rect(rect, (radius * scale) as f32));
        s.append_color(&pal.raised.rgba(), &rect);
        let art = self.app.media.borrow().art.clone();
        match art {
            Some(texture) => {
                let (tw, th) = (texture.width() as f64, texture.height() as f64);
                let k = (w / tw).max(h / th);
                let (dw, dh) = (tw * k, th * k);
                let bounds = graphene::Rect::new(
                    (rect.x() as f64 + (w - dw) / 2.0) as f32,
                    (rect.y() as f64 + (h - dh) / 2.0) as f32,
                    dw as f32,
                    dh as f32,
                );
                s.append_scaled_texture(&texture, gsk::ScalingFilter::Trilinear, &bounds);
            }
            None => {
                let px = if r.w > 40.0 { 36.0 } else { 14.0 };
                self.icon(s, "music_note", px * scale, true, pal.ink_meta, r.x + r.w / 2.0, r.y + r.h / 2.0);
            }
        }
        s.pop();
    }
}
