//! Batch files: frames, line reading, `%` expansion, GOTO, IF, FOR, SHIFT,
//! CHOICE.

use super::builtins::args;
use super::{Flow, Shell, State, find_file, get_env, out, outs};
use crate::bus::Bus;
use crate::cpu::Cpu;
use crate::dos::Dos;
use crate::hle::HleResult;
use crate::vfs::{Attr, DosPath, OpenMode, SeekFrom};

/// Longest batch line read; the rest of a longer line is skipped.
const MAX_LINE: usize = 255;

/// A running batch file. The file is reopened for every line, so edits
/// made while it runs are seen, as in DOS.
pub(super) struct Frame {
    drive: u8,
    path: DosPath,
    /// Byte offset of the next line.
    pos: u64,
    /// `%0` (as typed) and the arguments.
    argv: Vec<Vec<u8>>,
    shift: usize,
}

/// Read the line at `pos`: returns it (without CR/LF) and the next offset,
/// or `None` at end of file (or ^Z).
fn read_line(dos: &mut Dos, drive: u8, path: &DosPath, pos: u64) -> Option<(Vec<u8>, u64)> {
    let mut f = dos.drive_mut(drive).ok()?.open(path, OpenMode::Read).ok()?;
    f.seek(SeekFrom::Start(pos)).ok()?;
    let mut line = Vec::new();
    let mut next = pos;
    let mut buf = [0u8; 128];
    'outer: loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            if line.is_empty() && next == pos {
                return None;
            }
            break;
        }
        for &b in &buf[..n] {
            if b == 0x1A {
                if next == pos {
                    return None;
                }
                break 'outer;
            }
            next += 1;
            if b == b'\n' {
                break 'outer;
            }
            if b != b'\r' && line.len() < MAX_LINE {
                line.push(b);
            }
        }
    }
    Some((line, next))
}

fn eq_word(a: &[u8], b: &[u8]) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// First whitespace-separated token and the rest.
fn token(s: &[u8]) -> (&[u8], &[u8]) {
    let s = s.trim_ascii_start();
    let end = s
        .iter()
        .position(|c| c.is_ascii_whitespace())
        .unwrap_or(s.len());
    (&s[..end], &s[end..])
}

impl Shell {
    /// Start a batch file. Without CALL, a batch run from a batch replaces
    /// it (chaining), as in COMMAND.COM.
    pub(super) fn start_batch(&mut self, drive: u8, path: DosPath, argv: Vec<Vec<u8>>, call: bool) {
        let frame = Frame {
            drive,
            path,
            pos: 0,
            argv,
            shift: 0,
        };
        if !call {
            self.frames.pop();
        }
        self.frames.push(frame);
    }

    pub(super) fn read_batch_line(&mut self, dos: &mut Dos) -> Option<Vec<u8>> {
        let f = self.frames.last_mut()?;
        let (line, next) = read_line(dos, f.drive, &f.path, f.pos)?;
        f.pos = next;
        Some(line)
    }

    /// Expand `%0`–`%9`, `%%` and `%VAR%` with the innermost frame.
    fn expand<B: Bus>(&self, dos: &Dos, bus: &mut B, line: &[u8]) -> Vec<u8> {
        let frame = self.frames.last();
        let mut out = Vec::with_capacity(line.len());
        let mut i = 0;
        while i < line.len() {
            let c = line[i];
            if c != b'%' {
                out.push(c);
                i += 1;
                continue;
            }
            match line.get(i + 1) {
                Some(b'%') => {
                    out.push(b'%');
                    i += 2;
                }
                Some(d @ b'0'..=b'9') => {
                    if let Some(f) = frame {
                        let n = f.shift + (d - b'0') as usize;
                        if let Some(a) = f.argv.get(n) {
                            out.extend(a);
                        }
                    }
                    i += 2;
                }
                _ => match line[i + 1..].iter().position(|&c| c == b'%') {
                    Some(len) if len > 0 => {
                        let name: String = line[i + 1..i + 1 + len]
                            .iter()
                            .map(|&b| b as char)
                            .collect();
                        if let Some(v) = get_env(dos, bus, &name) {
                            out.extend(v.chars().map(|c| c as u8));
                        }
                        i += len + 2;
                    }
                    _ => i += 1, // lone %: dropped
                },
            }
        }
        out
    }

