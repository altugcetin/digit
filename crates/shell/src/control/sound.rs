//! Control Center's sound details, under the volume slider: the output and input devices to
//! choose between and a mixer with each app that is playing.

use super::{Control, Hit, State, fill, rounded};
use crate::audio::{self, Channel, Device};
use crate::draw::{self, Face};
use gtk::{glib, graphene, pango, prelude::*};
use kit::motion::{Curve, Tween};
use std::rc::Rc;
use std::time::Duration;

const OPEN: Curve = Curve::new(0.16, 1.0, 0.3, 1.0);
const SHUT: Curve = Curve::new(0.4, 0.0, 0.9, 0.6);
/// Space above the card, between it and the volume slider.
const TOP: f64 = 4.0;
const HEAD: f64 = 30.0;
const DEVICE: f64 = 40.0;
const APP: f64 = 54.0;
const NOTE: f64 = 36.0;
/// Most apps the mixer lists before it would crowd the panel.
const APPS: usize = 6;

pub(super) struct Sound {
    pub(super) open: bool,
    pub(super) reveal: Tween,
    outs: Vec<Device>,
    ins: Vec<Device>,
    apps: Vec<Channel>,
    /// Each app's name and icon, looked up once per refresh rather than every frame.
    faces: Vec<(String, Option<gtk::gdk::Paintable>)>,
    /// An app slider being dragged, with the level it shows.
    sliding: Option<(usize, f64)>,
    /// The app sliders' left edge and width.
    track: (f64, f64),
    /// A timer keeps the details fresh while they are open; only one at a time.
    watching: bool,
    /// Bumped on each refresh, so a slow answer can't overwrite a newer one.
    asked: u32,
}

impl Default for Sound {
    fn default() -> Self {
        Self { open: false, reveal: Tween::still(0.0), outs: Vec::new(), ins: Vec::new(), apps: Vec::new(), faces: Vec::new(), sliding: None, track: (0.0, 0.0), watching: false, asked: 0 }
    }
}

impl State {
    pub(super) fn sound_height(&self) -> f64 {
        let s = &self.sound;
        let apps = if s.apps.is_empty() { NOTE } else { s.apps.len().min(APPS) as f64 * APP };
        let ins = if s.ins.is_empty() { 0.0 } else { HEAD + s.ins.len() as f64 * DEVICE };
        TOP + 8.0 + HEAD + s.outs.len().max(1) as f64 * DEVICE + ins + HEAD + apps + 8.0
    }

    /// The chevron at the end of the volume row that shows the sound details.
    pub(super) fn draw_sound_more(&self, s: &gtk::Snapshot, cx: f64, cy: f64, shift: f64, hits: &mut Vec<(Hit, f64, f64, f64, f64)>) {
        let pal = self.app.palette_at(glib::monotonic_time());
        if self.hover == Some(Hit::SoundMore) {
            fill(s, &rounded(cx - 13.0, cy - 13.0, 26.0, 26.0, 13.0), pal.ink_main.alpha(0.10));
        }
        let open = self.sound.open;
        draw::icon(s, &self.canvas, &self.app.cfg, if open { "expand_less" } else { "expand_more" }, 18.0, false, pal.ink_meta, cx, cy);
        hits.push((Hit::SoundMore, cx - 15.0, cy - 15.0 + shift, 30.0, 30.0));
    }

    /// Draws the details card at `y` as far as it is revealed; returns the height it takes.
    pub(super) fn draw_sound(&mut self, s: &gtk::Snapshot, now: i64, x0: f64, y: f64, inner: f64, shift: f64, hits: &mut Vec<(Hit, f64, f64, f64, f64)>) -> f64 {
        let reveal = self.sound.reveal.value(now).clamp(0.0, 1.0);
        self.draw_card(s, now, x0, y, inner, shift, reveal, hits)
    }

