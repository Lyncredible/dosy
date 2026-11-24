//! Private smoke test: boot a zipped DOS program headlessly and check that
//! it runs and draws. Skipped unless `DOSY_SMOKE_ZIP` is set, so CI never
//! needs (or sees) any DOS software.
//!
//! Environment:
//! - `DOSY_SMOKE_ZIP`: path to a `.zip` (or a directory), mounted as C: with
//!   an in-memory write overlay.
//! - `DOSY_SMOKE_CMD`: shell command line to run on C: (e.g. a batch file).
//! - `DOSY_SMOKE_SECS`: emulated seconds to run (default 30).
//! - `DOSY_SMOKE_KEYS`: optional key presses, `SECS=KEY` separated by commas
//!   or spaces, e.g. `5=enter,8.5=space,10=y`. KEY is a letter, digit,
//!   `enter`, `space`, `esc`, `tab`, `bksp`, `up`/`down`/`left`/`right`,
//!   `f1`–`f10`, or raw set-1 bytes like `0x1c` (make + break sent).
//! - `DOSY_SMOKE_PNG`: where to write the final frame (default
//!   `target/smoke/final.png`).
//! - `DOSY_SMOKE_ALLOW_TEXT=1`: pass even if no graphics-mode frame appeared.
//!
//! Run: `cargo test -p dosy-core --test smoke -- --nocapture`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use dosy_core::debug::Tracer;
use dosy_core::machine::{Machine, MachineConfig, RunExit};
use dosy_core::vfs::{self, MountOptions, Overlay};
use dosy_core::video::VideoFrame;

/// Collects `log` warnings so the test can report and check them.
struct Recorder;

static WARNINGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

impl log::Log for Recorder {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Warn
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            let mut w = WARNINGS.lock().unwrap_or_else(|e| e.into_inner());
            let msg = format!("{}: {}", r.level(), r.args());
            if !w.contains(&msg) {
                w.push(msg);
            }
        }
    }
    fn flush(&self) {}
}

static RECORDER: Recorder = Recorder;

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// Set-1 make code for a key name.
fn key_code(name: &str) -> Option<Vec<u8>> {
    let n = name.to_ascii_lowercase();
    if let Some(hex) = n.strip_prefix("0x") {
        let b = u8::from_str_radix(hex, 16).ok()?;
        return Some(vec![b, b | 0x80]);
    }
    const ROWS: [(&str, u8); 4] = [
        ("1234567890", 0x02),
        ("qwertyuiop", 0x10),
        ("asdfghjkl", 0x1E),
        ("zxcvbnm", 0x2C),
    ];
    let mut chars = n.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        for (row, base) in ROWS {
            if let Some(i) = row.find(c) {
                let m = base + i as u8;
                return Some(vec![m, m | 0x80]);
            }
        }
    }
    let (make, e0) = match n.as_str() {
        "enter" => (0x1C, false),
        "space" => (0x39, false),
        "esc" => (0x01, false),
        "tab" => (0x0F, false),
        "bksp" => (0x0E, false),
        "up" => (0x48, true),
        "down" => (0x50, true),
        "left" => (0x4B, true),
        "right" => (0x4D, true),
        f if f.starts_with('f') => {
            let k: u8 = f[1..].parse().ok().filter(|k| (1..=10).contains(k))?;
            (0x3A + k, false)
        }
        _ => return None,
    };
    Some(if e0 {
        vec![0xE0, make, 0xE0, make | 0x80]
    } else {
        vec![make, make | 0x80]
    })
}

/// `SECS=KEY` list → (ns, bytes), sorted by time.
fn parse_keys(s: &str) -> Vec<(u64, Vec<u8>)> {
    let mut keys: Vec<(u64, Vec<u8>)> = s
        .split([',', ' '])
        .filter(|t| !t.is_empty())
        .map(|t| {
            let (secs, key) = t
                .split_once('=')
                .unwrap_or_else(|| panic!("DOSY_SMOKE_KEYS: expected SECS=KEY, got {t:?}"));
            let secs: f64 = secs.parse().expect("DOSY_SMOKE_KEYS: bad seconds");
            let bytes =
                key_code(key).unwrap_or_else(|| panic!("DOSY_SMOKE_KEYS: unknown key {key:?}"));
            ((secs * 1e9) as u64, bytes)
        })
        .collect();
    keys.sort_by_key(|k| k.0);
    keys
}

