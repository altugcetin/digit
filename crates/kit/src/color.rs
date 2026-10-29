#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

fn num(s: &str, scale: f64) -> Option<f64> {
    let s = s.trim();
    if let Some(p) = s.strip_suffix('%') {
        return p.trim().parse::<f64>().ok().map(|v| v / 100.0);
    }
    s.parse::<f64>().ok().map(|v| v / scale)
}

fn args(s: &str, names: &[&str]) -> Option<Vec<String>> {
    let lower = s.trim().to_ascii_lowercase();
    let name = names.iter().find(|n| lower.starts_with(&format!("{n}(")))?;
    let inner = lower.strip_prefix(&format!("{name}("))?.strip_suffix(')')?;
    let parts: Vec<String> = inner.split(|c| c == ',' || c == '/' || c == ' ').map(str::trim).filter(|p| !p.is_empty()).map(str::to_string).collect();
    (parts.len() >= 3).then_some(parts)
}

fn hue_rgb(h: f64, s: f64, l: f64) -> (f64, f64, f64) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = (h.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    (r + m, g + m, b + m)
}

pub fn parse(text: &str) -> Option<Color> {
    let t = text.trim();
    if t.is_empty() || t.len() > 64 || t.contains('\n') {
        return None;
    }
    let hex = t.strip_prefix('#').or_else(|| t.strip_prefix("0x"));
    if let Some(h) = hex {
        if h.chars().all(|c| c.is_ascii_hexdigit()) {
            let v = |s: &str| u8::from_str_radix(s, 16).ok().map(|x| x as f64 / 255.0);
            let d = |c: char| v(&format!("{c}{c}"));
            let cs: Vec<char> = h.chars().collect();
            return match cs.len() {
                3 => Some(Color { r: d(cs[0])?, g: d(cs[1])?, b: d(cs[2])?, a: 1.0 }),
                4 => Some(Color { r: d(cs[0])?, g: d(cs[1])?, b: d(cs[2])?, a: d(cs[3])? }),
                6 => Some(Color { r: v(&h[0..2])?, g: v(&h[2..4])?, b: v(&h[4..6])?, a: 1.0 }),
                8 => Some(Color { r: v(&h[0..2])?, g: v(&h[2..4])?, b: v(&h[4..6])?, a: v(&h[6..8])? }),
                _ => None,
            };
        }
        return None;
    }
    if let Some(p) = args(t, &["rgba", "rgb"]) {
        let c = Color { r: num(&p[0], 255.0)?, g: num(&p[1], 255.0)?, b: num(&p[2], 255.0)?, a: p.get(3).and_then(|a| num(a, 1.0)).unwrap_or(1.0) };
        return valid(c);
    }
    if let Some(p) = args(t, &["hsla", "hsl"]) {
        let h = p[0].trim_end_matches("deg").parse::<f64>().ok()?;
        let (r, g, b) = hue_rgb(h, num(&p[1], 100.0)?, num(&p[2], 100.0)?);
        return valid(Color { r, g, b, a: p.get(3).and_then(|a| num(a, 1.0)).unwrap_or(1.0) });
    }
    let parts: Vec<&str> = t.split(|c| c == ',' || c == ' ').map(str::trim).filter(|p| !p.is_empty()).collect();
    if parts.len() == 3 && parts.iter().all(|p| p.parse::<u16>().is_ok_and(|v| v <= 255)) {
        let v: Vec<f64> = parts.iter().map(|p| p.parse::<f64>().unwrap_or(0.0) / 255.0).collect();
        return Some(Color { r: v[0], g: v[1], b: v[2], a: 1.0 });
    }
    None
}

fn valid(c: Color) -> Option<Color> {
    [c.r, c.g, c.b, c.a].iter().all(|v| (0.0..=1.0).contains(v)).then_some(c)
}

fn linear(c: f64) -> f64 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

impl Color {
    pub fn bytes(&self) -> (u8, u8, u8) {
        ((self.r * 255.0).round() as u8, (self.g * 255.0).round() as u8, (self.b * 255.0).round() as u8)
    }