    /// Lays the card out once off screen. The first time its text and glyphs are shaped takes
    /// longer than a frame; done as Control Center opens, the card then opens without a hitch.
    pub(super) fn warm_sound(&mut self) {
        let snap = gtk::Snapshot::new();
        let mut hits = Vec::new();
        self.draw_card(&snap, glib::monotonic_time(), 0.0, 0.0, super::W - 2.0 * super::PAD, 0.0, 1.0, &mut hits);
        let _ = snap.to_node();
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_card(&mut self, s: &gtk::Snapshot, now: i64, x0: f64, y: f64, inner: f64, shift: f64, reveal: f64, hits: &mut Vec<(Hit, f64, f64, f64, f64)>) -> f64 {
        let full = self.sound_height();
        let vis = full * reveal;
        if vis < 0.5 {
            return 0.0;
        }
        let pal = self.app.palette_at(now);
        let cfg = self.app.cfg.clone();
        let canvas = self.canvas.clone();
        s.push_clip(&graphene::Rect::new((x0 - 8.0) as f32, y as f32, (inner + 16.0) as f32, vis as f32));
        s.push_opacity(reveal.powf(0.6));
        let top = y + TOP;
        fill(s, &rounded(x0, top, inner, full - TOP, 20.0 * self.app.round()), pal.ink_main.mix(pal.body, 0.045));
        let (cx, cw) = (x0 + 8.0, inner - 16.0);
        let left = pango::Alignment::Left;
        let mut local: Vec<(Hit, f64, f64, f64, f64)> = Vec::new();
        let mut iy = top + 8.0;
        let head = |s: &gtk::Snapshot, text: &str, iy: f64| {
            draw::text(s, &canvas, &cfg, text, 12.5, Face::Text(600), pal.ink_meta, cx + 8.0, iy + HEAD / 2.0 + 2.0, cw - 16.0, left);
        };
        for (input, list) in [(false, self.sound.outs.clone()), (true, self.sound.ins.clone())] {
            if input && list.is_empty() {
                continue;
            }
            head(s, if input { "Input" } else { "Output" }, iy);
            iy += HEAD;
            if list.is_empty() {
                draw::text(s, &canvas, &cfg, if audio::live() { "Looking for devices…" } else { "Sound isn't available" }, 13.0, Face::Text(450), pal.ink_faint, cx + 8.0, iy + DEVICE / 2.0, cw - 16.0, left);
                iy += DEVICE;
            }
            for (i, d) in list.iter().enumerate() {
                let hit = if input { Hit::In(i) } else { Hit::Out(i) };
                if self.hover == Some(hit) {
                    fill(s, &rounded(cx, iy + 2.0, cw, DEVICE - 4.0, 12.0), pal.ink_main.alpha(0.07));
                }
                let tint = if d.default { pal.accent } else { pal.ink_main };
                draw::icon(s, &canvas, &cfg, d.glyph, 19.0, d.default, tint, cx + 20.0, iy + DEVICE / 2.0);
                draw::text(s, &canvas, &cfg, &d.desc, 13.5, Face::Text(if d.default { 560 } else { 450 }), pal.ink_main, cx + 40.0, iy + DEVICE / 2.0, cw - 76.0, left);
                if d.default {
                    draw::icon(s, &canvas, &cfg, "check", 18.0, false, pal.accent, cx + cw - 18.0, iy + DEVICE / 2.0);
                }
                local.push((hit, cx, iy, cw, DEVICE));
                iy += DEVICE;
            }
        }
        head(s, "Apps", iy);
        iy += HEAD;
        if self.sound.apps.is_empty() {
            draw::text(s, &canvas, &cfg, "No apps are playing", 13.0, Face::Text(450), pal.ink_faint, cx + 8.0, iy + NOTE / 2.0, cw - 16.0, left);
        }
        let (sx, sw) = (cx + 44.0, cw - 44.0 - 44.0);
        self.sound.track = (sx, sw);
        for (i, ch) in self.sound.apps.iter().take(APPS).enumerate() {
            let (name, icon) = self.sound.faces.get(i).cloned().unwrap_or_default();
            let mid = iy + APP / 2.0;
            match icon {
                Some(p) => draw::paintable(s, &p, cx + 6.0, mid - 14.0, 28.0),
                None => draw::monogram(s, &canvas, &cfg, &name, cx + 6.0, mid - 14.0, 28.0, pal.ink_main.alpha(0.1), pal.ink_main),
            }
            let level = match self.sound.sliding {
                Some((r, v)) if r == i => v,
                _ => ch.level,
            };
            draw::text(s, &canvas, &cfg, &name, 13.0, Face::Text(540), pal.ink_main, sx, iy + 16.0, sw - 40.0, left);
            draw::text(s, &canvas, &cfg, &format!("{:.0}%", level * 100.0), 12.0, Face::Number(500), pal.ink_meta, sx + sw - 40.0, iy + 16.0, 40.0, pango::Alignment::Right);
            let ty = iy + 36.0;
            let frac = level.clamp(0.0, 1.0);
            fill(s, &rounded(sx, ty - 3.0, sw, 6.0, 3.0), pal.ink_main.alpha(0.12));
            let tint = if ch.muted { pal.ink_faint } else { pal.accent };
            fill(s, &rounded(sx, ty - 3.0, (sw * frac).max(6.0), 6.0, 3.0), tint);
            let hot = self.hover == Some(Hit::AppLevel(i)) || matches!(self.sound.sliding, Some((r, _)) if r == i);
            let knob = if hot { 16.0 } else { 14.0 };
            fill(s, &rounded(sx + sw * frac - knob / 2.0, ty - knob / 2.0, knob, knob, knob / 2.0), tint);
            local.push((Hit::AppLevel(i), sx - 10.0, ty - 12.0, sw + 20.0, 24.0));
            let (mx, my) = (cx + cw - 20.0, mid);
            if self.hover == Some(Hit::AppMute(i)) {
                fill(s, &rounded(mx - 16.0, my - 16.0, 32.0, 32.0, 16.0), pal.ink_main.alpha(0.09));
            }
            draw::icon(s, &canvas, &cfg, if ch.muted { "volume_off" } else { "volume_up" }, 19.0, false, if ch.muted { super::RED } else { pal.ink_meta }, mx, my);
            local.push((Hit::AppMute(i), mx - 16.0, my - 16.0, 32.0, 32.0));
            iy += APP;
        }
        s.pop();
        s.pop();
        hits.extend(local.into_iter().filter(|(_, _, hy, _, hh)| hy + hh <= y + vis + 0.5).map(|(h, hx, hy, hw, hh)| (h, hx, hy + shift, hw, hh)));
        vis
    }
}

impl Control {
    /// Shows or hides the sound details, reading the devices and apps as it opens.
    pub(super) fn toggle_sound(&self) {
        let now = glib::monotonic_time();
        let opening = {
            let mut s = self.0.borrow_mut();
            let opening = !s.sound.open;
            s.sound.open = opening;
            if opening {
                s.sound.reveal.go(1.0, now, 420, OPEN);
            } else {
                s.sound.reveal.go(0.0, now, 240, SHUT);
            }
            opening
        };
        if opening {
            self.refresh_sound();
            self.keep_sound_fresh();
            self.resize_with(420, OPEN);
        } else {
            self.resize_with(240, SHUT);
        }
        // Run the frames that reveal (or fold) the card.
        self.kick();
    }