    /// Process one raw batch line.
    pub(super) fn batch_line<B: Bus>(
        &mut self,
        dos: &mut Dos,
        cpu: &mut Cpu,
        bus: &mut B,
        raw: &[u8],
    ) -> Option<HleResult> {
        let t = raw.trim_ascii_start();
        if t.starts_with(b":") {
            return None; // label (or `::` comment)
        }
        let line = self.expand(dos, bus, t);
        let (quiet, line) = match line.strip_prefix(b"@") {
            Some(rest) => (true, rest.to_vec()),
            None => (false, line),
        };
        if line.trim_ascii().is_empty() {
            return None;
        }
        if self.echo_on && !quiet {
            self.prompt(dos, bus);
            out(bus, &line);
            out(bus, b"\n");
        }
        self.finish(dos, cpu, bus, &line)
    }

    pub(super) fn shift(&mut self) {
        if let Some(f) = self.frames.last_mut() {
            f.shift += 1;
        }
    }

    /// GOTO: continue after the `:label` line (first 8 chars compared).
    pub(super) fn goto<B: Bus>(&mut self, dos: &mut Dos, bus: &mut B, a: &[Vec<u8>]) {
        let Some(f) = self.frames.last() else {
            return; // ignored at the prompt
        };
        let want = a
            .first()
            .map(|l| l.strip_prefix(b":").unwrap_or(l))
            .unwrap_or(&[]);
        let want = &want[..want.len().min(8)];
        let (drive, path) = (f.drive, f.path.clone());
        let mut pos = 0;
        while let Some((line, next)) = read_line(dos, drive, &path, pos) {
            pos = next;
            if let Some(label) = line.trim_ascii_start().strip_prefix(b":") {
                let (name, _) = token(label);
                if eq_word(&name[..name.len().min(8)], want) {
                    if let Some(f) = self.frames.last_mut() {
                        f.pos = pos;
                    }
                    // GOTO inside a FOR body ends the loop.
                    self.pending.clear();
                    return;
                }
            }
        }
        self.frames.clear();
        self.echo_on = true;
        outs(bus, "Label not found\n");
    }

    /// `IF [NOT] ERRORLEVEL n | EXIST file | s1==s2 command`
    pub(super) fn if_cmd<B: Bus>(
        &mut self,
        dos: &mut Dos,
        cpu: &mut Cpu,
        bus: &mut B,
        rest: &[u8],
    ) -> Flow {
        let (mut w, mut rest) = token(rest);
        let not = eq_word(w, b"NOT");
        if not {
            (w, rest) = token(rest);
        }
        let cond = if eq_word(w, b"ERRORLEVEL") {
            let (n, r) = token(rest);
            rest = r;
            let n: u16 = std::str::from_utf8(n)
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(256);
            self.errorlevel as u16 >= n
        } else if eq_word(w, b"EXIST") {
            let (f, r) = token(rest);
            rest = r;
            exists(dos, bus, f)
        } else {
            // s1==s2: the comparison may have spaces around `==`.
            let s = [w, rest].concat();
            let s = s.trim_ascii_start();
            let Some(eq) = s.windows(2).position(|p| p == b"==") else {
                outs(bus, "Syntax error\n");
                return Flow::Continue;
            };
            let lhs = s[..eq].trim_ascii();
            let (rhs, r) = token(&s[eq + 2..]);
            let cond = lhs == rhs;
            return if cond != not {
                self.run(dos, cpu, bus, r)
            } else {
                Flow::Continue
            };
        };
        if cond != not {
            self.run(dos, cpu, bus, rest)
        } else {
            Flow::Continue
        }
    }

