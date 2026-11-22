//! Typed config values and their text form.

use std::num::NonZeroU32;
use std::path::PathBuf;

/// A config value type: parse from and print to the file syntax.
pub trait Value: Sized {
    fn parse(s: &str) -> Result<Self, String>;
    fn show(&self) -> String;
}

/// `#rrggbb` (the `#` is optional).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// How the frame is scaled into the window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scale {
    /// Largest integer multiple that fits.
    #[default]
    Integer,
    /// Largest aspect-correct size that fits.
    Fit,
    /// Fill the window.
    Stretch,
}

/// What fills the padding around the frame (Ghostty `window-padding-color`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaddingColor {
    #[default]
    Background,
    Extend,
    ExtendAlways,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextRender {
    #[default]
    Bitmap,
    Vector,
}

/// `0..=100`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Percent(pub u8);

/// `mount = C:path`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    /// Uppercase `A..=Z`.
    pub letter: char,
    pub path: PathBuf,
}

macro_rules! enum_value {
    ($t:ty, $what:literal, $($s:literal => $v:expr),*) => {
        impl Value for $t {
            fn parse(s: &str) -> Result<Self, String> {
                match s {
                    $($s => Ok($v),)*
                    _ => Err(format!(concat!("invalid ", $what, " {:?}; expected one of: "), s)
                        + &[$($s),*].join(", ")),
                }
            }
            fn show(&self) -> String {
                $(if *self == $v { return $s.into(); })*
                unreachable!()
            }
        }
    };
}

enum_value!(bool, "bool", "true" => true, "false" => false);
enum_value!(Scale, "scale", "integer" => Scale::Integer, "fit" => Scale::Fit,
    "stretch" => Scale::Stretch);
enum_value!(PaddingColor, "padding color", "background" => PaddingColor::Background,
    "extend" => PaddingColor::Extend, "extend-always" => PaddingColor::ExtendAlways);
enum_value!(TextRender, "text render", "bitmap" => TextRender::Bitmap,
    "vector" => TextRender::Vector);

impl Value for String {
    fn parse(s: &str) -> Result<Self, String> {
        Ok(s.to_owned())
    }
    fn show(&self) -> String {
        self.clone()
    }
}

impl Value for u32 {
    fn parse(s: &str) -> Result<Self, String> {
        s.parse().map_err(|_| format!("invalid number {s:?}"))
    }
    fn show(&self) -> String {
        self.to_string()
    }
}

impl Value for NonZeroU32 {
    fn parse(s: &str) -> Result<Self, String> {
        s.parse()
            .map_err(|_| format!("invalid number {s:?}; expected a positive integer"))
    }
    fn show(&self) -> String {
        self.to_string()
    }
}

impl Value for f32 {
    fn parse(s: &str) -> Result<Self, String> {
        match s.parse::<f32>() {
            Ok(v) if v.is_finite() && v > 0.0 => Ok(v),
            _ => Err(format!("invalid number {s:?}; expected a positive number")),
        }
    }
    fn show(&self) -> String {
        self.to_string()
    }
}

impl Value for Percent {
    fn parse(s: &str) -> Result<Self, String> {
        match s.parse::<u8>() {
            Ok(v) if v <= 100 => Ok(Percent(v)),
            _ => Err(format!("invalid percentage {s:?}; expected 0..100")),
        }
    }
    fn show(&self) -> String {
        self.0.to_string()
    }
}

impl Value for Color {
    fn parse(s: &str) -> Result<Self, String> {
        let hex = s.strip_prefix('#').unwrap_or(s);
        let n = (hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| u32::from_str_radix(hex, 16).ok())
            .flatten()
            .ok_or_else(|| format!("invalid color {s:?}; expected #rrggbb"))?;
        let [_, r, g, b] = n.to_be_bytes();
        Ok(Color { r, g, b })
    }
    fn show(&self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

impl Value for Mount {
    fn parse(s: &str) -> Result<Self, String> {
        let err = || format!("invalid mount {s:?}; expected <letter>:<path>");
        let (letter, path) = s.split_once(':').ok_or_else(err)?;
        let mut chars = letter.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if c.is_ascii_alphabetic() && !path.is_empty() => Ok(Mount {
                letter: c.to_ascii_uppercase(),
                path: path.into(),
            }),
            _ => Err(err()),
        }
    }
    fn show(&self) -> String {
        format!("{}:{}", self.letter, self.path.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors() {
        let c = Color::parse("#1a2B3c").unwrap();
        assert_eq!(
            c,
            Color {
                r: 0x1A,
                g: 0x2B,
                b: 0x3C
            }
        );
        assert_eq!(Color::parse("ffffff").unwrap().show(), "#ffffff");
        for bad in ["#fff", "#12345g", "", "#1234567", "#+12345"] {
            assert!(Color::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn enums_and_bools() {
        assert_eq!(Scale::parse("fit"), Ok(Scale::Fit));
        assert_eq!(Scale::Stretch.show(), "stretch");
        assert!(
            Scale::parse("Fit")
                .unwrap_err()
                .contains("integer, fit, stretch")
        );
        assert_eq!(
            PaddingColor::parse("extend-always"),
            Ok(PaddingColor::ExtendAlways)
        );
        assert_eq!(TextRender::parse("vector"), Ok(TextRender::Vector));
        assert_eq!(bool::parse("false"), Ok(false));
        assert!(bool::parse("yes").is_err());
    }

    #[test]
    fn numbers() {
        assert_eq!(u32::parse("12"), Ok(12));
        assert!(u32::parse("-1").is_err());
        assert!(NonZeroU32::parse("0").is_err());
        assert_eq!(f32::parse("13.5"), Ok(13.5));
        assert!(f32::parse("nan").is_err());
        assert_eq!(Percent::parse("100"), Ok(Percent(100)));
        assert!(Percent::parse("101").is_err());
    }

    #[test]
    fn mounts() {
        let m = Mount::parse("c:/dos/apps").unwrap();
        assert_eq!((m.letter, m.path), ('C', PathBuf::from("/dos/apps")));
        // Windows paths keep their own drive colon.
        let m = Mount::parse(r"D:C:\dos").unwrap();
        assert_eq!(m.path, PathBuf::from(r"C:\dos"));
        for bad in ["C", "C:", "CD:/x", "1:/x"] {
            assert!(Mount::parse(bad).is_err(), "{bad}");
        }
    }
}