    /// Re-reads the devices and the apps playing, off the main thread.
    pub(super) fn refresh_sound(&self) {
        let asked = {
            let mut s = self.0.borrow_mut();
            s.sound.asked = s.sound.asked.wrapping_add(1);
            s.sound.asked
        };
        let weak = Rc::downgrade(&self.0);
        glib::spawn_future_local(async move {
            let read = gtk::gio::spawn_blocking(|| (audio::devices(false), audio::devices(true), audio::mix(false).map(|m| m.apps).unwrap_or_default())).await;
            let (Ok((outs, ins, apps)), Some(st)) = (read, weak.upgrade()) else { return };
            {
                let mut s = st.borrow_mut();
                if s.sound.asked != asked || s.sound.sliding.is_some() {
                    return;
                }
                if s.sound.apps != apps {
                    let scale = s.canvas.scale_factor();
                    s.sound.faces = apps.iter().take(APPS).map(|ch| {
                        let icon = [ch.name.to_lowercase(), ch.binary.clone()].into_iter().filter(|id| !id.is_empty()).find_map(|id| draw::app_icon(&s.app.apps, &id, 28, scale));
                        (audio::who(&s.app, ch), icon)
                    }).collect();
                }
                let same = s.sound.outs == outs && s.sound.ins == ins && s.sound.apps == apps;
                s.sound.outs = outs;
                s.sound.ins = ins;
                s.sound.apps = apps;
                if same {
                    return;
                }
            }
            // Devices or apps came or went: the card's height changes with them.
            let me = Control(st);
            me.0.borrow_mut().warm_sound();
            me.resize_with(260, OPEN);
            me.kick();
        });
    }

