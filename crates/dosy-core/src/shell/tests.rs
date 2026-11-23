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
