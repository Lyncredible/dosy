use crate::bus::Bus;
use crate::machine::{Machine, MachineConfig, RunExit};
use crate::vfs::MemDrive;

/// `mov ax,0E48h ; int 10h ; mov al,'I' ; int 10h ; mov ax,4C00h ; int 21h`
const HI_COM: &[u8] = &[
    0xB8, 0x48, 0x0E, 0xCD, 0x10, 0xB0, 0x49, 0xCD, 0x10, 0xB8, 0x00, 0x4C, 0xCD, 0x21,
];

fn boot(c: Option<MemDrive>, autoexec: &[&str]) -> Machine {
    let mut m = Machine::new(MachineConfig::default());
    if let Some(c) = c {
        m.mount('c', Box::new(c));
    }
    let lines: Vec<String> = autoexec.iter().map(|s| s.to_string()).collect();
    m.start_shell(&lines);
    settle(&mut m);
    m
}

/// Run 300 ms of emulated time; the shell must not exit.
fn settle(m: &mut Machine) {
    assert_eq!(m.run_for(3000 * 300), RunExit::Budget);
}

fn screen(m: &mut Machine) -> Vec<String> {
    (0..25u32)
        .map(|r| {
            let s: String = (0..80u32)
                .map(|c| m.hw.read8(0xB8000 + (r * 80 + c) * 2) as char)
                .collect();
            s.trim_end().to_string()
        })
        .collect()
}

fn has_line(m: &mut Machine, want: &str) -> bool {
    screen(m).iter().any(|l| l == want)
}

fn scancode(c: char) -> (bool, u8) {
    const ROWS: [(&str, u8); 3] = [("qwertyuiop", 0x10), ("asdfghjkl", 0x1E), ("zxcvbnm", 0x2C)];
    for (row, base) in ROWS {
        if let Some(i) = row.find(c) {
            return (false, base + i as u8);
        }
    }
    match c {
        '1'..='9' => (false, c as u8 - b'1' + 2),
        '0' => (false, 0x0B),
        ' ' => (false, 0x39),
        '.' => (false, 0x34),
        '\\' => (false, 0x2B),
        '=' => (false, 0x0D),
        ':' => (true, 0x27),
        '\r' => (false, 0x1C),
        _ => panic!("no scancode for {c:?}"),
    }
}

fn type_line(m: &mut Machine, s: &str) {
    for c in s.chars() {
        let (shift, code) = scancode(c);
        let mut bytes = vec![];
        if shift {
            bytes.push(0x2A);
        }
        bytes.extend([code, code | 0x80]);
        if shift {
            bytes.push(0xAA);
        }
        m.key_event(&bytes);
        // Let the keyboard drain (1 ms per byte) so the 15-key buffer
        // never overflows.
        assert_eq!(m.run_for(3000 * 10), RunExit::Budget);
    }
    settle(m);
}

fn drive_c() -> MemDrive {
    let mut c = MemDrive::new();
    c.add_file("\\README.TXT", b"readme text\r\n\x1Aignored");
    c.add_file("\\HI.COM", HI_COM);
    c.add_file("\\APPS\\TOOL.COM", HI_COM);
    c
}

#[test]
fn prompt_echo_dir_program_and_exit() {
    let mut m = boot(Some(drive_c()), &[]);
    assert!(has_line(&mut m, "C:\\>"), "{:#?}", screen(&mut m));
    type_line(&mut m, "echo hello\r");
    assert!(has_line(&mut m, "hello"));
    type_line(&mut m, "dir\r");
    let s = screen(&mut m);
    assert!(
        s.iter().any(|l| l.starts_with("README   TXT        21")),
        "{s:#?}"
    );
    assert!(
        s.iter().any(|l| l.starts_with("APPS         <DIR>")),
        "{s:#?}"
    );
    assert!(s.iter().any(|l| l.contains("2 file(s)")), "{s:#?}");
    // A program prints and exits back to the prompt.
    type_line(&mut m, "hi\r");
    assert!(has_line(&mut m, "C:\\>hi"));
    assert!(has_line(&mut m, "HI"), "{:#?}", screen(&mut m));
    assert!(m.dos.processes.is_empty());
    type_line(&mut m, "exit");
    m.key_event(&[0x1C, 0x9C]);
    assert_eq!(m.run_for(3000 * 300), RunExit::Exited(0));
    assert_eq!(m.run_for(1000), RunExit::Exited(0), "sticky");
}