    /// While the details are open, apps start and stop playing: read them again now and then.
    fn keep_sound_fresh(&self) {
        if std::mem::replace(&mut self.0.borrow_mut().sound.watching, true) {
            return;
        }
        let weak = Rc::downgrade(&self.0);
        glib::timeout_add_local(Duration::from_millis(1500), move || {
            let Some(st) = weak.upgrade() else { return glib::ControlFlow::Break };
            let alive = {
                let s = st.borrow();
                s.open && s.sound.open
            };
            if !alive {
                st.borrow_mut().sound.watching = false;
                return glib::ControlFlow::Break;
            }
            Control(st).refresh_sound();
            glib::ControlFlow::Continue
        });
    }

    pub(super) fn sound_act(&self, hit: Hit) {
        let st = self.0.clone();
        match hit {
            Hit::SoundMore => self.toggle_sound(),
            Hit::Out(i) | Hit::In(i) => {
                let input = matches!(hit, Hit::In(_));
                let name = {
                    let mut s = st.borrow_mut();
                    let list = if input { &mut s.sound.ins } else { &mut s.sound.outs };
                    let Some(name) = list.get(i).map(|d| d.name.clone()) else { return };
                    for (k, d) in list.iter_mut().enumerate() {
                        d.default = k == i;
                    }
                    name
                };
                st.borrow().canvas.queue_draw();
                let me = self.clone();
                glib::spawn_future_local(async move {
                    let _ = gtk::gio::spawn_blocking(move || audio::set_default(input, &name)).await;
                    audio::ask(audio::Ask::Read);
                    me.refresh_sound();
                });
            }
            Hit::AppMute(i) => {
                let Some(ch) = st.borrow().sound.apps.get(i).cloned() else { return };
                if let Some(c) = st.borrow_mut().sound.apps.get_mut(i) {
                    c.muted = !ch.muted;
                }
                st.borrow().canvas.queue_draw();
                let me = self.clone();
                glib::spawn_future_local(async move {
                    let _ = gtk::gio::spawn_blocking(move || audio::set_channel(false, false, &ch, None, Some(!ch.muted))).await;
                    me.refresh_sound();
                });
            }
            _ => {}
        }
    }

    /// Starts dragging app slider `i` at `x`.
    pub(super) fn grab_app(&self, i: usize, x: f64) {
        let level = self.0.borrow().sound.apps.get(i).map(|c| c.level);
        if let Some(level) = level {
            self.0.borrow_mut().sound.sliding = Some((i, level));
            self.slide_app(x);
        }
    }

    pub(super) fn sliding_app(&self) -> bool {
        self.0.borrow().sound.sliding.is_some()
    }

    /// Moves the dragged app slider to `x`, setting that app's volume as it goes.
    pub(super) fn slide_app(&self, x: f64) {
        let (row, ch, level) = {
            let mut s = self.0.borrow_mut();
            let Some((row, _)) = s.sound.sliding else { return };
            let (sx, sw) = s.sound.track;
            if sw <= 0.0 {
                return;
            }
            let level = ((x - sx) / sw).clamp(0.0, 1.0);
            s.sound.sliding = Some((row, level));
            let Some(ch) = s.sound.apps.get(row).cloned() else { return };
            (row, ch, level)
        };
        self.0.borrow().canvas.queue_draw();
        let _ = row;
        glib::spawn_future_local(async move {
            let _ = gtk::gio::spawn_blocking(move || audio::set_channel(false, false, &ch, Some(level), None)).await;
        });
    }

    /// Lets go of the app slider, keeping the level it was left at.
    pub(super) fn drop_app(&self) {
        {
            let mut s = self.0.borrow_mut();
            if let Some((row, level)) = s.sound.sliding.take() && let Some(c) = s.sound.apps.get_mut(row) {
                c.level = level;
            }
        }
        self.refresh_sound();
    }
}

