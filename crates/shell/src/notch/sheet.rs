//! The open sheet's Nook tab: media player or call card, calendar and tools.

use super::*;

impl View {
    pub(super) fn day_rects(&self) -> Vec<(Rect, crate::calendar::Day)> {
        let x0 = CAL_X + 64.0;
        let col = (CAL_W - 64.0) / 7.0;
        let top = self.body_top() + 6.0;
        crate::calendar::week(0).into_iter().enumerate().map(|(i, d)| (Rect { x: x0 + i as f64 * col, y: top, w: col, h: 46.0 }, d)).collect()
    }

    pub(super) fn tool_rect(&self, i: usize) -> Rect {
        let x0 = TOOLS_X + ((RIGHT - TOOLS_X) - 96.0) / 2.0;
        let y0 = self.body_top() + 12.0;
        Rect { x: x0 + (i % 2) as f64 * 52.0, y: y0 + (i / 2) as f64 * 52.0, w: 44.0, h: 44.0 }
    }

    /// Whether the open notch shows the call card rather than the player.
    pub(super) fn showing_call(&self, now: i64) -> bool {
        if self.app.call.borrow().is_none() {
            self.media_over_call.set(false);
            return false;
        }
        !(self.media_over_call.get() && self.app.media.borrow().present(now))
    }

    /// A call and a player are both around, so the media spot can flip between them.
    pub(super) fn can_switch(&self, now: i64) -> bool {
        self.app.call.borrow().is_some() && self.app.media.borrow().present(now)
    }

    pub(super) fn switch_rect(&self) -> Rect {
        Rect { x: LEFT + MEDIA_W - 34.0, y: self.media_top() + 6.0, w: 34.0, h: 34.0 }
    }

    pub(super) fn draw_switch(&self, s: &gtk::Snapshot, now: i64) {
        if !self.can_switch(now) {
            return;
        }
        let pal = self.pal.get();
        let r = self.switch_rect();
        let glow = self.glow[10].value(now).clamp(0.0, 1.0);
        s.push_rounded_clip(&gsk::RoundedRect::from_rect(r.graphene(), (r.h / 2.0) as f32));
        s.append_color(&pal.ink_main.alpha(0.09 + 0.07 * glow).rgba(), &r.graphene());
        s.pop();
        let icon = if self.showing_call(now) { "music_note" } else { "call" };
        self.icon(s, icon, 18.0, true, pal.ink_main, r.x + r.w / 2.0, r.y + r.h / 2.0);
    }

    pub(super) fn call_buttons(&self) -> (Rect, Rect) {
        let y = self.media_top() + 76.0;
        (Rect { x: LEFT + 82.0, y, w: 104.0, h: 34.0 }, Rect { x: LEFT + 194.0, y, w: 104.0, h: 34.0 })
    }

    pub(super) fn media_top(&self) -> f64 {
        self.body_top() + 2.0
    }

