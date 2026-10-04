//! Window class -> app icon, the way a dock would do it (simplified):
//! 1. find the app's .desktop file (by file name or `StartupWMClass`) and read `Icon=`;
//! 2. look that name up in the user's icon theme, then hicolor, then /usr/share/pixmaps.
//!
//! Not the full freedesktop icon spec: theme `Inherits` chains and `index.theme`
//! directory lists are ignored; we probe the usual `<size>/apps` directories instead.

use crate::{
    canvas::{Image, Svg},
    gif::scale_premultiplied,
};
use anyhow::{Context, Result};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    rc::Rc,
};

/// Probed in this order: vector first, then sizes close to the bar's ~36 px, then
/// small ones (Papirus only ships action icons at 16-24 px, as SVG).
const SIZES: &[&str] = &[
    "scalable", "64x64", "48x48", "96x96", "128x128", "256x256", "32x32", "24x24", "22x22", "16x16",
];
/// App icons for windows; the rest for config buttons (media keys etc.).
const CATEGORIES: &[&str] = &["apps", "actions", "status", "devices", "places", "panel"];

#[derive(Clone)]
pub enum AppIcon {
    Svg(Rc<Svg>),
    Raster(Rc<Image>),
}

pub struct IconResolver {
    /// Icon roots (…/share/icons), user's first.
    icon_bases: Vec<PathBuf>,
    themes: Vec<String>,
    app_dirs: Vec<PathBuf>,
    /// Built on first use: lowercase key -> Icon= value.
    by_wm_class: HashMap<String, String>,
    by_desktop_id: HashMap<String, String>,
    indexed: bool,
    size: u32,
    cache: HashMap<String, Option<AppIcon>>,
}

impl IconResolver {
    /// `home` is the real (sudo) user's home, for their theme and local apps/icons.
    pub fn new(home: Option<&Path>, size: u32) -> IconResolver {
        let mut icon_bases = Vec::new();
        let mut app_dirs = Vec::new();
        let mut themes = Vec::new();
        if let Some(h) = home {
            icon_bases.push(h.join(".local/share/icons"));
            icon_bases.push(h.join(".icons"));
            app_dirs.push(h.join(".local/share/applications"));
            if let Some(t) = gtk_icon_theme(h) {
                themes.push(t);
            }
        }
        icon_bases.extend(
            [
                "/usr/local/share/icons",
                "/usr/share/icons",
                "/var/lib/flatpak/exports/share/icons",
            ]
            .map(PathBuf::from),
        );
        app_dirs.extend(
            [
                "/usr/local/share/applications",
                "/usr/share/applications",
                "/var/lib/flatpak/exports/share/applications",
            ]
            .map(PathBuf::from),
        );
        // The bar is dark: prefer the theme's dark variant if it has one (Papirus's
        // action icons are dark grey, invisible on black; Papirus-Dark's are light).
        if let Some(t) = themes.first().cloned() {
            let dark = format!("{t}-Dark");
            if icon_bases.iter().any(|b| b.join(&dark).is_dir()) {
                themes.insert(0, dark);
            }
        }
        themes.push("hicolor".into());
        themes.push("Adwaita".into());
        eprintln!("icons: themes {themes:?}");
        IconResolver {
            icon_bases,
            themes,
            app_dirs,
            by_wm_class: HashMap::new(),
            by_desktop_id: HashMap::new(),
            indexed: false,
            size,
            cache: HashMap::new(),
        }
    }

    /// Icon for a window class, or `None` (callers draw a generic one). Cached,
    /// including misses.
    pub fn for_class(&mut self, class: &str) -> Option<AppIcon> {
        if let Some(hit) = self.cache.get(class) {
            return hit.clone();
        }
        let icon = self.resolve(class);
        if icon.is_none() {
            eprintln!("icons: nothing for class {class:?}, using generic icon");
        }
        self.cache.insert(class.to_string(), icon.clone());
        icon
    }

    /// Icon by theme name or absolute path (config buttons). Cached.
    pub fn named(&mut self, name: &str) -> Option<AppIcon> {
        let key = format!("name:{name}");
        if let Some(hit) = self.cache.get(&key) {
            return hit.clone();
        }
        let icon = self.load_named(name);
        if icon.is_none() {
            eprintln!("icons: icon {name:?} not found");
        }
        self.cache.insert(key, icon.clone());
        icon
    }