    /// `FOR %v IN (set) DO command`: queues one command per item.
    pub(super) fn for_cmd<B: Bus>(&mut self, dos: &mut Dos, bus: &mut B, rest: &[u8]) {
        let syntax = |bus: &mut B| outs(bus, "Syntax error\n");
        let (var, rest) = token(rest);
        let (kw, rest) = token(rest);
        if !var.starts_with(b"%") || var.len() < 2 || !eq_word(kw, b"IN") {
            return syntax(bus);
        }
        let rest = rest.trim_ascii_start();
        let (Some(open), Some(close)) = (
            rest.iter().position(|&c| c == b'('),
            rest.iter().position(|&c| c == b')'),
        ) else {
            return syntax(bus);
        };
        if open != 0 || close < open {
            return syntax(bus);
        }
        let set = &rest[1..close];
        let (kw, cmd) = token(&rest[close + 1..]);
        if !eq_word(kw, b"DO") {
            return syntax(bus);
        }
        let mut items = Vec::new();
        for item in set
            .split(|&c| c == b' ' || c == b',' || c == b';' || c == b'\t')
            .filter(|s| !s.is_empty())
        {
            if item.iter().any(|&c| c == b'*' || c == b'?') {
                items.extend(glob(dos, bus, item));
            } else {
                items.push(item.to_vec());
            }
        }
        let cmd = cmd.trim_ascii_start();
        let mut cmds: Vec<Vec<u8>> = items.iter().map(|it| replace(cmd, var, it)).collect();
        cmds.extend(self.pending.drain(..));
        self.pending = cmds.into();
    }

    /// `CHOICE [/C:keys] [/N] [text]` (basic): ERRORLEVEL = key index + 1.
    pub(super) fn choice<B: Bus>(&mut self, bus: &mut B, rest: &[u8]) {
        let mut keys = b"YN".to_vec();
        let mut list = true;
        let mut text = Vec::new();
        for a in args(rest) {
            let up = a.to_ascii_uppercase();
            if let Some(k) = up.strip_prefix(b"/C:").or_else(|| up.strip_prefix(b"/C")) {
                keys = k.to_vec();
            } else if up == b"/N" || up == b"/S" {
                list = up != b"/N" && list;
            } else if up.starts_with(b"/T") {
                // Timeouts are ignored.
            } else {
                if !text.is_empty() {
                    text.push(b' ');
                }
                text.extend(a);
            }
        }
        out(bus, &text);
        if list {
            let mut l = vec![b'['];
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    l.push(b',');
                }
                l.push(*k);
            }
            l.extend(b"]?");
            out(bus, &l);
        }
        self.state = State::Choice(keys);
    }
}

/// Batch parameters (`%1`…), as COMMAND.COM splits them: separators are
/// whitespace, `,`, `;` and `=`; quotes group but are kept in the value.
pub(super) fn batch_args(rest: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut cur = Vec::new();
    let mut quoted = false;
    for &c in rest {
        match c {
            b' ' | b'\t' | b',' | b';' | b'=' if !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => {
                quoted ^= c == b'"';
                cur.push(c);
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Replace every occurrence of `var` in `cmd` with `item`.
fn replace(cmd: &[u8], var: &[u8], item: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < cmd.len() {
        if cmd[i..].starts_with(var) {
            out.extend(item);
            i += var.len();
        } else {
            out.push(cmd[i]);
            i += 1;
        }
    }
    out
}

fn exists<B: Bus>(dos: &mut Dos, bus: &mut B, f: &[u8]) -> bool {
    if f.iter().any(|&c| c == b'*' || c == b'?') {
        !glob(dos, bus, f).is_empty()
    } else {
        find_file(dos, bus, f).is_some()
    }
}

/// Files matching a wildcard, with the item's directory prefix kept.
fn glob<B: Bus>(dos: &mut Dos, bus: &mut B, item: &[u8]) -> Vec<Vec<u8>> {
    let prefix_len = item
        .iter()
        .rposition(|&c| c == b'\\' || c == b':')
        .map_or(0, |i| i + 1);
    let Ok(r) = dos.resolve(bus, item) else {
        return Vec::new();
    };
    let Ok((dir, pat)) = r.dir_and_pattern() else {
        return Vec::new();
    };
    let Ok(d) = dos.drive_mut(r.drive) else {
        return Vec::new();
    };
    let skip = Attr::DIRECTORY | Attr::HIDDEN | Attr::SYSTEM | Attr::VOLUME;
    d.read_dir(&dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|e| !e.attr.intersects(skip) && pat.matches(&e.name))
        .map(|e| [&item[..prefix_len], &e.name.to_bytes()].concat())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_args_split_like_command_com() {
        let a = batch_args(b" a,b;c=d \"e f\"  g");
        let want: Vec<&[u8]> = vec![b"a", b"b", b"c", b"d", b"\"e f\"", b"g"];
        assert_eq!(a, want);
    }
}
