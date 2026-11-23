//! Emulation thread: owns the `Machine`, paces it to real time, publishes
//! frames to a [`Mailbox`] and wakes the main thread via a callback.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use dosy_core::cpu::CpuModel;
use dosy_core::machine::{Machine, MachineConfig, RunExit};
use dosy_core::vfs::Drive;
use dosy_core::video::VideoFrame;

use crate::debug::DebugSetup;
use crate::mailbox::Mailbox;
use crate::pacing::Pacer;

/// Main thread → emulation thread.
#[derive(Debug)]
pub enum HostEvent {
    /// Set-1 scancode bytes from the host keyboard.
    Key(Vec<u8>),
    Quit,
}

/// Emulation thread → main thread (via the wake callback).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmuEvent {
    /// A new frame is in the mailbox.
    Frame,
    /// Emulation stopped (until reset/resume actions, 042); says why.
    Paused(String),
}

/// Everything the emulation thread needs at startup.
pub struct EmuSetup {
    /// `None` = `cycles = max`.
    pub cycles_per_ms: Option<u32>,
    pub cpu_model: CpuModel,
    /// Host wall clock (Unix seconds) for the RTC.
    pub rtc_unix_secs: i64,
    /// Opened drives (see `cli::open_drive`).
    pub mounts: Vec<(char, Box<dyn Drive>)>,
    /// Shell lines to run at boot.
    pub autoexec: Vec<String>,
    /// Tracing and INT call logging.
    pub debug: DebugSetup,
}

/// Cycles per `run_for` call in `cycles = max` mode.
const MAX_CHUNK: u64 = 20_000;
/// Nominal tick.
const TICK: Duration = Duration::from_millis(1);
/// Minimum wall time between published frames in `cycles = max` mode
/// (the emulated 70 Hz retrace runs much faster there).
const MIN_PRESENT: Duration = Duration::from_millis(14);

pub struct EmuHandle {
    tx: Sender<HostEvent>,
    join: Option<JoinHandle<()>>,
}

impl EmuHandle {
    pub fn send(&self, ev: HostEvent) {
        let _ = self.tx.send(ev);
    }

