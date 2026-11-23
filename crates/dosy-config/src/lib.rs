//! Ghostty-style plain-text configuration (`key = value`) for dosy.
//! See `docs/ARCHITECTURE.md` ("Configuration").
//!
//! Syntax: one `key = value` per line, `#` comment lines, optional double
//! quotes around the value, repeated list keys append, an empty value resets
//! the key to its default. `config-file = path` includes another file
//! (relative to the including file; `?path` = optional) after the current
//! file is done, as in Ghostty. Problems become [`Diagnostic`]s, never errors.

mod value;

use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

pub use value::{Color, CpuKind, Cycles, Mount, PaddingColor, Percent, Scale, TextRender, Value};

/// Crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// File name of the config file inside the platform config directory.
pub const CONFIG_FILE_NAME: &str = "config";

/// A warning about one config line or CLI argument.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// Source file; `None` for CLI arguments and strings.
    pub file: Option<PathBuf>,
    /// 1-based line (or CLI argument index).
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.file {
            Some(p) => write!(f, "{}:{}: {}", p.display(), self.line, self.message),
            None => write!(f, "argument {}: {}", self.line, self.message),
        }
    }
}

fn set_scalar<T: Value>(field: &mut T, v: &str) -> Result<(), String> {
    *field = T::parse(v)?;
    Ok(())
}
fn set_list<T: Value>(field: &mut Vec<T>, v: &str) -> Result<(), String> {
    field.push(T::parse(v)?);
    Ok(())
}
fn show_scalar<T: Value>(field: &T) -> Vec<String> {
    vec![field.show()]
}
fn show_list<T: Value>(field: &[T]) -> Vec<String> {
    field.iter().map(Value::show).collect()
}