    pub fn hex(&self) -> String {
        let (r, g, b) = self.bytes();
        if self.a < 0.999 {
            format!("#{r:02x}{g:02x}{b:02x}{:02x}", (self.a * 255.0).round() as u8)
        } else {
            format!("#{r:02x}{g:02x}{b:02x}")
        }
    }

    pub fn hsl(&self) -> (f64, f64, f64) {
        let (max, min) = (self.r.max(self.g).max(self.b), self.r.min(self.g).min(self.b));
        let l = (max + min) / 2.0;
        let d = max - min;
        if d < 1e-9 {
            return (0.0, 0.0, l);
        }
        let s = d / (1.0 - (2.0 * l - 1.0).abs());
        (self.hue(max, d), s, l)
    }

    fn hue(&self, max: f64, d: f64) -> f64 {
        let h = if max == self.r {
            ((self.g - self.b) / d).rem_euclid(6.0)
        } else if max == self.g {
            (self.b - self.r) / d + 2.0
        } else {
            (self.r - self.g) / d + 4.0
        };
        h * 60.0
    }

    pub fn hsv(&self) -> (f64, f64, f64) {
        let (max, min) = (self.r.max(self.g).max(self.b), self.r.min(self.g).min(self.b));
        let d = max - min;
        let h = if d < 1e-9 { 0.0 } else { self.hue(max, d) };
        (h, if max > 0.0 { d / max } else { 0.0 }, max)
    }

    pub fn oklch(&self) -> (f64, f64, f64) {
        let (r, g, b) = (linear(self.r), linear(self.g), linear(self.b));
        let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
        let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
        let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
        let ll = 0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s;
        let a = 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s;
        let bb = 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s;
        let c = (a * a + bb * bb).sqrt();
        let h = if c < 1e-6 { 0.0 } else { bb.atan2(a).to_degrees().rem_euclid(360.0) };
        (ll, c, h)
    }

    pub fn formats(&self) -> Vec<(&'static str, String)> {
        let (r, g, b) = self.bytes();
        let (h, s, l) = self.hsl();
        let (vh, vs, vv) = self.hsv();
        let (ol, oc, oh) = self.oklch();
        let alpha = if self.a < 0.999 { format!(" / {:.2}", self.a) } else { String::new() };
        vec![
            ("HEX", self.hex()),
            ("RGB", format!("rgb({r} {g} {b}{alpha})")),
            ("HSL", format!("hsl({:.0} {:.0}% {:.0}%{alpha})", h, s * 100.0, l * 100.0)),
            ("HSV", format!("hsv({:.0} {:.0}% {:.0}%)", vh, vs * 100.0, vv * 100.0)),
            ("OKLCH", format!("oklch({:.1}% {:.3} {:.1}{alpha})", ol * 100.0, oc, oh)),
            ("Values", format!("{r}, {g}, {b}")),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_converts() {
        let c = parse("#FF8800").unwrap();
        assert_eq!(c.bytes(), (255, 136, 0));
        assert_eq!(parse("rgb(255, 136, 0)").unwrap().bytes(), (255, 136, 0));
        assert_eq!(parse("rgb(255 136 0 / 50%)").unwrap().a, 0.5);
        assert_eq!(parse("hsl(32, 100%, 50%)").unwrap().bytes(), (255, 136, 0));
        assert_eq!(parse("#f80").unwrap().bytes(), (255, 136, 0));
        assert_eq!(parse("255, 136, 0").unwrap().bytes(), (255, 136, 0));
        assert!(parse("hello").is_none());
        assert!(parse("#12345").is_none());
        let (l, ch, h) = parse("#ffffff").unwrap().oklch();
        assert!((l - 1.0).abs() < 1e-3 && ch < 1e-3 && h == 0.0);
        let (l, ch, h) = parse("#ff0000").unwrap().oklch();
        assert!((l - 0.628).abs() < 2e-3 && (ch - 0.2577).abs() < 2e-3 && (h - 29.23).abs() < 0.2, "{l} {ch} {h}");
        assert_eq!(c.formats()[2].1, "hsl(32 100% 50%)");
    }
}