    /// Stops the thread and waits for it.
    pub fn shutdown(&mut self) {
        let _ = self.tx.send(HostEvent::Quit);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Drop for EmuHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Starts the emulation thread.
pub fn spawn(
    setup: EmuSetup,
    frames: Arc<Mailbox<VideoFrame>>,
    wake: impl Fn(EmuEvent) + Send + 'static,
) -> std::io::Result<EmuHandle> {
    let (tx, rx) = mpsc::channel();
    let join = std::thread::Builder::new()
        .name("dosy-emu".into())
        .spawn(move || Emu::new(setup, frames, Box::new(wake)).run(rx))?;
    Ok(EmuHandle {
        tx,
        join: Some(join),
    })
}

struct Emu {
    machine: Machine,
    pacer: Option<Pacer>,
    frames: Arc<Mailbox<VideoFrame>>,
    wake: Box<dyn Fn(EmuEvent) + Send>,
    /// Reused frame buffer for publishing.
    out: VideoFrame,
    /// `seq` of the last published frame.
    last_seq: Option<u64>,
    last_present: Option<Instant>,
    paused: bool,
}

impl Emu {
    fn new(
        setup: EmuSetup,
        frames: Arc<Mailbox<VideoFrame>>,
        wake: Box<dyn Fn(EmuEvent) + Send>,
    ) -> Self {
        let mut cfg = MachineConfig {
            cpu_model: setup.cpu_model,
            ..MachineConfig::default()
        };
        if let Some(n) = setup.cycles_per_ms {
            cfg.cycles_per_ms = n;
        }
        let mut machine = Machine::new(cfg);
        match setup.debug.tracer() {
            Ok(t) => machine.tracer = t,
            Err(e) => eprintln!("dosy: trace file: {e}"),
        }
        for (letter, drive) in setup.mounts {
            machine.mount(letter, drive);
        }
        machine.set_rtc(setup.rtc_unix_secs);
        machine.start_shell(&setup.autoexec);
        Self {
            machine,
            pacer: setup.cycles_per_ms.map(Pacer::new),
            frames,
            wake,
            out: VideoFrame::blank(1, 1),
            last_seq: None,
            last_present: None,
            paused: false,
        }
    }

    fn run(mut self, rx: Receiver<HostEvent>) {
        let mut last = Instant::now();
        loop {
            // Wait for the next tick (or forever while paused), handling events.
            let wait = if self.paused {
                Duration::from_secs(3600)
            } else {
                TICK.saturating_sub(last.elapsed())
            };
            match rx.recv_timeout(wait) {
                Ok(ev) => {
                    if !self.handle(ev) {
                        return;
                    }
                    while let Ok(ev) = rx.try_recv() {
                        if !self.handle(ev) {
                            return;
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            let now = Instant::now();
            let elapsed = now - last;
            last = now;
            if !self.paused {
                self.run_slice(elapsed);
                self.publish();
            }
        }
    }

    /// Returns false on quit.
    fn handle(&mut self, ev: HostEvent) -> bool {
        match ev {
            HostEvent::Quit => false,
            HostEvent::Key(bytes) => {
                self.machine.key_event(&bytes);
                true
            }
        }
    }

    fn run_slice(&mut self, elapsed: Duration) {
        let exit = match &mut self.pacer {
            Some(p) => self.machine.run_for(p.budget(elapsed)),
            None => {
                // Flat out for about one tick.
                let start = Instant::now();
                loop {
                    let e = self.machine.run_for(MAX_CHUNK);
                    if e != RunExit::Budget || start.elapsed() >= TICK {
                        break e;
                    }
                }
            }
        };
        if !matches!(exit, RunExit::Budget | RunExit::Exited(_)) {
            eprint!("dosy: {}", self.machine.fault_report(exit));
        }
        let why = match exit {
            RunExit::Budget => return,
            RunExit::Halted => "CPU halted".to_string(),
            RunExit::Fault(f) => format!("CPU fault: {f:?}"),
            RunExit::UnhandledCallback(id) => format!("unhandled HLE callback {id:?}"),
            RunExit::Exited(code) => format!("shell exited with code {code}"),
        };
        self.paused = true;
        (self.wake)(EmuEvent::Paused(why));
    }

    /// Publishes the machine's latest completed frame if it is new (and,
    /// in `cycles = max` mode, if [`MIN_PRESENT`] has passed).
    fn publish(&mut self) {
        let Some(frame) = self.machine.frame() else {
            return;
        };
        if Some(frame.seq) == self.last_seq {
            return;
        }
        // Always show the final frame when the machine just paused.
        if self.pacer.is_none()
            && !self.paused
            && self.last_present.is_some_and(|t| t.elapsed() < MIN_PRESENT)
        {
            return;
        }
        self.last_seq = Some(frame.seq);
        self.last_present = Some(Instant::now());
        self.out.clone_from(frame);
        self.frames.publish(&mut self.out);
        (self.wake)(EmuEvent::Frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use dosy_core::vfs::MemDrive;

    /// C: holds LOOP.COM, which writes a white-on-blue 'A' to B800:0000
    /// and spins.
    fn setup(cycles: Option<u32>) -> EmuSetup {
        let mut c = MemDrive::new();
        #[rustfmt::skip]
        c.add_file("\\LOOP.COM", &[
            0xB8, 0x00, 0xB8,                         // mov ax, B800h
            0x8E, 0xC0,                               // mov es, ax
            0x26, 0xC7, 0x06, 0x00, 0x00, 0x41, 0x1F, // mov word es:[0], 1F41h
            0xEB, 0xFE,                               // jmp $
        ]);
        EmuSetup {
            cycles_per_ms: cycles,
            cpu_model: CpuModel::I386,
            rtc_unix_secs: 0,
            mounts: vec![('C', Box::new(c))],
            autoexec: vec!["C:".into(), "LOOP.COM".into()],
            debug: DebugSetup::default(),
        }
    }

    #[test]
    fn boots_to_shell_prompt() {
        let frames = Arc::new(Mailbox::new(VideoFrame::blank(1, 1)));
        let (wtx, wrx) = mpsc::channel();
        let mut s = setup(Some(3000));
        s.mounts.clear();
        s.autoexec.clear();
        let mut h = spawn(s, frames.clone(), move |e| {
            let _ = wtx.send(e);
        })
        .unwrap();
        // `Z:\>` in light grey (7) on the top text row (16 scanlines).
        let mut f = VideoFrame::blank(1, 1);
        let lit = |f: &VideoFrame| f.width == 720 && f.pixels[..720 * 16].contains(&7);
        while !lit(&f) {
            match wrx.recv_timeout(Duration::from_secs(5)).unwrap() {
                EmuEvent::Frame => {
                    frames.take(&mut f);
                }
                EmuEvent::Paused(why) => panic!("paused: {why}"),
            }
        }
        h.shutdown();
    }

    #[test]
    fn shell_exit_pauses() {
        let frames = Arc::new(Mailbox::new(VideoFrame::blank(1, 1)));
        let (wtx, wrx) = mpsc::channel();
        let mut s = setup(Some(3000));
        s.autoexec = vec!["exit".into()];
        let mut h = spawn(s, frames, move |e| {
            let _ = wtx.send(e);
        })
        .unwrap();
        loop {
            match wrx.recv_timeout(Duration::from_secs(5)).unwrap() {
                EmuEvent::Frame => {}
                EmuEvent::Paused(why) => {
                    assert_eq!(why, "shell exited with code 0");
                    break;
                }
            }
        }
        h.shutdown();
    }

    #[test]
    fn publishes_machine_frames_and_shuts_down() {
        for cycles in [Some(3000), None] {
            let frames = Arc::new(Mailbox::new(VideoFrame::blank(1, 1)));
            let (wtx, wrx) = mpsc::channel();
            let mut h = spawn(setup(cycles), frames.clone(), move |e| {
                let _ = wtx.send(e);
            })
            .unwrap();
            let mut f = VideoFrame::blank(1, 1);
            let mut seqs = Vec::new();
            while seqs.len() < 2 {
                match wrx.recv_timeout(Duration::from_secs(5)).unwrap() {
                    EmuEvent::Frame => {
                        if frames.take(&mut f) {
                            seqs.push(f.seq);
                        }
                    }
                    EmuEvent::Paused(why) => panic!("paused: {why}"),
                }
            }
            assert!(seqs[1] > seqs[0], "new frames only: {seqs:?}");
            assert_eq!((f.width, f.height), (720, 400), "BIOS text mode");
            // The 'A' cell: some pixel in it has the blue background (1).
            assert!(f.pixels[..9].contains(&1), "{:?}", &f.pixels[..9]);
            h.shutdown();
        }
    }
}