/// The key table: the one place that lists every key, its field, type,
/// default and whether it is a list.
macro_rules! keys {
    ($($(#[$doc:meta])* $name:literal => $field:ident: $ty:ty = $default:expr, $set:ident, $show:ident;)*) => {
        /// Typed configuration. Field docs give the key name.
        #[derive(Clone, Debug, PartialEq)]
        pub struct Config {
            $($(#[$doc])* pub $field: $ty,)*
        }

        impl Default for Config {
            fn default() -> Self {
                Self { $($field: $default,)* }
            }
        }

        impl Config {
            /// Every key name, in table order.
            pub const KEYS: &[&str] = &[$($name),*];

            /// Set one key. An empty value resets it to its default.
            /// `config-file` is handled by the loaders, not here.
            pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
                match key {
                    $($name => if value.is_empty() {
                        self.$field = $default;
                        Ok(())
                    } else {
                        $set(&mut self.$field, value)
                    },)*
                    _ => Err(format!("unknown key {key:?}")),
                }
            }

            /// `(key, value)` per value, in table order, for dumping.
            pub fn show(&self) -> Vec<(&'static str, String)> {
                let mut out = Vec::new();
                $(for v in $show(&self.$field) { out.push(($name, v)); })*
                out
            }
        }
    };
}

keys! {
    /// `cycles`: instructions per emulated millisecond, or `max`.
    "cycles" => cycles: Cycles = Cycles::Fixed(NonZeroU32::new(3000).unwrap()), set_scalar,
        show_scalar;
    /// `cpu`: `8086`, `186` or `386`.
    "cpu" => cpu: CpuKind = CpuKind::I386, set_scalar, show_scalar;
    /// `mount` (list): `C:path`.
    "mount" => mount: Vec<Mount> = Vec::new(), set_list, show_list;
    /// `autoexec` (list): shell lines run at boot.
    "autoexec" => autoexec: Vec<String> = Vec::new(), set_list, show_list;
    /// `scale`.
    "scale" => scale: Scale = Scale::Integer, set_scalar, show_scalar;
    /// `aspect`: correct to the display aspect (usually 4:3).
    "aspect" => aspect: bool = true, set_scalar, show_scalar;
    /// `crt`: CRT shader.
    "crt" => crt: bool = false, set_scalar, show_scalar;
    /// `theme`: theme name; empty = built-in.
    "theme" => theme: String = String::new(), set_scalar, show_scalar;
    /// `background`.
    "background" => background: Color = Color::default(), set_scalar, show_scalar;
    /// `window-padding-x` in points.
    "window-padding-x" => window_padding_x: u32 = 2, set_scalar, show_scalar;
    /// `window-padding-y` in points.
    "window-padding-y" => window_padding_y: u32 = 2, set_scalar, show_scalar;
    /// `window-padding-color`.
    "window-padding-color" => window_padding_color: PaddingColor = PaddingColor::Background,
        set_scalar, show_scalar;
    /// `text-render`.
    "text-render" => text_render: TextRender = TextRender::Bitmap, set_scalar, show_scalar;
    /// `font-family` for vector text; empty = default.
    "font-family" => font_family: String = String::new(), set_scalar, show_scalar;
    /// `font-size` in points.
    "font-size" => font_size: f32 = 16.0, set_scalar, show_scalar;
    /// `audio`: enable sound output.
    "audio" => audio: bool = true, set_scalar, show_scalar;
    /// `audio-volume` in percent.
    "audio-volume" => audio_volume: Percent = Percent(100), set_scalar, show_scalar;
    /// `keybind` (list): raw `trigger=action`, parsed by task 042.
    "keybind" => keybind: Vec<String> = Vec::new(), set_list, show_list;
}

/// The include key, handled by the loaders.
pub const CONFIG_FILE_KEY: &str = "config-file";

impl Config {
    /// Defaults plus the platform config file (if any). Missing file is fine.
    pub fn load_default() -> (Config, Vec<Diagnostic>) {
        let mut cfg = Config::default();
        let mut diags = Vec::new();
        if let Some(p) = default_path() {
            cfg.load_file(&p, true, &mut diags);
        }
        (cfg, diags)
    }

    /// Load `path` and its includes. `optional`: a missing file is silent.
    pub fn load_file(&mut self, path: &Path, optional: bool, diags: &mut Vec<Diagnostic>) {
        self.load_file_inner(path, optional, &mut Vec::new(), diags, None);
    }

    /// Parse config text. Relative includes resolve against `file`'s
    /// directory (or the current directory when `None`).
    pub fn load_str(&mut self, text: &str, file: Option<&Path>, diags: &mut Vec<Diagnostic>) {
        self.load_str_inner(text, file, &mut Vec::new(), diags);
    }

    /// Apply `--key=value` arguments in order (`--key` alone means `true`).
    /// `--config-file=path` loads immediately, relative to the current
    /// directory. Other arguments are returned untouched.
    pub fn apply_cli<I, S>(&mut self, args: I, diags: &mut Vec<Diagnostic>) -> Vec<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut rest = Vec::new();
        for (i, arg) in args.into_iter().enumerate() {
            let arg = arg.as_ref();
            let Some(kv) = arg.strip_prefix("--") else {
                rest.push(arg.to_owned());
                continue;
            };
            let (key, value) = kv.split_once('=').unwrap_or((kv, "true"));
            let at = |message| Diagnostic {
                file: None,
                line: i + 1,
                message,
            };
            if key == CONFIG_FILE_KEY {
                let (p, optional) = include_target(value);
                self.load_file(Path::new(p), optional, diags);
            } else if let Err(e) = self.set(key, unquote(value)) {
                diags.push(at(e));
            }
        }
        rest
    }

    fn load_file_inner(
        &mut self,
        path: &Path,
        optional: bool,
        stack: &mut Vec<PathBuf>,
        diags: &mut Vec<Diagnostic>,
        from: Option<(&Path, usize)>,
    ) {
        let at = |message| match from {
            Some((f, line)) => Diagnostic {
                file: Some(f.to_owned()),
                line,
                message,
            },
            None => Diagnostic {
                file: Some(path.to_owned()),
                line: 0,
                message,
            },
        };
        let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
        if stack.contains(&canon) {
            diags.push(at(format!("config-file cycle: {}", path.display())));
            return;
        }
        match std::fs::read_to_string(path) {
            Ok(text) => {
                stack.push(canon);
                self.load_str_inner(&text, Some(path), stack, diags);
                stack.pop();
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && optional => {}
            Err(e) => diags.push(at(format!("cannot read {}: {e}", path.display()))),
        }
    }

    fn load_str_inner(
        &mut self,
        text: &str,
        file: Option<&Path>,
        stack: &mut Vec<PathBuf>,
        diags: &mut Vec<Diagnostic>,
    ) {
        let mut includes = Vec::new();
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        for (i, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let at = |message| Diagnostic {
                file: file.map(Path::to_owned),
                line: i + 1,
                message,
            };
            let Some((key, value)) = line.split_once('=') else {
                diags.push(at("expected `key = value`".into()));
                continue;
            };
            let (key, value) = (key.trim(), unquote(value.trim()));
            if key == CONFIG_FILE_KEY {
                if !value.is_empty() {
                    includes.push((i + 1, value.to_owned()));
                }
            } else if let Err(e) = self.set(key, value) {
                diags.push(at(e));
            }
        }
        let dir = file.and_then(Path::parent).unwrap_or(Path::new(""));
        for (line, inc) in includes {
            let (p, optional) = include_target(&inc);
            let from = file.map(|f| (f, line));
            self.load_file_inner(&dir.join(p), optional, stack, diags, from);
        }
    }
}

fn unquote(v: &str) -> &str {
    v.strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(v)
}

/// `?path` → `(path, optional)`.
fn include_target(v: &str) -> (&str, bool) {
    match v.strip_prefix('?') {
        Some(p) => (p, true),
        None => (v, false),
    }
}

/// Platform config file path, from environment variables only.
pub fn default_path() -> Option<PathBuf> {
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "unix"
    };
    default_path_for(os, |k| std::env::var_os(k).filter(|v| !v.is_empty()))
}

fn default_path_for(os: &str, env: impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<PathBuf> {
    let base = match os {
        "windows" => PathBuf::from(env("APPDATA")?),
        "macos" => PathBuf::from(env("HOME")?).join("Library/Application Support"),
        _ => match env("XDG_CONFIG_HOME") {
            Some(x) => PathBuf::from(x),
            None => PathBuf::from(env("HOME")?).join(".config"),
        },
    };
    Some(base.join("dosy").join(CONFIG_FILE_NAME))
}

#[cfg(test)]
mod tests;
