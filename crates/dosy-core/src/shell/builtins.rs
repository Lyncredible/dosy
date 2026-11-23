//! Command parsing, built-in commands and program lookup.

use std::path::Path;

use super::{
    Flow, Shell, State, Z, cwd_string, env_vars, error_text, find_file, get_env, out, outs, set_env,
};
use crate::bios::{bda, video};
use crate::bus::Bus;
use crate::cpu::Cpu;
use crate::dos::Dos;
use crate::dos::exec::ExecParams;
use crate::dos::path::Resolved;
use crate::vfs::{self, Attr, DirEntry, DosError, DosPattern, OpenMode};

/// Built-ins that may be glued to their argument (`CD\`, `ECHO.`).
const GLUED: &[&str] = &["CD", "CHDIR", "ECHO"];

/// Split a line into the command word and the rest (with its leading
/// separator, as DOS passes command tails).
pub(super) fn split_command(line: &[u8]) -> (String, Vec<u8>) {
    let line = line.trim_ascii();
    let end = line
        .iter()
        .position(|&c| c == b' ' || c == b'\t' || c == b'/')
        .unwrap_or(line.len());
    let (mut word, mut rest) = (&line[..end], &line[end..]);
    let upper = word.to_ascii_uppercase();
    for g in GLUED {
        let g = g.as_bytes();
        if upper.len() > g.len()
            && upper.starts_with(g)
            && matches!(upper[g.len()], b'\\' | b'.' | b'/')
        {
            (word, rest) = (&line[..g.len()], &line[g.len()..]);
            break;
        }
    }
    let word = word
        .iter()
        .map(|&b| b.to_ascii_uppercase() as char)
        .collect();
    (word, rest.to_vec())
}

