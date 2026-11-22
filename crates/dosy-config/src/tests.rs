use super::*;
use std::ffi::OsString;

fn parse(text: &str) -> (Config, Vec<Diagnostic>) {
    let mut c = Config::default();
    let mut d = Vec::new();
    c.load_str(text, None, &mut d);
    (c, d)
}

/// Fresh temp dir per test.
fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("dosy-config-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn every_key_round_trips() {
    let text = "\
cycles = 12000
mount = C:/dos
scale = stretch
aspect = false
crt = true
theme = \"Builtin Dark\"
background = #102030
window-padding-x = 4
window-padding-y = 0
window-padding-color = extend
text-render = vector
font-family = JetBrains Mono
font-size = 14.5
audio = false
audio-volume = 40
autoexec = dir
keybind = ctrl+enter=toggle_fullscreen
";
    let (c, d) = parse(text);
    assert_eq!(d, vec![]);
    assert_eq!(c.cycles.get(), 12000);
    assert_eq!(c.scale, Scale::Stretch);
    assert!(!c.aspect && c.crt && !c.audio);
    assert_eq!(c.theme, "Builtin Dark");
    assert_eq!(
        c.background,
        Color {
            r: 0x10,
            g: 0x20,
            b: 0x30
        }
    );
    assert_eq!((c.window_padding_x, c.window_padding_y), (4, 0));
    assert_eq!(c.window_padding_color, PaddingColor::Extend);
    assert_eq!(c.text_render, TextRender::Vector);
    assert_eq!(c.font_family, "JetBrains Mono");
    assert_eq!(c.font_size, 14.5);
    assert_eq!(c.audio_volume, Percent(40));
    assert_eq!(c.keybind, ["ctrl+enter=toggle_fullscreen"]);

    // show() output parses back to the same config.
    let dumped: String = c
        .show()
        .iter()
        .map(|(k, v)| format!("{k} = {v}\n"))
        .collect();
    assert_eq!(parse(&dumped).0, c);
    assert_eq!(Config::default().show().len(), Config::KEYS.len() - 3); // 3 empty lists
}

#[test]
fn lists_append_and_empty_value_resets() {
    let (c, d) = parse("mount = C:/a\nmount = d:/b\nautoexec = x\nautoexec =\nautoexec = y\n");
    assert_eq!(d, vec![]);
    let letters: Vec<char> = c.mount.iter().map(|m| m.letter).collect();
    assert_eq!(letters, ['C', 'D']);
    assert_eq!(c.autoexec, ["y"]);

    let (c, _) = parse("cycles = 5\ncycles =\nscale = fit\nscale = \"\"\n");
    assert_eq!(c, Config::default());
}

#[test]
fn diagnostics_have_line_numbers_and_never_abort() {
    let (c, d) = parse("# comment\n\nbogus = 1\nscale = huge\njunk line\ncrt = true\n");
    assert!(c.crt, "later lines still apply");
    let lines: Vec<usize> = d.iter().map(|d| d.line).collect();
    assert_eq!(lines, [3, 4, 5]);
    assert!(d[0].message.contains("unknown key"));
    assert!(d[1].message.contains("invalid scale"));
    assert_eq!(c.scale, Scale::Integer, "bad value keeps the old one");
}

#[test]
fn includes_optional_and_cycles() {
    let dir = tmp("include");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(
        dir.join("main"),
        "config-file = sub/a\nconfig-file = ?missing\nconfig-file = gone\ncycles = 100\n",
    )
    .unwrap();
    // Relative to the including file; included after main, so it wins.
    std::fs::write(dir.join("sub/a"), "cycles = 200\nconfig-file = ../main\n").unwrap();

    let mut c = Config::default();
    let mut d = Vec::new();
    c.load_file(&dir.join("main"), false, &mut d);
    assert_eq!(c.cycles.get(), 200);
    assert_eq!(d.len(), 2, "{d:?}");
    assert!(d[0].message.contains("cycle"));
    assert_eq!(
        (d[0].file.as_deref(), d[0].line),
        (Some(&*dir.join("sub/a")), 2)
    );
    assert!(d[1].message.contains("cannot read"));
    assert_eq!(
        (d[1].file.as_deref(), d[1].line),
        (Some(&*dir.join("main")), 3)
    );

    // Optional top-level file that does not exist is silent.
    let mut d = Vec::new();
    c.load_file(&dir.join("nope"), true, &mut d);
    assert_eq!(d, vec![]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_overrides_in_order() {
    let (mut c, _) = parse("cycles = 100\nmount = C:/a\n");
    let mut d = Vec::new();
    let rest = c.apply_cli(
        [
            "--cycles=200",
            "game.exe",
            "--cycles=300",
            "--mount=",
            "--mount=D:/b",
            "--crt",
            "--nope=1",
        ],
        &mut d,
    );
    assert_eq!(rest, ["game.exe"]);
    assert_eq!(c.cycles.get(), 300);
    assert_eq!(c.mount.len(), 1);
    assert_eq!(c.mount[0].letter, 'D');
    assert!(c.crt);
    assert_eq!(d.len(), 1);
    assert_eq!((d[0].file.as_ref(), d[0].line), (None, 7));

    let dir = tmp("cli");
    std::fs::write(dir.join("extra"), "scale = fit\n").unwrap();
    let arg = format!("--config-file={}", dir.join("extra").display());
    c.apply_cli([arg.as_str(), "--scale=stretch"], &mut d);
    assert_eq!(c.scale, Scale::Stretch);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn platform_paths() {
    fn p(os: &str, vars: &[(&str, &str)]) -> Option<PathBuf> {
        default_path_for(os, |k| {
            vars.iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| OsString::from(v))
        })
    }
    assert_eq!(
        p("windows", &[("APPDATA", "C:/AppData")]),
        Some(PathBuf::from("C:/AppData/dosy/config"))
    );
    assert_eq!(
        p("macos", &[("HOME", "/h")]),
        Some(PathBuf::from("/h/Library/Application Support/dosy/config"))
    );
    assert_eq!(
        p("unix", &[("HOME", "/h"), ("XDG_CONFIG_HOME", "/x")]),
        Some(PathBuf::from("/x/dosy/config"))
    );
    assert_eq!(
        p("unix", &[("HOME", "/h")]),
        Some(PathBuf::from("/h/.config/dosy/config"))
    );
    assert_eq!(p("unix", &[]), None);
}