    pub(super) fn draw_call(&self, s: &gtk::Snapshot, now: i64) {
        let pal = self.pal.get();
        let start = pango::Alignment::Left;
        let Some((who, since)) = self.app.call.borrow().as_ref().map(|c| (c.who.clone(), c.since)) else { return };
        let top = self.media_top();
        let icon_rect = Rect { x: LEFT, y: top + 4.0, w: 68.0, h: 68.0 };
        let gicon = {
            let list = self.app.apps.list();
            list.iter().find(|e| e.name.eq_ignore_ascii_case(&who)).and_then(|e| e.icon.clone())
        };
        let scale = self.window.scale_factor().max(1);
        let paint: Option<gdk::Paintable> = gicon.and_then(|g| {
            let theme = gtk::IconTheme::for_display(&WidgetExt::display(&self.window));
            Some(theme.lookup_by_gicon(&g, 68, scale, gtk::TextDirection::Ltr, gtk::IconLookupFlags::empty()).upcast())
        });
        match paint {
            Some(p) => self.draw_paintable(s, &p, icon_rect.x, icon_rect.y, 68.0, 16.0),
            None => self.draw_monogram(s, &who, icon_rect.x, icon_rect.y + 34.0, 68.0),
        }
        let dot = graphene::Rect::new((LEFT + 82.0) as f32, (top + 12.0) as f32, 8.0, 8.0);
        let pulse = if motion::reduced() {
            1.0
        } else {
            0.55 + 0.45 * ((now as f64 / 1_000_000.0) * std::f64::consts::PI).sin().abs()
        };
        s.push_rounded_clip(&gsk::RoundedRect::from_rect(dot, 4.0));
        s.append_color(&GREEN.alpha(pulse).rgba(), &dot);
        s.pop();
        let t = ((now - since).max(0) / 1_000_000) as u64;
        let clock = if t >= 3600 { format!("{}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60) } else { format!("{}:{:02}", t / 60, t % 60) };
        let text_w = MEDIA_W - 82.0 - if self.can_switch(now) { 42.0 } else { 0.0 };
        let place = self.app.call_place.borrow().clone();
        match place {
            Some((channel, server)) => {
                self.text(s, &format!("On a call · {who}"), 13.0, REST, GREEN, LEFT + 96.0, top + 16.0, text_w - 14.0, start);
                self.text(s, &channel, 17.0, Face::Text(620), pal.ink_main, LEFT + 82.0, top + 38.0, text_w, start);
                let sub = if server.is_empty() { clock.clone() } else { format!("{server} · {clock}") };
                self.text(s, &sub, 14.0, Face::Text(480), pal.ink_meta, LEFT + 82.0, top + 60.0, text_w, start);
            }
            None => {
                self.text(s, "On a call", 13.0, REST, GREEN, LEFT + 96.0, top + 16.0, text_w - 14.0, start);
                self.text(s, &who, 17.0, Face::Text(620), pal.ink_main, LEFT + 82.0, top + 38.0, text_w, start);
                self.text(s, &clock, 15.0, Face::Number(560), pal.ink_meta, LEFT + 82.0, top + 60.0, text_w, start);
            }
        }
        let muted = self.app.mic_muted.get().unwrap_or(false);
        let (mute, open) = self.call_buttons();
        for (i, (r, icon, label, tint)) in [(mute, if muted { "mic_off" } else { "mic" }, if muted { "Unmute" } else { "Mute" }, if muted { RED } else { pal.ink_main }), (open, "open_in_new", "Show", pal.ink_main)].into_iter().enumerate() {
            let glow = self.glow[i].value(now).clamp(0.0, 1.0);
            let fill = if i == 0 && muted { RED.alpha(0.18 + 0.08 * glow) } else { pal.ink_main.alpha(0.09 + 0.07 * glow) };
            s.push_rounded_clip(&gsk::RoundedRect::from_rect(r.graphene(), self.corner(17.0, r.h)));
            s.append_color(&fill.rgba(), &r.graphene());
            s.pop();
            self.icon(s, icon, 17.0, true, tint, r.x + 22.0, r.y + r.h / 2.0);
            self.text(s, label, 13.0, POINT, tint, r.x + 36.0, r.y + r.h / 2.0, r.w - 42.0, start);
        }
    }

    pub(super) fn player(&self, now: i64) -> Option<Player> {
        if self.showing_call(now) || !self.app.media.borrow().present(now) {
            return None;
        }
        let top = self.media_top();
        let art = Rect { x: LEFT, y: top + 4.0, w: 104.0, h: 104.0 };
        let info_x = LEFT + 122.0;
        let info_w = MEDIA_W - 122.0 - if self.can_switch(now) { 42.0 } else { 0.0 };
        let ccx = info_x + 52.0;
        let button = |cx: f64| Rect { x: cx - 18.0, y: top + 96.0 - 18.0, w: 36.0, h: 36.0 };
        Some(Player {
            art,
            info_x,
            info_w,
            top,
            seek: Rect { x: info_x, y: top + 54.0, w: info_w, h: 16.0 },
            controls: [(button(ccx - 44.0), Action::Previous), (button(ccx), Action::Toggle), (button(ccx + 44.0), Action::Next)],
        })
    }

    pub(super) fn draw_sheet(&self, s: &gtk::Snapshot, now: i64) {
        let fade = self.tab_fade.value(now).clamp(0.0, 1.0);
        if fade < 0.01 {
            return;
        }
        s.push_opacity(fade);
        match self.tab {
            Tab::Nook => self.draw_nook(s, now),
            Tab::Tray => self.draw_tray(s, now),
            Tab::Phone => self.draw_phone(s, now),
        }
        s.pop();
    }

    pub(super) fn divider(&self, s: &gtk::Snapshot, x: f64) {
        let top = self.body_top();
        let line = graphene::Rect::new(x as f32, top as f32, 1.0, 120.0);
        s.append_color(&self.pal.get().ink_main.alpha(0.08).rgba(), &line);
    }

    pub(super) fn draw_nook(&self, s: &gtk::Snapshot, now: i64) {
        self.draw_media(s, now);
        self.divider(s, LEFT + MEDIA_W + 14.0);
        self.draw_calendar(s, now);
        self.divider(s, CAL_X + CAL_W + 14.0);
        self.draw_tools(s, now);
    }

    pub(super) fn draw_media(&self, s: &gtk::Snapshot, now: i64) {
        self.draw_switch(s, now);
        s.push_opacity(self.flip.value(now).clamp(0.0, 1.0));
        self.draw_media_body(s, now);
        s.pop();
    }

    pub(super) fn draw_media_body(&self, s: &gtk::Snapshot, now: i64) {
        if self.showing_call(now) {
            self.draw_call(s, now);
            return;
        }
        let pal = self.pal.get();
        let start = pango::Alignment::Left;
        let Some(p) = self.player(now) else {
            let mid = self.media_top() + 56.0;
            self.icon(s, "music_note", 28.0, true, pal.ink_faint, LEFT + MEDIA_W / 2.0, mid - 12.0);
            self.text(s, "Nothing playing", 14.0, REST, pal.ink_meta, LEFT, mid + 18.0, MEDIA_W, pango::Alignment::Center);
            return;
        };
        let media = self.app.media.borrow().clone();
        self.draw_art(s, &p.art, 20.0, self.art_scale.value(now));
        if let Some(badge) = self.badge_icon() {
            let b = 26.0;
            let (bx, by) = (p.art.x + p.art.w - b + 5.0, p.art.y + p.art.h - b + 5.0);
            let ring = graphene::Rect::new((bx - 3.0) as f32, (by - 3.0) as f32, (b + 6.0) as f32, (b + 6.0) as f32);
            s.push_rounded_clip(&gsk::RoundedRect::from_rect(ring, 10.0));
            s.append_color(&pal.body.alpha(1.0).rgba(), &ring);
            s.pop();
            self.draw_paintable(s, &badge, bx, by, b, 7.0);
        }
        self.text(s, &media.title, 16.0, Face::Text(600), pal.ink_main, p.info_x, p.top + 13.0, p.info_w, start);
        self.text(s, &media.artist, 14.0, REST, pal.ink_meta, p.info_x, p.top + 34.0, p.info_w, start);
        let bar_y = p.seek.y + p.seek.h / 2.0;
        let small = |t: f64| {
            let t = t.max(0.0) as u64;
            if t >= 3600 { format!("{}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60) } else { format!("{}:{:02}", t / 60, t % 60) }
        };
        let face = Face::Number(450);
        let half = p.info_w / 2.0;
        match media.timing {
            Timing::Timed => {
                let elapsed = self.scrub.map_or_else(|| media.elapsed(now), |f| f * media.length);
                let progress = (elapsed / media.length).clamp(0.0, 1.0) * self.fill.value(now).clamp(0.0, 1.0);
                let lift = if media.can_seek { self.glow[3].value(now).clamp(0.0, 1.0) } else { 0.0 };
                let thick = 4.0 + 2.0 * lift;
                let track = Rect { x: p.info_x, y: bar_y - thick / 2.0, w: p.info_w, h: thick };
                self.bar(s, &track, progress, pal.ink_main.alpha(0.15 + 0.05 * lift), pal.ink_main);
                if media.can_seek && (lift > 0.01 || self.scrub.is_some()) {
                    let r = if self.scrub.is_some() { 6.5 } else { 3.0 + 3.0 * lift };
                    let knob = graphene::Rect::new((track.x + track.w * progress - r) as f32, (bar_y - r) as f32, (2.0 * r) as f32, (2.0 * r) as f32);
                    s.push_rounded_clip(&gsk::RoundedRect::from_rect(knob, r as f32));
                    s.append_color(&pal.ink_main.rgba(), &knob);
                    s.pop();
                }
                self.text(s, &small(elapsed), 11.0, face, pal.ink_faint, p.info_x, bar_y + 14.0, half, start);
                self.text(s, &small(media.length), 11.0, face, pal.ink_faint, p.info_x + half, bar_y + 14.0, half, pango::Alignment::Right);
            }
            Timing::Live => {
                let pill = Rect { x: p.info_x, y: bar_y - 9.0, w: 48.0, h: 18.0 };
                s.push_rounded_clip(&gsk::RoundedRect::from_rect(pill.graphene(), 9.0));
                s.append_color(&pal.risk.rgba(), &pill.graphene());
                s.pop();
                self.text(s, "LIVE", 11.0, Face::Text(700), Rgb::new(1.0, 1.0, 1.0), pill.x, pill.y + pill.h / 2.0, pill.w, pango::Alignment::Center);
                self.text(s, &media.source, 12.0, REST, pal.ink_meta, pill.x + pill.w + 10.0, bar_y, p.info_w - pill.w - 10.0, start);
            }
            Timing::Untimed => {
                let track = Rect { x: p.info_x, y: bar_y - 2.0, w: p.info_w, h: 4.0 };
                self.bar(s, &track, 0.0, pal.ink_main.alpha(0.15), pal.ink_main);
                if media.counted {
                    self.text(s, &small(media.elapsed(now)), 11.0, face, pal.ink_faint, p.info_x, bar_y + 14.0, half, start);
                }
            }
        }
        for (i, (r, action)) in p.controls.iter().enumerate() {
            let (name, px, enabled) = match action {
                Action::Previous => ("skip_previous", 22.0, media.can_prev),
                Action::Toggle => (if media.playing { "pause" } else { "play_arrow" }, 28.0, true),
                Action::Next => ("skip_next", 22.0, media.can_next),
            };
            let glow = if enabled { self.glow[i].value(now).clamp(0.0, 1.0) } else { 0.0 };
            let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
            if glow > 0.01 {
                let rad = r.w / 2.0 + 3.0 * glow;
                let halo = graphene::Rect::new((cx - rad) as f32, (cy - rad) as f32, (2.0 * rad) as f32, (2.0 * rad) as f32);
                s.push_rounded_clip(&gsk::RoundedRect::from_rect(halo, rad as f32));
                s.append_color(&pal.ink_main.alpha(0.1 * glow).rgba(), &halo);
                s.pop();
            }
            let ink = if enabled { pal.ink_main } else { pal.ink_main.alpha(0.28) };
            self.icon(s, name, px, true, ink, cx, cy);
        }
    }

    pub(super) fn draw_calendar(&self, s: &gtk::Snapshot, now: i64) {
        let pal = self.pal.get();
        let top = self.body_top() + 6.0;
        let days = self.day_rects();
        if days.len() < 7 {
            return;
        }
        let sel = (3 + self.day).clamp(0, 6) as usize;
        let center = pango::Alignment::Center;
        self.text(s, &crate::calendar::month(&days[sel].1), 24.0, Face::Text(650), pal.ink_main, CAL_X, top + 23.0, 64.0, pango::Alignment::Left);
        let glow = self.glow[9].value(now).clamp(0.0, 1.0);
        for (i, (r, d)) in days.iter().enumerate() {
            let hot = self.hot == Some(Hot::Day(i));
            let back = if i == sel && !d.today { 0.11 } else { 0.0 } + if hot && !d.today { 0.06 * glow } else { 0.0 };
            if back > 0.005 {
                let pill = graphene::Rect::new((r.x + 1.0) as f32, (r.y - 2.0) as f32, (r.w - 2.0) as f32, (r.h + 4.0) as f32);
                s.push_rounded_clip(&gsk::RoundedRect::from_rect(pill, 10.0));
                s.append_color(&pal.ink_main.alpha(back).rgba(), &pill);
                s.pop();
            }
            let weekend = d.weekday >= 6;
            let letter = if d.today { crate::calendar::label(d) } else { d.letter.clone() };
            let quiet = if weekend { pal.risk.mix(pal.ink_meta, 0.55) } else { pal.ink_meta };
            self.text(s, &letter, 11.5, Face::Text(650), if d.today { pal.accent } else { quiet }, r.x - 8.0, r.y + 9.0, r.w + 16.0, center);
            let (px, face) = if d.today { (21.0, Face::Number(650)) } else { (15.0, Face::Number(500)) };
            let ink = if d.today { pal.accent } else if weekend { pal.risk.mix(pal.ink_meta, 0.5) } else { pal.ink_meta };
            self.text(s, &format!("{:02}", d.number), px, face, ink, r.x - 6.0, r.y + 31.0, r.w + 12.0, center);
        }
        let day = &days[sel].1;
        let events = crate::calendar::on(day);
        let ey = top + 66.0;
        if events.is_empty() {
            let msg = if day.today { "Nothing for today".to_string() } else { format!("Nothing on {}", crate::calendar::name(day)) };
            let tw = self.text_width(&msg, 13.0, REST);
            let x = CAL_X + (CAL_W - tw - 22.0) / 2.0;
            self.icon(s, "event_available", 16.0, false, pal.ink_faint, x + 8.0, ey + 16.0);
            self.text(s, &msg, 13.0, REST, pal.ink_meta, x + 22.0, ey + 16.0, tw + 2.0, pango::Alignment::Left);
            return;
        }
        for (k, e) in events.iter().take(2).enumerate() {
            let y = ey + 6.0 + k as f64 * 22.0;
            let mark = graphene::Rect::new(CAL_X as f32, (y - 8.0) as f32, 3.0, 16.0);
            s.push_rounded_clip(&gsk::RoundedRect::from_rect(mark, 1.5));
            s.append_color(&pal.accent.rgba(), &mark);
            s.pop();
            let when = if e.all_day { "All day".to_string() } else { crate::calendar::clock(e.start) };
            self.text(s, &when, 12.0, Face::Number(500), pal.ink_meta, CAL_X + 10.0, y, 52.0, pango::Alignment::Left);
            self.text(s, &e.title, 13.0, REST, pal.ink_main, CAL_X + 64.0, y, CAL_W - 64.0, pango::Alignment::Left);
        }
        if events.len() > 2 {
            self.text(s, &format!("+{} more", events.len() - 2), 11.0, REST, pal.ink_faint, CAL_X + 10.0, ey + 50.0, CAL_W - 10.0, pango::Alignment::Left);
        }
    }

    pub(super) fn draw_tools(&self, s: &gtk::Snapshot, now: i64) {
        let pal = self.pal.get();
        let glow = self.glow[9].value(now).clamp(0.0, 1.0);
        let awake = self.app.awake.borrow().as_ref().is_some_and(|a| a.on());
        let mut label = None;
        for (i, (icon, name)) in TOOLS.iter().enumerate() {
            let r = self.tool_rect(i);
            let on = match i {
                2 => self.app.dnd.get(),
                3 => awake,
                _ => false,
            };
            let hot = self.hot == Some(Hot::Tool(i));
            if hot {
                label = Some(*name);
            }
            let fill = if on { pal.accent } else { pal.ink_main.alpha(0.1 + if hot { 0.08 * glow } else { 0.0 }) };
            s.push_rounded_clip(&gsk::RoundedRect::from_rect(r.graphene(), self.corner(15.0, r.h)));
            s.append_color(&fill.rgba(), &r.graphene());
            s.pop();
            let ink = if on { pal.on_accent } else { pal.ink_main };
            self.icon(s, icon, 20.0, on, ink, r.x + r.w / 2.0, r.y + r.h / 2.0);
        }
        if let Some(name) = label {
            let r = self.tool_rect(2);
            self.text(s, name, 11.0, REST, pal.ink_meta.alpha(glow), TOOLS_X - 6.0, r.y + r.h + 13.0, RIGHT - TOOLS_X + 12.0, pango::Alignment::Center);
        }
    }
}
