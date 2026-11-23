//! Emulation thread: owns the `Machine`, paces it to real time, publishes
//! frames to a [`Mailbox`] and wakes the main thread via a callback.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use dosy_core::machine::{Machine, MachineConfig, RunExit};
use dosy_core::video::VideoFrame;

use crate::mailbox::Mailbox;
use crate::pacing::Pacer;
use crate::pattern;

/// Main thread → emulation thread.
#[derive(Debug)]
pub enum HostEvent {
    /// Debug: cycle the test pattern (until VGA output exists).
    NextPattern,
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

/// What a drive letter is backed by on the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MountSource {
    Dir(PathBuf),
    Zip(PathBuf),
    File(PathBuf),
}

/// Everything the emulation thread needs at startup.
#[derive(Debug, Clone)]
pub struct EmuSetup {
    /// `None` = `cycles = max`.
    pub cycles_per_ms: Option<u32>,
    /// Host wall clock (Unix seconds) for the RTC.
    pub rtc_unix_secs: i64,
    pub mounts: Vec<(char, MountSource)>,
    /// Shell lines to run at boot.
    pub autoexec: Vec<String>,
}

/// Cycles per `run_for` call in `cycles = max` mode.
const MAX_CHUNK: u64 = 20_000;
/// Nominal tick.
const TICK: Duration = Duration::from_millis(1);

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
    patterns: Vec<VideoFrame>,
    current: usize,
    /// Reused frame buffer for publishing.
    out: VideoFrame,
    paused: bool,
}

impl Emu {
    fn new(
        setup: EmuSetup,
        frames: Arc<Mailbox<VideoFrame>>,
        wake: Box<dyn Fn(EmuEvent) + Send>,
    ) -> Self {
        let mut cfg = MachineConfig::default();
        if let Some(n) = setup.cycles_per_ms {
            cfg.cycles_per_ms = n;
        }
        let mut machine = Machine::new(cfg);
        machine.set_rtc(setup.rtc_unix_secs);
        // Host drives (011/028) and the shell (021) don't exist yet.
        for (letter, src) in &setup.mounts {
            eprintln!("dosy: mount {letter}: {src:?} not available yet (host drives pending)");
        }
        let _ = &setup.autoexec;
        Self {
            machine,
            pacer: setup.cycles_per_ms.map(Pacer::new),
            frames,
            wake,
            patterns: vec![pattern::bars_320x200(), pattern::text_720x400()],
            current: 0,
            out: VideoFrame::blank(1, 1),
            paused: false,
        }
    }

    fn run(mut self, rx: Receiver<HostEvent>) {
        self.publish();
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
            }
        }
    }

    /// Returns false on quit.
    fn handle(&mut self, ev: HostEvent) -> bool {
        match ev {
            HostEvent::Quit => return false,
            HostEvent::NextPattern => {
                self.current = (self.current + 1) % self.patterns.len();
                self.publish();
            }
        }
        true
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
        let why = match exit {
            RunExit::Budget => return,
            RunExit::Halted => "CPU halted".to_string(),
            RunExit::Fault(f) => format!("CPU fault: {f:?}"),
            RunExit::UnhandledCallback(id) => format!("unhandled HLE callback {id:?}"),
            RunExit::Exited(code) => format!("program exited with code {code}"),
        };
        self.paused = true;
        (self.wake)(EmuEvent::Paused(why));
    }

    fn publish(&mut self) {
        self.out.clone_from(&self.patterns[self.current]);
        self.out.seq = self.out.seq.wrapping_add(1);
        self.frames.publish(&mut self.out);
        (self.wake)(EmuEvent::Frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(cycles: Option<u32>) -> EmuSetup {
        EmuSetup {
            cycles_per_ms: cycles,
            rtc_unix_secs: 0,
            mounts: vec![],
            autoexec: vec![],
        }
    }

    #[test]
    fn publishes_frames_and_shuts_down() {
        for cycles in [Some(1000), None] {
            let frames = Arc::new(Mailbox::new(VideoFrame::blank(1, 1)));
            let (wtx, wrx) = mpsc::channel();
            let mut h = spawn(setup(cycles), frames.clone(), move |e| {
                let _ = wtx.send(e);
            })
            .unwrap();
            let first = wrx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(first, EmuEvent::Frame);
            let mut f = VideoFrame::blank(1, 1);
            assert!(frames.take(&mut f));
            assert_eq!((f.width, f.height), (320, 200));

            h.send(HostEvent::NextPattern);
            loop {
                if wrx.recv_timeout(Duration::from_secs(5)).unwrap() == EmuEvent::Frame {
                    break;
                }
            }
            assert!(frames.take(&mut f));
            assert_eq!((f.width, f.height), (720, 400));
            h.shutdown();
        }
    }
}