#[test]
fn cd_type_path_and_errors() {
    let mut m = boot(Some(drive_c()), &[]);
    type_line(&mut m, "type readme.txt\r");
    assert!(has_line(&mut m, "readme text"));
    assert!(!screen(&mut m).iter().any(|l| l.contains("ignored")));
    type_line(&mut m, "cd apps\r");
    type_line(&mut m, "cd\r");
    assert!(has_line(&mut m, "C:\\APPS"));
    type_line(&mut m, "cd\\\r");
    type_line(&mut m, "nosuch\r");
    assert!(has_line(&mut m, "Bad command or file name"));
    type_line(&mut m, "tool\r");
    assert!(has_line(&mut m, "Bad command or file name"));
    type_line(&mut m, "path c:\\apps\r");
    type_line(&mut m, "path\r");
    assert!(has_line(&mut m, "PATH=C:\\APPS"));
    type_line(&mut m, "tool\r");
    assert!(has_line(&mut m, "HI"), "{:#?}", screen(&mut m));
    type_line(&mut m, "d:\r");
    assert!(has_line(&mut m, "Invalid drive specification"));
}

#[test]
fn autoexec_runs_on_z_and_set_works() {
    let mut m = boot(None, &["echo auto", "set foo=bar", "set"]);
    let s = screen(&mut m);
    assert!(s.contains(&"Z:\\>echo auto".to_string()), "{s:#?}");
    assert!(s.contains(&"auto".to_string()));
    assert!(s.contains(&"FOO=bar".to_string()));
    assert!(s.contains(&"COMSPEC=Z:\\COMMAND.COM".to_string()));
    assert_eq!(s.last().map(String::as_str), Some(""));
    assert!(s.contains(&"Z:\\>".to_string()));
    type_line(&mut m, "set foo=\r");
    type_line(&mut m, "set\r");
    let after: Vec<String> = screen(&mut m);
    let last_set = after.iter().rposition(|l| l == "Z:\\>set").unwrap();
    assert!(!after[last_set..].iter().any(|l| l.starts_with("FOO=")));
}

