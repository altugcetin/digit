//! The open sheet's Phone tab.

use super::*;
use crate::feel;

impl View {
    pub(super) fn phone_rect(&self) -> Rect {
        Rect { x: LEFT, y: self.body_top(), w: PHONE_W, h: 120.0 }
    }

    pub(super) fn phone_switch_rect(&self) -> Rect {
        let p = self.phone_rect();
        Rect { x: p.x + p.w - 42.0, y: p.y + 14.0, w: 30.0, h: 30.0 }
    }

    pub(super) fn phone_action_rect(&self, i: usize) -> Rect {
        let p = self.phone_rect();
        Rect { x: p.x + 16.0 + i as f64 * 42.0, y: p.y + p.h - 46.0, w: 36.0, h: 36.0 }
    }

    pub(super) fn note_rect(&self, i: usize) -> Rect {
        let x = self.phone_section_rect().x + 14.0;
        Rect { x, y: self.body_top() + 14.0 + i as f64 * 38.0, w: RIGHT - 14.0 - x, h: 36.0 }
    }

    pub(super) fn phone_section_rect(&self) -> Rect {
        let x = LEFT + PHONE_W + 14.0;
        Rect { x, y: self.body_top(), w: RIGHT - x, h: 120.0 }
    }

    pub(super) fn draw_phone(&self, s: &gtk::Snapshot, now: i64) {
        let pal = self.pal.get();
        let start = pango::Alignment::Left;
        let center = pango::Alignment::Center;
        let top = self.body_top();
        let Some(phone) = crate::phone::phone() else {
            let (title, sub) = if crate::phone::daemon() {
                ("No phone paired", "Pair your phone in the KDE Connect app to see it here")
            } else {
                ("KDE Connect isn't running", "Install kdeconnect and pair your phone to see it here")
            };
            let mid = top + 56.0;
            self.icon(s, "smartphone", 30.0, true, pal.ink_faint, CX, mid - 22.0);
            self.text(s, title, 15.0, POINT, pal.ink_main, LEFT, mid + 12.0, RIGHT - LEFT, center);
            self.text(s, sub, 12.0, REST, pal.ink_meta, LEFT, mid + 32.0, RIGHT - LEFT, center);
            return;
        };
        let p = self.phone_rect();
        s.push_rounded_clip(&gsk::RoundedRect::from_rect(p.graphene(), self.corner(CARD_RADIUS, p.h)));
        s.append_color(&pal.ink_main.alpha(0.04).rgba(), &p.graphene());
        s.pop();
        let disc = graphene::Rect::new((p.x + 14.0) as f32, (p.y + 14.0) as f32, 40.0, 40.0);
        s.push_rounded_clip(&gsk::RoundedRect::from_rect(disc, 20.0));
        s.append_color(&(if phone.reachable { pal.accent.alpha(0.18) } else { pal.ink_main.alpha(0.08) }).rgba(), &disc);
        s.pop();
        self.icon(s, phone.glyph(), 22.0, true, if phone.reachable { pal.accent } else { pal.ink_meta }, p.x + 34.0, p.y + 34.0);
        let next = crate::phone::next();
        let name_w = p.w - 80.0 - if next.is_some() { 36.0 } else { 0.0 };
        self.text(s, &phone.name, 16.0, Face::Text(620), pal.ink_main, p.x + 66.0, p.y + 25.0, name_w, start);
        if next.is_some() {
            let r = self.phone_switch_rect();
            let glow = self.feel.glow(Hot::PhoneSwitch, now);
            let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
            feel::scaled(s, self.feel.scale(Hot::PhoneSwitch, feel::SMALL, now), cx, cy, || {
                s.push_rounded_clip(&gsk::RoundedRect::from_rect(r.graphene(), self.corner(15.0, r.h)));
                s.append_color(&pal.ink_main.alpha(0.07 + 0.10 * glow).rgba(), &r.graphene());
                s.pop();
                self.icon(s, "swap_horiz", 17.0, false, pal.ink_main.mix(pal.ink_meta, glow), cx, cy);
            });
        }
        let mut status = Vec::new();
        if !phone.reachable {
            status.push("Not reachable".to_string());
        }
        if let Some((charge, charging)) = phone.battery {
            status.push(format!("{charge}%{}", if charging { " · Charging" } else { "" }));
        }
        if let Some((kind, _)) = &phone.signal {
            status.push(kind.clone());
        }
        // The status line gives way to the hovered button's name, then to an action's outcome.
        let hovered = match self.hot {
            Some(Hot::PhoneAction(i)) if phone.reachable => crate::phone::ACTIONS.get(i).map(|a| a.2.to_string()),
            Some(Hot::PhoneSwitch) => next.as_ref().map(|n| format!("Switch to {}", n.name)),
            _ => None,
        };
        let line = hovered.or_else(crate::phone::note).unwrap_or_else(|| if status.is_empty() { "Connected".to_string() } else { status.join(" · ") });
        self.text(s, &line, 12.0, REST, if phone.reachable { pal.ink_meta } else { pal.ink_faint }, p.x + 66.0, p.y + 45.0, p.w - 80.0, start);
        for (i, (_, icon, _)) in crate::phone::ACTIONS.iter().enumerate() {
            let r = self.phone_action_rect(i);
            let key = Hot::PhoneAction(i);
            let fill = if phone.reachable { 0.09 + 0.08 * self.feel.glow(key, now) } else { 0.04 };
            let ink = if phone.reachable { pal.ink_main } else { pal.ink_faint };
            let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
            feel::scaled(s, self.feel.scale(key, feel::SMALL, now), cx, cy, || {
                s.push_rounded_clip(&gsk::RoundedRect::from_rect(r.graphene(), self.corner(18.0, r.h)));
                s.append_color(&pal.ink_main.alpha(fill).rgba(), &r.graphene());
                s.pop();
                self.icon(s, icon, 18.0, true, ink, cx, cy);
            });
        }
        let card = self.phone_section_rect();
        s.push_rounded_clip(&gsk::RoundedRect::from_rect(card.graphene(), self.corner(CARD_RADIUS, card.h)));
        s.append_color(&pal.ink_main.alpha(0.04).rgba(), &card.graphene());
        s.pop();
        let x = card.x + 14.0;
        let w = card.w - 28.0;
        let call = crate::phone::call();
        if phone.notes.is_empty() && call.is_none() {
            let msg = if phone.reachable { "No notifications from your phone" } else { "Notifications show up while the phone is reachable" };
            self.text(s, msg, 13.0, REST, pal.ink_meta, x, top + 25.0, w - 44.0, start);
            let close = Rect { x: card.x + card.w - 44.0, y: card.y + 10.0, w: 30.0, h: 30.0 };
            self.icon(s, "close", 16.0, false, pal.ink_faint, close.x + close.w / 2.0, close.y + close.h / 2.0);
            return;
        }
        if let Some(c) = &call {
            let r = self.note_rect(0);
            let mid = r.y + r.h / 2.0;
            self.icon(s, "call", 18.0, true, GREEN, r.x + 16.0, mid);
            self.text(s, c, 13.0, Face::Text(560), pal.ink_main, r.x + 36.0, mid, r.w - 44.0, start);
        }
        for (row, i) in phone_rows(call.is_some(), phone.notes.len()) {
            let n = &phone.notes[i];
            let r = self.note_rect(row);
            let mid = r.y + r.h / 2.0;
            match &n.icon {
                Some(t) => self.draw_paintable(s, t.upcast_ref(), r.x + 4.0, mid - 12.0, 24.0, 6.0),
                None => self.icon(s, "notifications", 18.0, false, pal.ink_meta, r.x + 16.0, mid),
            }
            let head = if n.title.is_empty() { n.app.clone() } else if n.app.is_empty() || n.app == n.title { n.title.clone() } else { format!("{} · {}", n.app, n.title) };
            let room = r.w - 44.0 - if n.dismissable { 30.0 } else { 0.0 };
            self.text(s, &head, 13.0, Face::Text(560), pal.ink_main, r.x + 36.0, mid - 8.0, room, start);
            self.text(s, &n.text, 12.0, REST, pal.ink_meta, r.x + 36.0, mid + 9.0, room, start);
            if n.dismissable {
                let cx = r.x + r.w - 14.0;
                let glow = self.feel.glow(Hot::PhoneDismiss(i), now);
                feel::scaled(s, self.feel.scale(Hot::PhoneDismiss(i), feel::SMALL, now), cx, mid, || {
                    if glow > 0.01 {
                        let d = graphene::Rect::new((cx - 11.0) as f32, (mid - 11.0) as f32, 22.0, 22.0);
                        s.push_rounded_clip(&gsk::RoundedRect::from_rect(d, 11.0));
                        s.append_color(&pal.ink_main.alpha(0.16 * glow).rgba(), &d);
                        s.pop();
                    }
                    self.icon(s, "close", 14.0, false, pal.ink_main.mix(pal.ink_faint, glow), cx, mid);
                });
            }
        }
        let more = phone.notes.len() + call.is_some() as usize;
        if more > NOTES {
            self.text(s, &format!("+{} more", more - NOTES), 11.0, REST, pal.ink_faint, x, top + 4.0 + NOTES as f64 * 38.0 + 6.0, w, pango::Alignment::Right);
        }
    }
}