fn non_blank(f: &VideoFrame) -> bool {
    match &f.text {
        Some(t) => t.cells.iter().any(|c| c.ch != b' ' && c.ch != 0),
        None => f.pixels.iter().any(|&p| p != f.pixels[0]),
    }
}

/// The 024 fault report: registers, disassembly at CS:IP, trace history.
fn dump(m: &mut Machine, exit: RunExit) -> String {
    m.fault_report(exit)
}

#[test]
fn smoke() {
    let Some(zip) = env("DOSY_SMOKE_ZIP") else {
        eprintln!("smoke: DOSY_SMOKE_ZIP not set; skipping");
        return;
    };
    let _ = log::set_logger(&RECORDER);
    log::set_max_level(log::LevelFilter::Warn);

    let cmd = env("DOSY_SMOKE_CMD");
    let secs: f64 = env("DOSY_SMOKE_SECS").map_or(30.0, |s| s.parse().expect("DOSY_SMOKE_SECS"));
    let mut keys = parse_keys(&env("DOSY_SMOKE_KEYS").unwrap_or_default())
        .into_iter()
        .peekable();
    let allow_text = env("DOSY_SMOKE_ALLOW_TEXT").is_some_and(|v| v == "1");

    let opts = MountOptions {
        overlay: Overlay::Memory,
        ..Default::default()
    };
    let drive = vfs::open_mount(Path::new(&zip), &opts).expect("mount DOSY_SMOKE_ZIP");
    let mut m = Machine::new(MachineConfig::default());
    m.tracer = Some(Box::new(Tracer::new(64)));
    m.mount('c', drive);
    let mut lines = vec!["C:".to_string()];
    lines.extend(cmd);
    m.start_shell(&lines);

    let end = (secs * 1e9) as u64;
    let slice = m.hw.cycles_per_ms() as u64 * 50; // 50 ms of emulated time
    let (mut frames, mut non_blank_frames, mut gfx_frames, mut last_seq) = (0u32, 0u32, 0u32, None);
    let mut last: Option<VideoFrame> = None;
    let mut failure = None;
    while m.hw.now() < end {
        while let Some((t, _)) = keys.peek() {
            if *t > m.hw.now() {
                break;
            }
            let (_, bytes) = keys.next().expect("peeked");
            m.key_event(&bytes);
        }
        match m.run_for(slice) {
            RunExit::Budget => {}
            RunExit::Exited(code) => {
                eprintln!("smoke: shell exited with {code}");
                break;
            }
            e => {
                failure = Some(e);
                break;
            }
        }
        if let Some(f) = m.frame() {
            if last_seq != Some(f.seq) {
                last_seq = Some(f.seq);
                frames += 1;
                if non_blank(f) {
                    non_blank_frames += 1;
                    if f.text.is_none() {
                        gfx_frames += 1;
                    }
                }
                last = Some(f.clone());
            }
        }
    }

    let png = env("DOSY_SMOKE_PNG").map(PathBuf::from).unwrap_or_else(|| {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/smoke/final.png")
    });
    if let Some(f) = &last {
        if let Some(dir) = png.parent() {
            std::fs::create_dir_all(dir).expect("create PNG dir");
        }
        std::fs::write(&png, png::encode(f)).expect("write PNG");
        eprintln!(
            "smoke: final frame {}x{} → {}",
            f.width,
            f.height,
            png.display()
        );
    }
    let warnings = WARNINGS.lock().unwrap_or_else(|e| e.into_inner()).clone();
    eprintln!(
        "smoke: {:.1}s emulated, {frames} frames ({non_blank_frames} non-blank, {gfx_frames} graphics), \
         {} distinct warnings",
        m.hw.now() as f64 / 1e9,
        warnings.len()
    );
    for w in &warnings {
        eprintln!("  {w}");
    }
    if let Some(f) = failure {
        panic!("smoke: stopped with {f:?}\n{}", dump(&mut m, f));
    }
    let opcode = warnings
        .iter()
        .find(|w| w.to_ascii_lowercase().contains("opcode"));
    assert!(
        opcode.is_none(),
        "smoke: {}\n{}",
        opcode.unwrap(),
        dump(&mut m, RunExit::Budget)
    );
    assert!(
        non_blank_frames > 0,
        "smoke: no non-blank frame\n{}",
        dump(&mut m, RunExit::Budget)
    );
    assert!(
        allow_text || gfx_frames > 0,
        "smoke: no graphics-mode frame (DOSY_SMOKE_ALLOW_TEXT=1 to accept)\n{}",
        dump(&mut m, RunExit::Budget)
    );
}