#[test]
fn mount_host_directory() {
    let dir = std::env::temp_dir().join(format!("dosy-shell-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), b"host file").unwrap();
    let mount = format!("mount d \"{}\"", dir.display());
    let mut m = boot(None, &[&mount, "d:", "type a.txt", "mount", "mount -u d"]);
    let s = screen(&mut m);
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(s.contains(&"host file".to_string()), "{s:#?}");
    assert!(s.iter().any(|l| l.starts_with("Drive D: ")), "{s:#?}");
    assert!(s.contains(&"Drive D: unmounted".to_string()), "{s:#?}");
    assert!(s.contains(&"Z:\\>".to_string()), "back on Z: after unmount");
}

/// COM that bumps the byte at 0000:04F0 and exits with `code`.
fn counter_com(code: u8) -> Vec<u8> {
    vec![
        0x31, 0xC0, // xor ax, ax
        0x8E, 0xD8, // mov ds, ax
        0xFE, 0x06, 0xF0, 0x04, // inc byte [04F0h]
        0xB8, code, 0x4C, // mov ax, 4C00h + code
        0xCD, 0x21, // int 21h
    ]
}

fn count(m: &mut Machine) -> u8 {
    m.hw.read8(0x4F0)
}

#[test]
fn launcher_batch_runs_programs_in_sequence() {
    let mut c = MemDrive::new();
    c.add_file("\\A.COM", &counter_com(0));
    c.add_file("\\B.COM", &counter_com(0));
    c.add_file(
        "\\GAME.BAT",
        b"@echo off\r\ncls\r\na\r\nb\r\ncls\r\necho\t\tThanks for playing\r\necho\tBye\r\n",
    );
    let mut m = boot(Some(c), &[]);
    type_line(&mut m, "game\r");
    assert_eq!(count(&mut m), 2, "both programs ran");
    let s = screen(&mut m);
    // The first separator after ECHO is eaten, as in DOS; the next tab
    // expands to column 8.
    assert_eq!(s[0], "        Thanks for playing", "{s:#?}");
    assert_eq!(s[1], "Bye", "{s:#?}");
    assert!(s.contains(&"C:\\>".to_string()), "{s:#?}");
    assert!(
        !s.iter().any(|l| l.contains(">a") || l.contains(">b")),
        "echo off"
    );
    assert!(m.dos.processes.is_empty());
}

#[test]
fn goto_if_for_call_shift_and_errorlevel() {
    let mut c = MemDrive::new();
    c.add_file("\\E3.COM", &counter_com(3));
    c.add_file(
        "\\T.BAT",
        b"@echo off\r\n\
          :loop\r\n\
          echo arg %1\r\n\
          shift\r\n\
          if not \"%1\"==\"\" goto loop\r\n\
          e3\r\n\
          if errorlevel 3 echo level3\r\n\
          if errorlevel 4 echo level4\r\n\
          if exist t.bat echo found\r\n\
          if not exist nope.txt echo missing\r\n\
          if exist *.com echo coms\r\n\
          for %%f in (x y) do echo item %%f\r\n\
          for %%f in (*.com) do echo has %%f\r\n\
          set who=world\r\n\
          echo hello %who% 100%%\r\n\
          call sub one\r\n\
          echo back\r\n\
          goto end\r\n\
          echo skipped\r\n\
          :end\r\n",
    );
    c.add_file("\\SUB.BAT", b"echo sub %1\r\n");
    let mut m = boot(Some(c), &[]);
    type_line(&mut m, "t p q\r");
    let s = screen(&mut m);
    let want = [
        "arg p",
        "arg q",
        "level3",
        "found",
        "missing",
        "coms",
        "item x",
        "item y",
        "has E3.COM",
        "hello world 100%",
        "sub one",
        "back",
    ];
    let got: Vec<&str> = s
        .iter()
        .map(String::as_str)
        .filter(|l| !l.is_empty() && !l.starts_with("C:\\>"))
        .collect();
    assert_eq!(got, want, "{s:#?}");
    assert_eq!(m.shell.errorlevel(), 3);
    assert_eq!(count(&mut m), 1);
}

#[test]
fn batch_from_autoexec_then_next_line_and_echo_on() {
    let mut c = MemDrive::new();
    c.add_file("\\T2.BAT", b"echo in bat\r\nrem nothing\r\n");
    let mut m = boot(Some(c), &["t2", "echo after"]);
    let s = screen(&mut m);
    let pos = |w: &str| {
        s.iter()
            .position(|l| l == w)
            .unwrap_or_else(|| panic!("{w}: {s:#?}"))
    };
    // Echo on: batch lines are shown after the prompt.
    assert!(pos("C:\\>echo in bat") < pos("in bat"));
    assert!(pos("C:\\>rem nothing") < pos("C:\\>echo after"));
    assert!(pos("in bat") < pos("after"));
}

#[test]
fn pause_waits_for_a_key() {
    let mut c = MemDrive::new();
    c.add_file("\\P.BAT", b"@echo off\r\npause\r\necho done\r\n");
    let mut m = boot(Some(c), &["p"]);
    assert!(has_line(&mut m, "Press any key to continue . . ."));
    assert!(!has_line(&mut m, "done"));
    type_line(&mut m, " ");
    assert!(has_line(&mut m, "done"), "{:#?}", screen(&mut m));
}
