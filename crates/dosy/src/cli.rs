//! Command line parsing and config loading.

use std::path::PathBuf;

use dosy_config::{Config, Diagnostic, Scale};
use dosy_core::video::Rgb8;
use dosy_render::{RenderConfig, ScaleMode};

pub const USAGE: &str = "\
usage: dosy [options]

  -V, --version        print version and exit
  -h, --help           print this help and exit
  --config <file>      use <file> instead of the default config file
  --<key>=<value>      override a config key (e.g. --scale=fit)";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Cli {
    pub version: bool,
    pub help: bool,
    /// Replaces the default config file.
    pub config: Option<PathBuf>,
    /// `--key=value` overrides, in order, for [`Config::apply_cli`].
    pub overrides: Vec<String>,
    /// Positional arguments (unused for now).
    pub rest: Vec<String>,
}

/// Parses arguments (without the program name).
pub fn parse<I, S>(args: I) -> Result<Cli, String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut cli = Cli::default();
    let mut args = args.into_iter().map(Into::into);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-V" | "--version" => cli.version = true,
            "-h" | "--help" => cli.help = true,
            "--config" => {
                let p = args.next().ok_or("--config needs a file argument")?;
                cli.config = Some(p.into());
            }
            _ => {
                if let Some(p) = arg.strip_prefix("--config=") {
                    cli.config = Some(p.into());
                } else if arg.starts_with("--") {
                    cli.overrides.push(arg);
                } else if arg.starts_with('-') && arg.len() > 1 {
                    return Err(format!("unknown option {arg:?}"));
                } else {
                    cli.rest.push(arg);
                }
            }
        }
    }
    Ok(cli)
}

/// Default config file (or `--config`), then CLI overrides.
pub fn load_config(cli: &Cli) -> (Config, Vec<Diagnostic>) {
    let (mut cfg, mut diags) = match &cli.config {
        Some(p) => {
            let mut cfg = Config::default();
            let mut diags = Vec::new();
            cfg.load_file(p, false, &mut diags);
            (cfg, diags)
        }
        None => Config::load_default(),
    };
    cfg.apply_cli(&cli.overrides, &mut diags);
    (cfg, diags)
}

/// Maps the app config to renderer settings. Padding is in points, scaled
/// by the window's `scale_factor`.
pub fn render_config(cfg: &Config, scale_factor: f64) -> RenderConfig {
    let pts = |p: u32| (p as f64 * scale_factor).round() as u32;
    RenderConfig {
        background: Rgb8 {
            r: cfg.background.r,
            g: cfg.background.g,
            b: cfg.background.b,
        },
        padding: (pts(cfg.window_padding_x), pts(cfg.window_padding_y)),
        scale: match cfg.scale {
            Scale::Integer => ScaleMode::Integer,
            Scale::Fit => ScaleMode::Fit,
            Scale::Stretch => ScaleMode::Stretch,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flags_config_and_overrides() {
        let cli = parse(["-V", "--config", "my.conf", "--scale=fit", "--crt", "prog"]).unwrap();
        assert_eq!(
            cli,
            Cli {
                version: true,
                help: false,
                config: Some("my.conf".into()),
                overrides: vec!["--scale=fit".into(), "--crt".into()],
                rest: vec!["prog".into()],
            }
        );
        assert_eq!(
            parse(["--config=a b.conf"]).unwrap().config,
            Some("a b.conf".into())
        );
        assert!(parse(["--help"]).unwrap().help);
        assert!(parse(["-h"]).unwrap().help);
        assert_eq!(parse(Vec::<String>::new()).unwrap(), Cli::default());
    }

    #[test]
    fn rejects_bad_args() {
        assert!(parse(["--config"]).is_err());
        assert!(parse(["-x"]).is_err());
        // A lone "-" is positional.
        assert_eq!(parse(["-"]).unwrap().rest, vec!["-".to_string()]);
    }

    #[test]
    fn overrides_reach_config() {
        let cli = parse(["--scale=stretch", "--background=#102030", "--nope=1"]).unwrap();
        let mut cfg = Config::default();
        let mut diags = Vec::new();
        cfg.apply_cli(&cli.overrides, &mut diags);
        assert_eq!(cfg.scale, Scale::Stretch);
        assert_eq!(diags.len(), 1); // unknown key warns, never fails
        let r = render_config(&cfg, 2.0);
        assert_eq!(r.scale, ScaleMode::Stretch);
        assert_eq!(
            r.background,
            Rgb8 {
                r: 0x10,
                g: 0x20,
                b: 0x30
            }
        );
        assert_eq!(r.padding, (4, 4)); // default 2pt at 2x
    }

    #[test]
    fn config_file_is_loaded() {
        let dir = std::env::temp_dir().join(format!("dosy-cli-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("config");
        std::fs::write(&file, "scale = fit\nwindow-padding-x = 7\n").unwrap();
        let cli = parse([
            "--config".to_string(),
            file.display().to_string(),
            "--window-padding-x=9".into(),
        ])
        .unwrap();
        let (cfg, diags) = load_config(&cli);
        std::fs::remove_dir_all(&dir).ok();
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(cfg.scale, Scale::Fit);
        assert_eq!(cfg.window_padding_x, 9); // CLI wins
    }
}
