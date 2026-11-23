//! Command line parsing and config loading.

use std::path::{Path, PathBuf};

use dosy_config::{Config, Diagnostic, Scale};
use dosy_core::video::Rgb8;
use dosy_render::{RenderConfig, ScaleMode};

use crate::emu::{EmuSetup, MountSource};

pub const USAGE: &str = "\
usage: dosy [options] [PATH]

  PATH                 directory, .zip or file to mount as C:
  -c <command>         run <command> at boot (repeatable)
  -V, --version        print version and exit
  -h, --help           print this help and exit
  --config <file>      use <file> instead of the default config file
  --<key>=<value>      override a config key (e.g. --cycles=max)";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Cli {
    pub version: bool,
    pub help: bool,
    /// Replaces the default config file.
    pub config: Option<PathBuf>,
    /// `--key=value` overrides, in order, for [`Config::apply_cli`].
    pub overrides: Vec<String>,
    /// Mounted as C:.
    pub path: Option<PathBuf>,
    /// `-c` commands, in order.
    pub commands: Vec<String>,
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
            "-c" => cli
                .commands
                .push(args.next().ok_or("-c needs a command argument")?),
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
                } else if cli.path.is_some() {
                    return Err(format!("unexpected argument {arg:?}"));
                } else {
                    cli.path = Some(arg.into());
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

/// Classifies a host path for mounting.
pub fn mount_source(p: &Path) -> Result<MountSource, String> {
    let meta = std::fs::metadata(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let is_zip = p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip"));
    Ok(if meta.is_dir() {
        MountSource::Dir(p.to_owned())
    } else if is_zip {
        MountSource::Zip(p.to_owned())
    } else {
        MountSource::File(p.to_owned())
    })
}

/// Emulation startup settings: config `mount`/`autoexec`, then `PATH` as
/// C: (replacing a configured C:) followed by `C:` and the `-c` commands.
/// Unusable config mounts are skipped with a warning (config never aborts);
/// an unusable `PATH` is an error.
pub fn emu_setup(
    cfg: &Config,
    cli: &Cli,
    rtc_unix_secs: i64,
) -> Result<(EmuSetup, Vec<String>), String> {
    let mut mounts = Vec::new();
    let mut warnings = Vec::new();
    for m in &cfg.mount {
        match mount_source(&m.path) {
            Ok(src) => mounts.push((m.letter, src)),
            Err(e) => warnings.push(format!("mount {}: skipped: {e}", m.letter)),
        }
    }
    let mut autoexec = cfg.autoexec.clone();
    if let Some(p) = &cli.path {
        mounts.retain(|(l, _)| *l != 'C');
        mounts.push(('C', mount_source(p)?));
        autoexec.push("C:".into());
    }
    autoexec.extend(cli.commands.iter().cloned());
    let setup = EmuSetup {
        cycles_per_ms: cfg.cycles.per_ms(),
        rtc_unix_secs,
        mounts,
        autoexec,
    };
    Ok((setup, warnings))
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
        let cli = parse([
            "-V",
            "--config",
            "my.conf",
            "--scale=fit",
            "-c",
            "dir",
            "--crt",
            "prog",
            "-c",
            "x",
        ])
        .unwrap();
        assert_eq!(
            cli,
            Cli {
                version: true,
                help: false,
                config: Some("my.conf".into()),
                overrides: vec!["--scale=fit".into(), "--crt".into()],
                path: Some("prog".into()),
                commands: vec!["dir".into(), "x".into()],
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
        assert!(parse(["-c"]).is_err());
        assert!(parse(["a", "b"]).is_err());
        // A lone "-" is positional.
        assert_eq!(parse(["-"]).unwrap().path, Some("-".into()));
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
    fn emu_setup_merges_config_and_cli() {
        let dir = std::env::temp_dir().join(format!("dosy-setup-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let zip = dir.join("disk.ZIP");
        std::fs::write(&zip, b"").unwrap();
        let d = dir.display();
        let cli = parse([
            format!("--mount=C:{d}"),
            format!("--mount=D:{d}"),
            "--autoexec=echo hi".into(),
            "--cycles=max".into(),
            zip.display().to_string(),
            "-c".into(),
            "run".into(),
        ])
        .unwrap();
        let (cfg, diags) = load_config(&Cli {
            config: Some(dir.join("absent")),
            ..Default::default()
        });
        assert_eq!(diags.len(), 1, "missing --config file warns");
        let mut cfg = cfg;
        let mut diags = Vec::new();
        cfg.apply_cli(&cli.overrides, &mut diags);
        assert!(diags.is_empty(), "{diags:?}");

        let (s, warnings) = emu_setup(&cfg, &cli, 42).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(s.cycles_per_ms, None);
        assert_eq!(s.rtc_unix_secs, 42);
        assert_eq!(
            s.mounts,
            [
                ('D', MountSource::Dir(dir.clone())),
                ('C', MountSource::Zip(zip.clone()))
            ]
        );
        assert_eq!(s.autoexec, ["echo hi", "C:", "run"]);

        let missing = parse([dir.join("nope").display().to_string()]).unwrap();
        assert!(emu_setup(&Config::default(), &missing, 0).is_err());
        // A bad config mount only warns.
        let mut bad = Config::default();
        bad.set("mount", &format!("E:{}", dir.join("nope").display()))
            .unwrap();
        let (s, warnings) = emu_setup(&bad, &Cli::default(), 0).unwrap();
        assert!(s.mounts.is_empty());
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            mount_source(&dir.join("disk.ZIP")).unwrap(),
            MountSource::Zip(zip.clone())
        );
        std::fs::remove_dir_all(&dir).ok();
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
