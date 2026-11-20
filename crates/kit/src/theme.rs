use crate::palette::{Rgb, Roles};
use gtk::gdk_pixbuf::Pixbuf;
use material_colors::color::Rgb as Argb;
use material_colors::dynamic_color::{DynamicColor, DynamicScheme, MaterialDynamicColors as M};
use material_colors::hct::Hct;
use material_colors::scheme::variant::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const SCHEMES: [&str; 9] = ["content", "tonal-spot", "monochrome", "neutral", "expressive", "fidelity", "vibrant", "rainbow", "fruit-salad"];

type Role = fn() -> DynamicColor<'static>;

const ROLES: [(&str, Role); 37] = [
    ("background", M::background),
    ("on_background", M::on_background),
    ("surface", M::surface),
    ("surface_dim", M::surface_dim),
    ("surface_bright", M::surface_bright),
    ("surface_container_lowest", M::surface_container_lowest),
    ("surface_container_low", M::surface_container_low),
    ("surface_container", M::surface_container),
    ("surface_container_high", M::surface_container_high),
    ("surface_container_highest", M::surface_container_highest),
    ("on_surface", M::on_surface),
    ("surface_variant", M::surface_variant),
    ("on_surface_variant", M::on_surface_variant),
    ("inverse_surface", M::inverse_surface),
    ("inverse_on_surface", M::inverse_on_surface),
    ("outline", M::outline),
    ("outline_variant", M::outline_variant),
    ("shadow", M::shadow),
    ("scrim", M::scrim),
    ("primary", M::primary),
    ("on_primary", M::on_primary),
    ("primary_container", M::primary_container),
    ("on_primary_container", M::on_primary_container),
    ("inverse_primary", M::inverse_primary),
    ("secondary", M::secondary),
    ("on_secondary", M::on_secondary),
    ("secondary_container", M::secondary_container),
    ("on_secondary_container", M::on_secondary_container),
    ("tertiary", M::tertiary),
    ("on_tertiary", M::on_tertiary),
    ("tertiary_container", M::tertiary_container),
    ("on_tertiary_container", M::on_tertiary_container),
    ("error", M::error),
    ("on_error", M::on_error),
    ("error_container", M::error_container),
    ("on_error_container", M::on_error_container),
    ("surface_tint", M::surface_tint),
];

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Theme {
    pub wallpaper: String,
    pub scheme: String,
    pub source: String,
    pub roles: BTreeMap<String, String>,
}

impl Theme {
    pub fn role(&self, name: &str) -> Option<Rgb> {
        Rgb::hex(self.roles.get(name)?)
    }

    pub fn hex(&self, name: &str) -> String {
        self.roles.get(name).cloned().unwrap_or_else(|| "#ff00ff".into())
    }

    pub fn shell_roles(&self) -> Option<Roles> {
        Some(Roles {
            surface: self.role("surface_container_low")?,
            surface_high: self.role("surface_container_high")?,
            on_surface: self.role("on_surface")?,
            primary: self.role("primary")?,
            accent_container: self.role("primary_container")?,
            on_accent: self.role("on_primary")?,
            error: self.role("error")?,
        })
    }
}

pub fn state_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| gtk::glib::home_dir().join(".local/state"))
        .join("digit")
}

pub fn path() -> PathBuf {
    state_dir().join("theme.json")
}

pub fn load() -> Option<Theme> {
    serde_json::from_str(&std::fs::read_to_string(path()).ok()?).ok()
}

pub fn save(theme: &Theme) -> std::io::Result<()> {
    crate::atomic::write(&path(), serde_json::to_string_pretty(theme).unwrap_or_default())
}

fn source_color(image: &Path) -> Option<Argb> {
    let pixbuf = Pixbuf::from_file_at_scale(image, 128, 128, true).ok()?;
    let n = pixbuf.n_channels() as usize;
    let stride = pixbuf.rowstride() as usize;
    let alpha = pixbuf.has_alpha();
    let bytes = pixbuf.read_pixel_bytes();
    let mut pixels = Vec::with_capacity((pixbuf.width() * pixbuf.height()) as usize);
    for y in 0..pixbuf.height() as usize {
        for x in 0..pixbuf.width() as usize {
            let i = y * stride + x * n;
            if alpha && bytes[i + 3] < 255 {
                continue;
            }
            pixels.push(Argb::new(bytes[i], bytes[i + 1], bytes[i + 2]));
        }
    }
    (!pixels.is_empty()).then(|| material_colors::image::extract_color(&pixels))
}

fn scheme(name: &str, source: Hct) -> DynamicScheme {
    match name {
        "tonal-spot" => SchemeTonalSpot::new(source, true, None).scheme,
        "monochrome" => SchemeMonochrome::new(source, true, None).scheme,
        "neutral" => SchemeNeutral::new(source, true, None).scheme,
        "expressive" => SchemeExpressive::new(source, true, None).scheme,
        "fidelity" => SchemeFidelity::new(source, true, None).scheme,
        "vibrant" => SchemeVibrant::new(source, true, None).scheme,
        "rainbow" => SchemeRainbow::new(source, true, None).scheme,
        "fruit-salad" => SchemeFruitSalad::new(source, true, None).scheme,
        _ => SchemeContent::new(source, true, None).scheme,
    }
}

/// A scheme's look for a preview, from the wallpaper's source color: its surface, then its
/// primary, secondary and tertiary colors.
pub fn preview(source: &str, name: &str) -> Option<[Rgb; 4]> {
    let c = Rgb::hex(source)?;
    let argb = Argb::new((c.r * 255.0).round() as u8, (c.g * 255.0).round() as u8, (c.b * 255.0).round() as u8);
    let dynamic = scheme(name, Hct::new(argb));
    let get = |role: Role| {
        let v = dynamic.get_rgb(role());
        Rgb::new(v.red as f64 / 255.0, v.green as f64 / 255.0, v.blue as f64 / 255.0)
    };
    Some([get(M::surface_container_high), get(M::primary), get(M::secondary), get(M::tertiary)])
}

fn hex(c: Argb) -> String {
    format!("#{:02x}{:02x}{:02x}", c.red, c.green, c.blue)
}

pub fn generate(image: &Path, name: &str) -> Option<Theme> {
    let source = source_color(image)?;
    let dynamic = scheme(name, Hct::new(source));
    let roles = ROLES.iter().map(|(k, role)| (k.to_string(), hex(dynamic.get_rgb(role())))).collect();
    Some(Theme {
        wallpaper: image.to_string_lossy().into_owned(),
        scheme: if SCHEMES.contains(&name) { name.into() } else { "content".into() },
        source: hex(source),
        roles,
    })
}