/// Minimal PNG writer (RGB, uncompressed deflate) for frame dumps.
mod png {
    use dosy_core::video::VideoFrame;

    fn crc32(data: &[u8]) -> u32 {
        let mut c = !0u32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
        }
        !c
    }

    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend((data.len() as u32).to_be_bytes());
        let start = out.len();
        out.extend(kind);
        out.extend(data);
        let crc = crc32(&out[start..]);
        out.extend(crc.to_be_bytes());
    }

    /// zlib stream with stored (uncompressed) blocks.
    fn zlib_stored(data: &[u8]) -> Vec<u8> {
        let mut out = vec![0x78, 0x01];
        let mut chunks = data.chunks(0xFFFF).peekable();
        if chunks.peek().is_none() {
            out.extend([1, 0, 0, 0xFF, 0xFF]);
        }
        while let Some(c) = chunks.next() {
            out.push(chunks.peek().is_none() as u8);
            let n = c.len() as u16;
            out.extend(n.to_le_bytes());
            out.extend((!n).to_le_bytes());
            out.extend(c);
        }
        let (mut a, mut b) = (1u32, 0u32);
        for &x in data {
            a = (a + x as u32) % 65521;
            b = (b + a) % 65521;
        }
        out.extend((b << 16 | a).to_be_bytes());
        out
    }

    /// Indexed frame → RGB PNG. Text frames are shown as the rasterized
    /// pixels the core produced.
    pub fn encode(f: &VideoFrame) -> Vec<u8> {
        let (w, h) = (f.width as usize, f.height as usize);
        let mut raw = Vec::with_capacity((w * 3 + 1) * h);
        for row in f.pixels.chunks(w.max(1)).take(h) {
            raw.push(0); // filter: none
            for &p in row {
                let c = f.palette[p as usize];
                raw.extend([c.r, c.g, c.b]);
            }
        }
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend((w as u32).to_be_bytes());
        ihdr.extend((h as u32).to_be_bytes());
        ihdr.extend([8, 2, 0, 0, 0]); // 8-bit RGB
        chunk(&mut out, b"IHDR", &ihdr);
        chunk(&mut out, b"IDAT", &zlib_stored(&raw));
        chunk(&mut out, b"IEND", &[]);
        out
    }

    #[test]
    fn crc_matches_reference() {
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
    }
}

#[test]
fn key_spec_parses() {
    let k = parse_keys("2=enter, 1.5=a,3=up,4=0x1c,5=f10");
    let t: Vec<u64> = k.iter().map(|k| k.0).collect();
    assert_eq!(
        t,
        [
            1_500_000_000,
            2_000_000_000,
            3_000_000_000,
            4_000_000_000,
            5_000_000_000
        ]
    );
    assert_eq!(k[0].1, [0x1E, 0x9E]);
    assert_eq!(k[2].1, [0xE0, 0x48, 0xE0, 0xC8]);
    assert_eq!(k[4].1, [0x44, 0xC4]);
}
