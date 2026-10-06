//! The box an item is drawn in: its shape, corner radius and background, from the
//! config (`shape`, `radius`, `background` on an item, or `item_*` on its layer).

use crate::canvas::{Canvas, Rect, Rgba};
use serde::Deserialize;

/// Corner radius when nothing is configured.
pub const DEFAULT_RADIUS: f32 = 8.0;
/// Background when nothing is configured.
pub const DEFAULT_BACKGROUND: Rgba = Rgba(0x3a, 0x3a, 0x3c, 0xff);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Shape {
    /// Rectangle with rounded corners (`radius`).
    #[default]
    Rounded,
    /// A circle; the item is made square (as wide as the row is tall).
    Circle,
    /// No background at all, just the content.
    None,
}

/// `radius = <px>` or `radius = "full"` (half the height: a pill).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Radius {
    Px(f32),
    Full,
}

impl<'de> Deserialize<'de> for Radius {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Radius, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Num(f64),
            Text(String),
        }
        let bad = |what: String| {
            serde::de::Error::custom(format!(
                "bad radius {what}, want a number of px >= 0 or \"full\""
            ))
        };
        match Raw::deserialize(d).map_err(|_| bad("value".into()))? {
            Raw::Num(n) if n.is_finite() && (0.0..=4000.0).contains(&n) => Ok(Radius::Px(n as f32)),
            Raw::Num(n) => Err(bad(n.to_string())),
            Raw::Text(s) if s == "full" => Ok(Radius::Full),
            Raw::Text(s) => Err(bad(format!("{s:?}"))),
        }
    }
}

/// `background = "#rrggbb" | "#rrggbbaa" | "transparent"`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Background {
    Color(Rgba),
    Transparent,
}

impl<'de> Deserialize<'de> for Background {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Background, D::Error> {
        let s = String::deserialize(d)?;
        if s == "transparent" {
            return Ok(Background::Transparent);
        }
        crate::config::parse_color(&s)
            .map(Background::Color)
            .ok_or_else(|| {
                serde::de::Error::custom(format!(
                    "bad background {s:?}, want \"#rrggbb\", \"#rrggbbaa\" or \"transparent\""
                ))
            })
    }
}

/// How one item's box looks, with every default resolved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub shape: Shape,
    pub radius: Radius,
    /// `None`: nothing drawn behind the content.
    pub background: Option<Rgba>,
}

impl Default for Frame {
    fn default() -> Frame {
        Frame {
            shape: Shape::Rounded,
            radius: Radius::Px(DEFAULT_RADIUS),
            background: Some(DEFAULT_BACKGROUND),
        }
    }
}

impl Frame {
    /// A frame like the default one with another background colour.
    pub fn with_background(color: Rgba) -> Frame {
        Frame {
            background: Some(color),
            ..Frame::default()
        }
    }

    /// Corner radius for a box of this size. Circles are pills: on the square they
    /// are given that is a circle, and an unfolding slider stays round-ended.
    pub fn corner(&self, r: Rect) -> f32 {
        let half = r.w.min(r.h) / 2.0;
        match (self.shape, self.radius) {
            (Shape::Circle, _) | (_, Radius::Full) => half,
            (_, Radius::Px(px)) => px.min(half),
        }
    }

    /// Whether a background is drawn at all.
    pub fn has_background(&self) -> bool {
        self.shape != Shape::None && self.background.is_some()
    }

    /// The background, if any.
    pub fn draw_background(&self, canvas: &mut Canvas, r: Rect) {
        if let (true, Some(color)) = (self.has_background(), self.background) {
            self.fill(canvas, r, color);
        }
    }

    /// Fills the frame's outline with `color`, whatever the background: used for the
    /// pressed highlight, so it shows on transparent and shapeless items too.
    pub fn fill(&self, canvas: &mut Canvas, r: Rect, color: Rgba) {
        canvas.fill_rounded_rect(r.x, r.y, r.w, r.h, self.corner(r), color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct T {
        radius: Option<Radius>,
        background: Option<Background>,
        shape: Option<Shape>,
    }

    fn parse(s: &str) -> Result<T, toml::de::Error> {
        toml::from_str(s)
    }

    #[test]
    fn parses_and_rejects() {
        assert_eq!(parse("radius = 12").unwrap().radius, Some(Radius::Px(12.0)));
        assert_eq!(parse("radius = 2.5").unwrap().radius, Some(Radius::Px(2.5)));
        assert_eq!(parse("radius = 'full'").unwrap().radius, Some(Radius::Full));
        assert_eq!(
            parse("background = 'transparent'").unwrap().background,
            Some(Background::Transparent)
        );
        assert!(matches!(
            parse("background = '#10203080'").unwrap().background,
            Some(Background::Color(Rgba(0x10, 0x20, 0x30, 0x80)))
        ));
        assert_eq!(parse("shape = 'circle'").unwrap().shape, Some(Shape::Circle));
        for bad in [
            "radius = -1",
            "radius = nan",
            "radius = inf",
            "radius = 'round'",
            "radius = true",
            "background = 'red'",
            "background = '#fff'",
            "background = 'none'",
            "shape = 'square'",
            "shape = 'Rounded'",
        ] {
            assert!(parse(bad).is_err(), "accepted: {bad}");
        }
        let e = parse("radius = -3").err().unwrap().to_string();
        assert!(e.contains("bad radius -3"), "{e}");
    }

    #[test]
    fn corners() {
        let r = Rect::new(0.0, 0.0, 200.0, 52.0);
        let f = |shape, radius| Frame {
            shape,
            radius,
            background: None,
        };
        assert_eq!(f(Shape::Rounded, Radius::Px(8.0)).corner(r), 8.0);
        assert_eq!(f(Shape::Rounded, Radius::Px(99.0)).corner(r), 26.0);
        assert_eq!(f(Shape::Rounded, Radius::Full).corner(r), 26.0);
        assert_eq!(f(Shape::Circle, Radius::Px(3.0)).corner(r), 26.0);
        assert!(!f(Shape::None, Radius::Full).has_background());
        assert!(!f(Shape::Rounded, Radius::Full).has_background());
        assert!(Frame::default().has_background());
    }
}
