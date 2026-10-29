use gtk::gdk::RGBA;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

impl Rgb {
    pub const fn new(r: f64, g: f64, b: f64) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    pub fn hex(s: &str) -> Option<Self> {
        let s = s.trim().trim_start_matches('#');
        let v = |i: usize| u8::from_str_radix(s.get(i..i + 2)?, 16).ok().map(|x| x as f64 / 255.0);
        match s.len() {
            6 => Some(Self::new(v(0)?, v(2)?, v(4)?)),
            8 => Some(Self { a: v(6)?, ..Self::new(v(0)?, v(2)?, v(4)?) }),
            _ => None,
        }
    }

    pub fn mix(self, other: Self, p: f64) -> Self {
        let q = 1.0 - p;
        Self {
            r: p * self.r + q * other.r,
            g: p * self.g + q * other.g,
            b: p * self.b + q * other.b,
            a: p * self.a + q * other.a,
        }
    }

    pub fn alpha(self, a: f64) -> Self {
        Self { a, ..self }
    }

    pub fn rgba(self) -> RGBA {
        RGBA::new(self.r as f32, self.g as f32, self.b as f32, self.a as f32)
    }

    fn lightness(self) -> f64 {
        (self.r.max(self.g).max(self.b) + self.r.min(self.g).min(self.b)) / 2.0
    }

    fn luminance(self) -> f64 {
        let y = 0.2126729 * self.r.powf(2.4) + 0.7151522 * self.g.powf(2.4) + 0.0721750 * self.b.powf(2.4);
        if y > 0.022 { y } else { y + (0.022 - y).powf(1.414) }
    }
}

pub fn apca(text: Rgb, ground: Rgb) -> f64 {
    let (yt, yb) = (text.luminance(), ground.luminance());
    if yb > yt {
        let s = (yb.powf(0.56) - yt.powf(0.57)) * 1.14;
        if s < 0.1 { 0.0 } else { (s - 0.027) * 100.0 }
    } else {
        let s = (yb.powf(0.65) - yt.powf(0.62)) * 1.14;
        if s > -0.1 { 0.0 } else { (s + 0.027) * 100.0 }
    }
}

fn ink_for(lc: f64, ink: Rgb, ground: Rgb) -> Rgb {
    let far = if ground.lightness() < 0.5 { Rgb::new(1.0, 1.0, 1.0) } else { Rgb::new(0.0, 0.0, 0.0) };
    let at = |t: f64| if t <= 1.0 { ink.mix(ground, t) } else { far.mix(ink, t - 1.0) };
    let (mut lo, mut hi) = (0.0, 2.0);
    for _ in 0..18 {
        let mid = (lo + hi) / 2.0;
        if apca(at(mid), ground).abs() < lc { lo = mid } else { hi = mid }
    }
    at(hi)
}

#[derive(Clone, Copy, Debug)]
pub struct Roles {
    pub surface: Rgb,
    pub surface_high: Rgb,
    pub on_surface: Rgb,
    pub primary: Rgb,
    pub accent_container: Rgb,
    pub on_accent: Rgb,
    pub error: Rgb,
}

impl Default for Roles {
    fn default() -> Self {
        Self {
            surface: Rgb::new(0.078, 0.075, 0.075),
            surface_high: Rgb::new(0.169, 0.165, 0.165),
            on_surface: Rgb::new(0.902, 0.882, 0.882),
            primary: Rgb::new(0.906, 0.741, 0.718),
            accent_container: Rgb::new(0.169, 0.165, 0.165),
            on_accent: Rgb::new(0.27, 0.16, 0.15),
            error: Rgb::new(1.0, 0.706, 0.671),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub body: Rgb,
    pub ground: Rgb,
    pub ink_main: Rgb,
    pub ink_sec: Rgb,
    pub ink_meta: Rgb,
    pub ink_faint: Rgb,
    pub hairline: Rgb,
    pub raised: Rgb,
    pub accent: Rgb,
    pub on_accent: Rgb,
    pub risk: Rgb,
}

impl Palette {
    pub fn lerp(&self, to: &Palette, t: f64) -> Palette {
        let m = |a: Rgb, b: Rgb| b.mix(a, t);
        Palette {
            body: m(self.body, to.body),
            ground: m(self.ground, to.ground),
            ink_main: m(self.ink_main, to.ink_main),
            ink_sec: m(self.ink_sec, to.ink_sec),
            ink_meta: m(self.ink_meta, to.ink_meta),
            ink_faint: m(self.ink_faint, to.ink_faint),
            hairline: m(self.hairline, to.hairline),
            raised: m(self.raised, to.raised),
            accent: m(self.accent, to.accent),
            on_accent: m(self.on_accent, to.on_accent),
            risk: m(self.risk, to.risk),
        }
    }

    pub fn new(roles: Roles, body: Rgb) -> Self {
        let ground = body.alpha(1.0);
        let main = ink_for(92.0, roles.on_surface, ground);
        let secondary = ink_for(85.0, roles.on_surface, ground);
        let meta = ink_for(75.0, roles.on_surface, ground);
        Self {
            body,
            ground,
            ink_main: main,
            ink_sec: main.mix(secondary, 0.25),
            ink_meta: secondary.mix(meta, 0.4),
            ink_faint: ink_for(60.0, roles.on_surface, ground),
            hairline: ink_for(16.0, roles.on_surface, ground),
            raised: main.mix(ground, 0.11),
            accent: roles.primary,
            on_accent: roles.on_accent,
            risk: roles.error,
        }
    }
}

pub fn gtk_dir() -> PathBuf {
    gtk::glib::user_config_dir().join("gtk-4.0")
}

fn defines() -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let Ok(entries) = std::fs::read_dir(gtk_dir()) else { return map };
    let mut files: Vec<_> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "css")).collect();
    files.sort();
    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else { continue };
        for line in text.lines() {
            let Some(rest) = line.trim().strip_prefix("@define-color ") else { continue };
            let mut parts = rest.trim_end_matches(';').splitn(2, char::is_whitespace);
            if let (Some(k), Some(v)) = (parts.next(), parts.next()) {
                map.insert(k.to_string(), v.trim().to_string());
            }
        }
    }
    map
}

pub fn gtk_roles() -> Option<Roles> {
    let map = defines();
    let resolve = |key: &str| {
        let mut v = map.get(key)?.as_str();
        for _ in 0..4 {
            match v.strip_prefix('@') {
                Some(next) => v = map.get(next)?.as_str(),
                None => break,
            }
        }
        Rgb::hex(v)
    };
    let fallback = Roles::default();
    let surface = resolve("window_bg_color")?;
    Some(Roles {
        surface,
        surface_high: resolve("card_bg_color").or_else(|| resolve("popover_bg_color")).unwrap_or(surface),
        on_surface: resolve("window_fg_color")?,
        primary: resolve("accent_bg_color").or_else(|| resolve("accent_color")).unwrap_or(fallback.primary),
        accent_container: resolve("card_bg_color").unwrap_or(surface),
        on_accent: resolve("accent_fg_color").unwrap_or(fallback.on_accent),
        error: resolve("destructive_bg_color").or_else(|| resolve("error_color")).unwrap_or(fallback.error),
    })
}