/// Arguments: whitespace-separated, double quotes group.
pub(super) fn args(rest: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut cur = Vec::new();
    let mut quoted = false;
    for &c in rest {
        match c {
            b'"' => quoted = !quoted,
            b' ' | b'\t' if !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub(super) fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

impl Shell {
    /// Run one command line.
    pub(crate) fn run<B: Bus>(
        &mut self,
        dos: &mut Dos,
        cpu: &mut Cpu,
        bus: &mut B,
        line: &[u8],
    ) -> Flow {
        let line = line.strip_prefix(b"@").unwrap_or(line);
        let (word, rest) = split_command(line);
        let a = args(&rest);
        match word.as_str() {
            "" => {}
            w if w.len() == 2 && w.ends_with(':') => {
                let d = w.as_bytes()[0].wrapping_sub(b'A');
                if d < 26 && dos.is_mounted(d) {
                    dos.set_current_drive(bus, d);
                } else {
                    outs(bus, "Invalid drive specification\n");
                }
            }
            "ECHO" => self.echo(bus, &rest),
            "REM" => {}
            "PAUSE" => {
                outs(bus, "Press any key to continue . . .");
                self.state = State::Pause;
            }
            "CHOICE" => self.choice(bus, &rest),
            "GOTO" => self.goto(dos, bus, &a),
            "SHIFT" => self.shift(),
            "IF" => return self.if_cmd(dos, cpu, bus, &rest),
            "FOR" => self.for_cmd(dos, bus, &rest),
            "CALL" => {
                self.calling = true;
                let f = self.run(dos, cpu, bus, &rest);
                self.calling = false;
                return f;
            }
            "CLS" => {
                let mode = bda::read8(bus, bda::VIDEO_MODE) & 0x7F;
                video::set_mode(bus, mode);
            }
            "VER" => outs(bus, &format!("\ndosy {} (MS-DOS 5.00)\n\n", crate::VERSION)),
            "CD" | "CHDIR" => cd(dos, bus, &a),
            "DIR" => dir(dos, bus, &a),
            "TYPE" => type_file(dos, bus, &a),
            "SET" => set(dos, bus, &rest),
            "PATH" => path(dos, bus, &rest),
            "MOUNT" => self.mount(dos, bus, &a),
            "EXIT" => {
                dos.terminate(cpu, bus, 0, 0);
                return Flow::Exit;
            }
            _ => return self.run_program(dos, cpu, bus, &word, &rest),
        }
        Flow::Continue
    }

    fn mount<B: Bus>(&mut self, dos: &mut Dos, bus: &mut B, a: &[Vec<u8>]) {
        let letter = |s: &[u8]| match s {
            [l] | [l, b':'] if l.is_ascii_alphabetic() => Some(l.to_ascii_uppercase() - b'A'),
            _ => None,
        };
        match a {
            [] => {
                for d in 0..26u8 {
                    if dos.is_mounted(d) {
                        let what = self.mounts[d as usize]
                            .clone()
                            .unwrap_or_else(|| "internal".to_string());
                        outs(bus, &format!("Drive {}: {what}\n", (b'A' + d) as char));
                    }
                }
            }
            [flag, l] if flag.eq_ignore_ascii_case(b"-u") => match letter(l) {
                Some(d) if d == Z => outs(bus, "Drive Z: cannot be unmounted\n"),
                Some(d) if dos.is_mounted(d) => {
                    dos.drives[d as usize] = None;
                    self.mounts[d as usize] = None;
                    if dos.current_drive(bus) == d {
                        dos.set_current_drive(bus, Z);
                    }
                    outs(bus, &format!("Drive {}: unmounted\n", (b'A' + d) as char));
                }
                _ => outs(bus, "Drive not mounted\n"),
            },
            [l, p] => match letter(l) {
                Some(d) if d == Z => outs(bus, "Drive Z: is reserved\n"),
                Some(d) => {
                    let host = latin1(p);
                    match vfs::open_mount(Path::new(&host), &vfs::MountOptions::default()) {
                        Ok(drive) => {
                            dos.mount(d, drive);
                            dos.cwd[d as usize] = Default::default();
                            self.mounts[d as usize] = Some(host.clone());
                            outs(bus, &format!("Drive {}: is {host}\n", (b'A' + d) as char));
                        }
                        Err(e) => outs(bus, &format!("Cannot mount {host}: {e}\n")),
                    }
                }
                None => outs(bus, "Usage: MOUNT [X path | -u X]\n"),
            },
            _ => outs(bus, "Usage: MOUNT [X path | -u X]\n"),
        }
    }
}

fn cd<B: Bus>(dos: &mut Dos, bus: &mut B, a: &[Vec<u8>]) {
    match a.first().map(Vec::as_slice) {
        None => {
            let d = dos.current_drive(bus);
            out(bus, &cwd_string(dos, d));
            out(bus, b"\n");
        }
        Some([l, b':']) if l.is_ascii_alphabetic() => {
            let d = l.to_ascii_uppercase() - b'A';
            if dos.is_mounted(d) {
                out(bus, &cwd_string(dos, d));
                out(bus, b"\n");
            } else {
                outs(bus, "Invalid drive specification\n");
            }
        }
        Some(p) => {
            if dos.chdir(bus, p).is_err() {
                outs(bus, "Invalid directory\n");
            }
        }
    }
}

/// `1,234,567`
fn grouped(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn dir_line(e: &DirEntry) -> String {
    let fcb = e.name.fcb();
    let (base, ext) = (latin1(&fcb[..8]), latin1(&fcb[8..]));
    let size = if e.is_dir() {
        "<DIR>    ".to_string()
    } else {
        format!("{:>9}", grouped(e.size as u64))
    };
    let t = e.mtime;
    let (h, ampm) = match t.hour {
        0 => (12, 'a'),
        h @ 1..=11 => (h, 'a'),
        12 => (12, 'p'),
        h => (h - 12, 'p'),
    };
    format!(
        "{base} {ext} {size} {:02}-{:02}-{:02}  {h:>2}:{:02}{ampm}\n",
        t.month,
        t.day,
        t.year % 100,
        t.minute
    )
}

fn dir<B: Bus>(dos: &mut Dos, bus: &mut B, a: &[Vec<u8>]) {
    let wide = a.iter().any(|x| x.eq_ignore_ascii_case(b"/W"));
    let spec = a
        .iter()
        .find(|x| !x.starts_with(b"/"))
        .cloned()
        .unwrap_or_default();
    let r = match dos.resolve(bus, &spec) {
        Ok(r) => r,
        Err(e) => return outs(bus, &format!("{}\n", error_text(e))),
    };
    // A directory lists everything in it; otherwise the last part is a
    // pattern (`FOO` means `FOO.*`).
    let (dir, pat) = match r.path() {
        Ok(p) if dos.is_dir(r.drive, &p) => (p, DosPattern::parse("*.*").expect("valid")),
        _ => {
            let Some((last, parent)) = r.parts.split_last() else {
                return outs(bus, "File not found\n");
            };
            let mut last = last.clone();
            if !last.contains(&b'.') {
                last.extend(b".*");
            }
            let parent = Resolved {
                drive: r.drive,
                parts: parent.to_vec(),
            };
            let parent = match parent.path() {
                Ok(p) => p,
                Err(e) => return outs(bus, &format!("{}\n", error_text(e))),
            };
            match DosPattern::from_bytes(&last) {
                Ok(pat) => (parent, pat),
                Err(_) => return outs(bus, "File not found\n"),
            }
        }
    };
    let drive = r.drive;
    let Ok(d) = dos.drive_mut(drive) else {
        return outs(bus, "Invalid drive specification\n");
    };
    let info = d.info();
    let entries = match d.read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return outs(bus, "Path not found\n"),
    };
    let letter = (b'A' + drive) as char;
    if info.label.is_empty() {
        outs(bus, &format!("\n Volume in drive {letter} has no label\n"));
    } else {
        outs(
            bus,
            &format!("\n Volume in drive {letter} is {}\n", info.label),
        );
    }
    let mut head = format!(" Directory of {letter}:").into_bytes();
    head.extend(dir.to_bytes());
    head.extend(b"\n\n");
    out(bus, &head);
    let hidden = Attr::HIDDEN | Attr::SYSTEM | Attr::VOLUME;
    let shown: Vec<&DirEntry> = entries
        .iter()
        .filter(|e| !e.attr.intersects(hidden) && pat.matches(&e.name))
        .collect();
    let (mut files, mut bytes) = (0u64, 0u64);
    for (i, e) in shown.iter().enumerate() {
        if wide {
            let mut n = e.name.to_bytes();
            if e.is_dir() {
                n.insert(0, b'[');
                n.push(b']');
            }
            n.resize(16, b' ');
            out(bus, &n);
            if i % 5 == 4 {
                out(bus, b"\n");
            }
        } else {
            outs(bus, &dir_line(e));
        }
        if !e.is_dir() {
            files += 1;
            bytes += e.size as u64;
        }
    }
    if wide && !shown.len().is_multiple_of(5) {
        out(bus, b"\n");
    }
    if shown.is_empty() {
        outs(bus, "File not found\n");
    }
    outs(
        bus,
        &format!(
            "{files:>9} file(s){:>15} bytes\n{:>34} bytes free\n",
            grouped(bytes),
            grouped(info.free_bytes)
        ),
    );
}

fn type_file<B: Bus>(dos: &mut Dos, bus: &mut B, a: &[Vec<u8>]) {
    let Some(p) = a.first() else {
        return outs(bus, "Required parameter missing\n");
    };
    let res = (|| -> Result<Vec<u8>, DosError> {
        let (d, path) = dos.resolve_path(bus, p)?;
        let mut f = dos.drive_mut(d)?.open(&path, OpenMode::Read)?;
        let mut data = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match f.read(&mut buf)? {
                0 => break,
                n => data.extend_from_slice(&buf[..n]),
            }
        }
        Ok(data)
    })();
    match res {
        Ok(data) => {
            let end = data.iter().position(|&b| b == 0x1A).unwrap_or(data.len());
            out(bus, &data[..end]);
        }
        Err(e) => outs(bus, &format!("{}\n", error_text(e))),
    }
}

fn set<B: Bus>(dos: &mut Dos, bus: &mut B, rest: &[u8]) {
    let arg = latin1(rest.trim_ascii());
    match arg.split_once('=') {
        None if arg.is_empty() => {
            for v in env_vars(dos, bus) {
                outs(bus, &format!("{v}\n"));
            }
        }
        None => {
            // `SET NAME` shows matching variables.
            let up = arg.to_ascii_uppercase();
            for v in env_vars(dos, bus)
                .into_iter()
                .filter(|v| v.starts_with(&up))
            {
                outs(bus, &format!("{v}\n"));
            }
        }
        Some((k, v)) => {
            if set_env(dos, bus, k.trim(), v).is_err() {
                outs(bus, "Out of environment space\n");
            }
        }
    }
}

fn path<B: Bus>(dos: &mut Dos, bus: &mut B, rest: &[u8]) {
    let arg = latin1(rest.trim_ascii());
    let arg = arg.strip_prefix('=').unwrap_or(&arg).trim();
    if arg.is_empty() {
        match get_env(dos, bus, "PATH") {
            Some(p) if !p.is_empty() => outs(bus, &format!("PATH={p}\n")),
            _ => outs(bus, "No Path\n"),
        }
    } else {
        let v = if arg == ";" {
            String::new()
        } else {
            arg.to_ascii_uppercase()
        };
        if set_env(dos, bus, "PATH", &v).is_err() {
            outs(bus, "Out of environment space\n");
        }
    }
}

impl Shell {
    fn echo<B: Bus>(&mut self, bus: &mut B, rest: &[u8]) {
        let word = rest.trim_ascii();
        if word.eq_ignore_ascii_case(b"ON") || word.eq_ignore_ascii_case(b"OFF") {
            self.echo_on = word.eq_ignore_ascii_case(b"ON");
            return;
        }
        match rest.first() {
            Some(b'.') | Some(b'/') | Some(b':') if rest.len() == 1 => out(bus, b"\n"),
            Some(&c) if !word.is_empty() || c == b'.' => {
                out(bus, &rest[1..]);
                out(bus, b"\n");
            }
            _ => {
                let state = if self.echo_on { "on" } else { "off" };
                outs(bus, &format!("ECHO is {state}\n"));
            }
        }
    }

    /// Find `word` (cwd, then PATH; `.COM`, `.EXE`, `.BAT`) and start it.
    fn run_program<B: Bus>(
        &mut self,
        dos: &mut Dos,
        cpu: &mut Cpu,
        bus: &mut B,
        word: &str,
        rest: &[u8],
    ) -> Flow {
        let name = word.as_bytes();
        let has_ext = name
            .rsplit(|&c| c == b'\\' || c == b':')
            .next()
            .is_some_and(|last| last.contains(&b'.'));
        let candidates = |base: &[u8]| -> Vec<Vec<u8>> {
            if has_ext {
                vec![base.to_vec()]
            } else {
                [&b".COM"[..], b".EXE", b".BAT"]
                    .iter()
                    .map(|e| [base, e].concat())
                    .collect()
            }
        };
        let mut dirs: Vec<Vec<u8>> = vec![Vec::new()];
        if !name.iter().any(|&c| c == b'\\' || c == b':') {
            if let Some(p) = get_env(dos, bus, "PATH") {
                dirs.extend(p.split(';').filter(|d| !d.is_empty()).map(|d| {
                    let mut d = d.as_bytes().to_vec();
                    if !d.ends_with(b"\\") {
                        d.push(b'\\');
                    }
                    d
                }));
            }
        }
        let found = dirs.iter().find_map(|dir| {
            candidates(&[dir.as_slice(), name].concat())
                .into_iter()
                .find(|c| find_file(dos, bus, c).is_some())
        });
        let Some(path) = found else {
            outs(bus, "Bad command or file name\n");
            return Flow::Continue;
        };
        if path.to_ascii_uppercase().ends_with(b".BAT") {
            let (drive, file) = find_file(dos, bus, &path).expect("found above");
            let mut argv = vec![word.as_bytes().to_vec()];
            argv.extend(args(rest));
            let call = std::mem::take(&mut self.calling);
            self.start_batch(drive, file, argv, call);
            return Flow::Continue;
        }
        let tail = latin1(rest);
        match dos.exec(cpu, bus, &path, &ExecParams::from_tail(&tail)) {
            Ok(()) => Flow::Started,
            Err(e) => {
                outs(bus, &format!("{}\n", error_text(e)));
                Flow::Continue
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_splitting() {
        let s = |l: &str| {
            let (w, r) = split_command(l.as_bytes());
            (w, String::from_utf8(r).unwrap())
        };
        assert_eq!(s("  dir /w "), ("DIR".into(), " /w".into()));
        assert_eq!(s("cd\\apps"), ("CD".into(), "\\apps".into()));
        assert_eq!(s("echo.hi"), ("ECHO".into(), ".hi".into()));
        assert_eq!(s("echoes"), ("ECHOES".into(), "".into()));
        assert_eq!(s("tool.com a b"), ("TOOL.COM".into(), " a b".into()));
        assert_eq!(
            args(b" a \"b c\"  d"),
            [b"a".to_vec(), b"b c".to_vec(), b"d".to_vec()]
        );
        assert_eq!(grouped(1234567), "1,234,567");
        assert_eq!(grouped(999), "999");
    }
}