    fn resolve(&mut self, class: &str) -> Option<AppIcon> {
        self.index_desktop_files();
        let lower = class.to_lowercase();
        // `kitty-start` -> `kitty`, `org.foo.Bar` keeps its full name too.
        let stem = lower
            .split(['-', '_', ' '])
            .next()
            .unwrap_or(&lower)
            .to_string();
        let mut names = Vec::new();
        for key in [&lower, &stem] {
            if let Some(icon) = self.by_wm_class.get(key).or(self.by_desktop_id.get(key)) {
                names.push(icon.clone());
            }
        }
        names.extend([class.to_string(), lower.clone(), stem]);
        names.dedup();
        names.iter().find_map(|n| self.load_named(n))
    }

    fn index_desktop_files(&mut self) {
        if self.indexed {
            return;
        }
        self.indexed = true;
        for dir in &self.app_dirs {
            let Ok(entries) = fs::read_dir(dir) else {
                continue;
            };
            for e in entries.flatten() {
                let path = e.path();
                if path.extension().is_none_or(|x| x != "desktop") {
                    continue;
                }
                let Ok(text) = fs::read_to_string(&path) else {
                    continue;
                };
                let (icon, wm_class) = parse_desktop(&text);
                let Some(icon) = icon else { continue };
                if let Some(c) = wm_class {
                    self.by_wm_class
                        .entry(c.to_lowercase())
                        .or_insert(icon.clone());
                }
                if let Some(id) = path.file_stem().and_then(|s| s.to_str()) {
                    self.by_desktop_id.entry(id.to_lowercase()).or_insert(icon);
                }
            }
        }
    }

    fn load_named(&self, name: &str) -> Option<AppIcon> {
        let path = self.find_icon_file(name)?;
        match load_icon_file(&path, self.size) {
            Ok(icon) => Some(icon),
            Err(e) => {
                eprintln!("icons: {e:#}");
                None
            }
        }
    }

    fn find_icon_file(&self, name: &str) -> Option<PathBuf> {
        let p = Path::new(name);
        if p.is_absolute() {
            return p.is_file().then(|| p.to_path_buf());
        }
        for theme in &self.themes {
            for base in &self.icon_bases {
                let root = base.join(theme);
                if !root.is_dir() {
                    continue;
                }
                for size in SIZES {
                    for cat in CATEGORIES {
                        for ext in ["svg", "png"] {
                            let f = root.join(size).join(cat).join(format!("{name}.{ext}"));
                            if f.is_file() {
                                return Some(f);
                            }
                        }
                    }
                }
            }
        }
        ["svg", "png"]
            .iter()
            .map(|ext| PathBuf::from(format!("/usr/share/pixmaps/{name}.{ext}")))
            .find(|f| f.is_file())
    }
}

fn load_icon_file(path: &Path, size: u32) -> Result<AppIcon> {
    if path.extension().is_some_and(|x| x == "svg") {
        return Ok(AppIcon::Svg(Rc::new(Svg::load(path)?)));
    }
    let img = image::open(path)
        .with_context(|| format!("loading {}", path.display()))?
        .into_rgba8();
    Ok(AppIcon::Raster(Rc::new(scale_premultiplied(img, size)?)))
}

/// `Icon=` and `StartupWMClass=` from the `[Desktop Entry]` group.
fn parse_desktop(text: &str) -> (Option<String>, Option<String>) {
    let mut in_entry = false;
    let (mut icon, mut wm_class) = (None, None);
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        if let Some(v) = line.strip_prefix("Icon=") {
            icon = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("StartupWMClass=") {
            wm_class = Some(v.trim().to_string());
        }
    }
    (
        icon.filter(|s| !s.is_empty()),
        wm_class.filter(|s| !s.is_empty()),
    )
}

/// `gtk-icon-theme-name` from the user's GTK 3/4 settings.
fn gtk_icon_theme(home: &Path) -> Option<String> {
    ["gtk-3.0", "gtk-4.0"].iter().find_map(|v| {
        let text = fs::read_to_string(home.join(".config").join(v).join("settings.ini")).ok()?;
        text.lines().find_map(|l| {
            let (k, v) = l.split_once('=')?;
            (k.trim() == "gtk-icon-theme-name")
                .then(|| v.trim().trim_matches(['\'', '"']).to_string())
                .filter(|v| !v.is_empty())
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_entry_parsing_ignores_actions() {
        let text = "[Desktop Entry]\nName=Zen\nIcon=zen-browser\nStartupWMClass=zen\n\
                    [Desktop Action new-window]\nIcon=other\n";
        assert_eq!(
            parse_desktop(text),
            (Some("zen-browser".into()), Some("zen".into()))
        );
    }
}
