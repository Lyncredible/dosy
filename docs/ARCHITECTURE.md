# dosy architecture

dosy is a DOSBox-style PC emulator: an x86 real-mode CPU, PC hardware, BIOS
and DOS implemented as high-level emulation (HLE) in Rust, presented through
a GPU renderer with a Ghostty-like look and config.

## Principles

- **Layered like libghostty.** A headless core library does all emulation;
  the renderer and the app are consumers.
- **Deterministic core.** Same config + same drive contents + same input
  events (timestamped in emulated time) + same RTC seed → same frames,
  audio and memory. No wall clock, threads, randomness or OS calls in
  `dosy-core` (file access goes through `dosy-vfs` drives).
- **Thin platform shell.** Only the `dosy` binary touches windows, GPU
  surfaces, audio devices, threads and the host clock.
- **Static dispatch on hot paths.** CPU and bus are generic/concrete, not
  `dyn`. Trait objects are fine for drives, files and sinks.

## Crates

```
crates/
  dosy-core    cpu, memory, devices, bios, dos kernel, shell   deps: dosy-vfs, log
  dosy-vfs     Drive/DosFile traits, host dir / file / zip /   deps: zip (deflate only)
               overlay drives, 8.3 name mapping
  dosy-config  Ghostty-style config parsing → typed Config     deps: none
  dosy-render  wgpu presenter: scaling, palette, CRT, themes,  deps: dosy-core, wgpu, pollster
               vector text
  dosy         app: winit, input mapping, cpal, threads, CLI   deps: all of the above
```

Rules:

- `dosy-core`, `dosy-vfs`, `dosy-config` never depend on winit, wgpu, cpal or
  platform crates, and must test headless on every OS.
- `dosy-render` never depends on winit. The app creates the
  `wgpu::Surface<'static>` from its window and hands it to the renderer
  with the surface size.
- `dosy-core` does not depend on `dosy-config`; the app maps `Config` to
  `MachineConfig` (core) and `RenderConfig` (render).
- New third-party deps must be permissively licensed (MIT/Apache/BSD/Zlib);
  note them in the PR.

## dosy-core modules

```
bus.rs       Bus trait (CPU's view: memory + I/O), FlatBus  [exists]
hle.rs       HleId ranges, HleResult, ROM stub layout       [exists]
video.rs     VideoFrame, TextScreen, Rgb8                  [exists]
audio.rs     AudioSink, StereoFrame; mixer.rs later        [exists]
cpu/         Cpu state, decode, exec, alu.rs (pure fns), Cpu::step
memory.rs    Memory: 1 MiB + HMA, A20 gate, ROM ranges, VGA window hook
hw/          Hardware (impl Bus): Memory + devices, ports.rs dispatch, Scheduler
  pic pit kbd cmos speaker dma opl sb mpu   one file each
  vga/       registers, VRAM (4 planes), text/13h/planar rasterizers, font
bios/        BDA, IVT + ROM stub install, INT 08/09/10/11/12/15/16/1A/33
dos/         kernel: MCB, PSP, env, handles, drives, INT 20/21/2F, exec, XMS/EMS
shell/       built-in COMMAND.COM: line input, builtins, batch interpreter
machine.rs   Machine: Cpu + Hardware + Dos + Shell; run_for, input, frames, audio
```

## Key interfaces

### Bus (`bus.rs`, exists)

```rust
pub trait Bus {
    fn read8(&mut self, addr: u32) -> u8;           // linear address
    fn write8(&mut self, addr: u32, val: u8);
    fn io_in8(&mut self, port: u16) -> u8;
    fn io_out8(&mut self, port: u16, val: u8);
    // provided: read16/32, write16/32, io_in16, io_out16 (little-endian),
    // load_rom(addr, &[u8]): host write bypassing ROM protection (HLE stubs)
}
```

The CPU forms linear addresses; `Memory` applies A20 masking. `Hardware`
implements `Bus` for the real machine; CPU tests use a flat test bus.

### CPU (`cpu/`)

```rust
pub struct Cpu { pub regs: Regs, pub sregs: SegRegs, pub ip: u32, pub flags: Flags, pub halted: bool, pub int_shadow: bool, .. }
pub enum Step { Ok, Halt, Callback(HleId), Fault(CpuFault) }
impl Cpu {
    pub fn reset(&mut self);                                   // F000:FFF0
    pub fn step<B: Bus>(&mut self, bus: &mut B) -> (Step, u32); // one instruction, cost in cycles
    pub fn interrupt<B: Bus>(&mut self, bus: &mut B, vector: u8); // push FLAGS/CS/IP, clear IF/TF
}
```

- Scope: 8086 + 80186 + common 386 real-mode (66/67 prefixes, 32-bit regs,
  FS/GS, MOVZX/SETcc/…). No protected mode, no FPU (BIOS reports none).
- `CpuModel` (8086/186/386, default 386; config `cpu`) only changes what
  detection code sees: FLAGS 12..15, PUSH SP, shift-count masking, `0F`.
  Fault-class exceptions (#UD = INT 6, BOUND = INT 5, #DE) push the
  faulting instruction's address.
- `Flags` is `u32` (masks in `cpu/flags.rs`). `int_shadow` is set after
  STI / MOV SS / POP SS: the machine skips IRQs for that step.
- On `Fault`, IP is rewound to the instruction start (incl. prefixes).
- `alu.rs` is pure functions `(a, b, flags) -> (result, flags)` so it can be
  built and tested independently.
- **Callback opcode** `FE 38 lo hi` (an unused `FE /7` form, as in DOSBox)
  returns `Step::Callback(id)`. All HLE entry points are ROM stubs using it.
- Cycle cost follows DOSBox semantics (~1 per instruction, more for
  mul/div/string iterations); `cycles` config is "instructions per ms".

### Hardware, devices, scheduler (`hw/`)

```rust
pub struct IoCtx<'a> { pub now: u64, pub irq: &'a mut IrqLines, pub sched: &'a mut Scheduler }
pub trait IoDevice {
    fn read(&mut self, port: u16, ctx: &mut IoCtx) -> u8;
    fn write(&mut self, port: u16, val: u8, ctx: &mut IoCtx);
    fn event(&mut self, _kind: u32, _ctx: &mut IoCtx) {} // scheduled event fired
}
```

- `Hardware` owns `Memory` and each device as a concrete field; `ports.rs`
  is one `match` routing port ranges to devices. Unclaimed ports read `0xFF`,
  writes are ignored (logged once).
- Devices raise/lower IRQs through `IrqLines`; `Hardware` forwards rising
  edges to the PIC after each call (`pulse` = one edge). The PIC latches IRR
  on the edge; the machine uses `hw.pic.pending_vector()` /
  `hw.pic.acknowledge()`. Devices never call each other directly; cross-device
  wiring (PIT ch2 → speaker, 8042 → A20) is done in `Hardware`.
- `Scheduler`: min-heap of `(due_ns, DeviceId, kind)`. `now` is emulated
  nanoseconds.
- MMIO: `Memory` forwards `A0000–BFFFF` to the VGA (planar/chain-4 logic)
  via `trait Mmio` (`None`/`false` = plain RAM). `Hardware` owns the VGA
  as `vga: Vga` (no generic). `Hardware::start_video()` (called by
  `Machine::new`) schedules vretrace events; at each one the VGA renders a
  `VideoFrame`, taken with `hw.vga.take_frame()`. `Hardware::new()` leaves
  the scheduler empty so device tests stay idle.
- `Hardware`: `now()`, `advance_cycles(n)`, `advance_to(ns)`,
  `cycles_until(ns)`, `set_cycles_per_ms`, `fire_due()`. `DeviceId` is a
  newtype with reserved constants per device. Ports and events route in
  `hw/ports.rs` (one arm per device, `ctx!(self)` builds the `IoCtx`).

### HLE (BIOS / DOS / shell) (`hle.rs`, exists)

```rust
pub struct HleId(pub u16);           // high byte: 0 BIOS, 1 DOS, 2 shell, 3 drivers
pub enum HleResult { Done, Retry }
pub enum StubKind { IretSti, Iret, Retf, Loop }
pub fn install_stub<B: Bus>(bus: &mut B, id: HleId, kind: StubKind) -> u32;  // F000:(1000h + 8*id)
```

- Each subsystem allocates ids in its own range, so no shared enum to fight
  over. Stub addresses are fixed by id; no allocator.
- At boot, each subsystem installs its stubs and points IVT entries at them,
  so programs can hook and chain vectors.
- Handlers are generic over `Bus`, e.g.
  `fn int16<B: Bus>(cpu: &mut Cpu, bus: &mut B) -> HleResult`; DOS handlers
  also take `&mut Dos`. They unit-test on `FlatBus` without a machine.
- `machine.rs` dispatches `Step::Callback(id)` on `id.subsystem()`.
- BIOS: `bios::install(bus)` at power-on (also programs PIC IMR and PIT
  ch0), `bios::handle(id, cpu, bus)`. Handlers in `Iret` stubs return flags
  via `bios::set_caller_flag` (patches the FLAGS image the INT pushed).
- **Never block the host.** A handler that must wait (INT 16h/00 with an
  empty buffer, INT 21h/01) returns `Retry`: the machine rewinds IP to the
  callback, sets IF and halts; the next IRQ wakes it and the callback re-runs.
- HLE code may call other HLE code directly (e.g. DOS CON output calls the
  INT 10h teletype function) instead of re-entering the CPU.

### DOS kernel (`dos/`)

- Guest-visible structures live in guest memory: PSP, MCB chain,
  environment, DTA, List of Lists, InDOS flag. Target ≥ 600 KiB free
  conventional memory.
- Host-side state: `drives: [Option<Box<dyn Drive>>; 26]`, current drive and
  per-drive cwd, open-file table (`Box<dyn DosFile>`), JFT in PSP, process
  stack for EXEC/terminate.
- EXE (MZ, relocations) and COM loaders; INT 21h/4B00 and 4B03.
- `Dos::install(bus)` sets up vectors 20h–24h/28h/2Fh, kernel data at
  segment `0080h` (List of Lists at `:0026`, SDA at `:0100`, `:0180–:1800`
  free for SFT/CDS), the MCB chain from `0200h`, the master environment and
  the root (shell) PSP. The machine calls `Dos::callback(id, cpu, bus)`.
- Current PSP, DTA, return code and break flag live in the SDA (guest
  memory). Handlers report CF through the IRET frame at SS:SP.
- Files: `Dos::mount(index, drive)`; host SFT (`Dos::sft`, 40 entries,
  refcounted for dup) indexed by the PSP's JFT; entries 0–2 are AUX/CON/PRN.
  Guest paths are resolved by `Dos::resolve` (drive, cwd, `.`/`..`, over-long
  components truncated to 8.3). FindFirst state lives in the DTA, with the
  search directory as an id into a host-side table.
- `Dos::exec(cpu, bus, path, &ExecParams)` (INT 21h/4B00; also 4B01/4B03):
  the CPU's CS:IP/SS:SP/FLAGS at the call become the parent's resume point
  (child PSP INT 22h + pushed `Process { psp, ss, sp, flags }`). The child
  inherits the parent's JFT. `Dos::terminate` pops it, closes the child's
  handles, frees its blocks, restores INT 22h–24h and resumes the parent.
- Top-level exit: terminating with no `Process` left, or reaching the root
  PSP's INT 22h stub, sets `Dos::exit_code`; `run_for` then returns
  `RunExit::Exited(code)` (sticky). The shell (021) gives its children its
  own resume point, so this only fires when the shell itself exits or a
  program was started with `Machine::exec` (no shell).

### Shell (`shell/`)

COMMAND.COM is a native state machine running as a DOS process whose code is
a ROM stub `L: FE 38 SHELL; JMP L`. Each callback advances it: read a line
(DOS line input, `Retry` while waiting), run a builtin, or start a child via
HLE EXEC — which switches CS:IP to the child; on child exit control returns
to the stub and the shell resumes. Batch files are a stack of interpreter
frames. Config `autoexec` lines run as an implicit batch at boot.

The app boots with `Machine::new`, `mount`s, `set_rtc`, then
`Machine::start_shell(&autoexec)`. Z: is a built-in drive; the shell starts
there when C: isn't mounted. `RunExit::Exited` means the shell itself exited.

### Video output (`video.rs`, exists)

The VGA device owns 256 KiB VRAM + registers and produces a `VideoFrame` at
each vertical retrace:

- `pixels`: indexed, row-major (`320x200`, `640x480`, `720x400`, …);
  text modes are rasterized by the core with an 8x16/8x8 font, so the
  renderer only needs the indexed path.
- `palette: [Rgb8; 256]` (DAC), `display_aspect` (usually 4:3).
- `text: Option<TextScreen>` (cells, cursor, 16-color palette, blink) for
  vector-font rendering.

### Audio output (`audio.rs`, exists)

```rust
pub trait AudioSink: Send { fn sample_rate(&self) -> u32; fn push(&mut self, frames: &[[f32; 2]]); }
```

Each sound device renders lazily up to `now` (on state-changing port writes
and at the end of every `run_for` slice) into its own channel; the mixer
resamples to the sink rate and pushes. Tests use a `Vec`-backed sink.

### Machine

```rust
impl Machine {
    pub fn new(cfg: MachineConfig) -> Self;                 // power-on + BIOS/DOS/shell boot
    pub fn mount(&mut self, letter: char, drive: Box<dyn Drive>);
    pub fn run_for(&mut self, cycles: u64) -> RunExit;      // Budget | Halted | Fault(..)
    pub fn key_event(&mut self, scancode: &[u8]);           // set-1 bytes, incl. E0 prefixes
    pub fn mouse_event(&mut self, dx: i32, dy: i32, buttons: u8);
    pub fn frame(&self) -> Option<&VideoFrame>;            // latest completed frame
    pub fn set_audio_sink(&mut self, sink: Box<dyn AudioSink>);
    pub fn set_cycles_per_ms(&mut self, n: u32);
    pub fn set_rtc(&mut self, unix_secs: i64);              // host time injected, not read
    pub fn set_handler(&mut self, s: Subsystem, h: HleHandler); // callback routing
}
```

- `RunExit`: `Budget`, `Halted` (HLT with IF=0), `Fault(CpuFault)`,
  `UnhandledCallback(HleId)`, `Exited(u8)`.
- `Machine::dos` owns the kernel and the only drive table
  (`Machine::mount` forwards to it); DOS callbacks go to it unless a
  handler is registered for `Subsystem::Dos`.
- `HleHandler = Box<dyn FnMut(HleId, &mut Cpu, &mut Hardware) -> HleResult + Send>`,
  one per subsystem; `Machine` stays `Send`. Subsystems with host state
  (DOS, shell) may replace this with direct calls on `Machine` fields.

## Timing model

- Master time is **emulated nanoseconds** (`u64`). CPU speed is a knob:
  `cycles_per_ms` (config `cycles`, default 3000) maps instruction cost to
  time, so changing it never breaks device timing.
- `run_for(cycles)` loop: run instructions until the budget or the next
  scheduler deadline, advancing `now` per instruction; fire due events; take
  pending IRQs (PIC → `Cpu::interrupt`) when IF=1; repeat. HLT skips ahead to
  the next event.
- PIT (1.193182 MHz) counters are computed lazily from `now`; channel 0
  schedules IRQ0, channel 2 drives the speaker (`hw.pit.ch2_out(now)`,
  `ch2_reload()`, `hw.speaker_enabled()`). Port 61h lives in `Hardware`.
- Keyboard: `Machine::key_event` → `hw.push_scancodes` → 8042 queue →
  port 60h + IRQ1 → BIOS INT 09h. 8042 output-port A20 and port 92h share
  `mem.set_a20`.
- VGA schedules display/retrace edges (~70 Hz) so port 3DAh status is
  time-accurate.
- RTC: CMOS time = injected base (`Machine::set_rtc`) + emulated `now`;
  `set_rtc` also resyncs the BIOS tick count.

## Threading model (app)

```
main thread                    emulation thread               cpal thread
-----------                    ----------------               -----------
winit event loop   --HostEvent (mpsc)-->  owns Machine
renderer (wgpu)    <--latest frame (mailbox) + EventLoopProxy wake
                                          --[f32;2] SPSC ring-->  device callback
```

- **Emulation thread** owns `Machine`. Each ~1 ms tick: drain `HostEvent`s
  (keys as set-1 scancodes, mouse, pause/reset/mount/quit, config changes),
  run `cycles_per_ms × elapsed_ms` cycles (capped to avoid spiral of death),
  sleep to the next tick.
- **Frames**: latest-wins mailbox (`Mutex` around a double buffer; swap, no
  per-frame allocation). The emu thread wakes winit via `EventLoopProxy`;
  the main thread uploads and presents. Rendering never stalls emulation.
- **Audio**: the app's `AudioSink` writes into a lock-free SPSC ring
  (e.g. `rtrb`); the cpal callback drains it and outputs silence on underrun.
  Ring fill level may nudge pacing by a fraction of a percent.
- wgpu/winit stay on the main thread (required on macOS).
- wgpu instances use `dosy_render::default_backends()` (`WGPU_BACKEND`, else
  Vulkan/Metal/DX12); GL probing can hang adapter creation on Windows.

## Renderer (`dosy-render`)

Pipeline: upload `pixels` as `R8Uint` texture + palette as 256x1 texture →
palette-lookup + scale pass into the window (integer scale by default,
aspect-correct, `fit`/`stretch` optional, nearest filtering) → optional CRT
pass → present. Padding area is cleared to the theme background. Layout math
(scale factor, viewport rect) is pure and unit-tested; GPU tests are
`#[ignore]` in CI. Vector text mode renders `TextScreen` cells with a glyph
atlas from a configured font instead of the indexed frame.

API: `Renderer::new(&Instance, Surface<'static>, (w, h))` (blocks via
pollster), `set_config(RenderConfig { background, padding, scale })`,
`resize`, `upload(&VideoFrame)`, `render()`. `Presenter` is the
surface-independent part (draws into any `TextureView`); `layout::viewport`
is the pure scaling math. Integer scale works on the aspect-corrected size
(width kept, height = width·aspect_h/aspect_w, e.g. 320x200 → 320x240).

## dosy-vfs

```rust
pub trait Drive: Send {
    fn info(&self) -> DriveInfo;                              // label, read_only, sizes
    fn open(&mut self, p: &DosPath, mode: OpenMode) -> Result<Box<dyn DosFile>, DosError>;
    fn create(&mut self, p: &DosPath, attr: Attr) -> Result<Box<dyn DosFile>, DosError>;
    fn stat(&mut self, p: &DosPath) -> Result<DirEntry, DosError>;
    fn read_dir(&mut self, dir: &DosPath) -> Result<Vec<DirEntry>, DosError>;
    fn remove(&mut self, p: &DosPath) -> Result<(), DosError>;
    fn rename(&mut self, from: &DosPath, to: &DosPath) -> Result<(), DosError>;
    fn mkdir(&mut self, p: &DosPath) -> Result<(), DosError>;
    fn rmdir(&mut self, p: &DosPath) -> Result<(), DosError>;
    fn set_attr(&mut self, p: &DosPath, attr: Attr) -> Result<(), DosError>;
}
pub trait DosFile: Send {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, DosError>;
    fn write(&mut self, buf: &[u8]) -> Result<usize, DosError>;
    fn seek(&mut self, pos: SeekFrom) -> Result<u64, DosError>;
    fn set_len(&mut self, len: u64) -> Result<(), DosError>;
    fn mtime(&self) -> DosDateTime;
    fn set_mtime(&mut self, t: DosDateTime) -> Result<(), DosError>;
}
```

- `DosPath`: drive-relative absolute path of validated uppercase 8.3
  components. `DosError` maps 1:1 to DOS error codes. Wildcard matching
  (`*.EXE`, `?`, FCB semantics via `DosPattern`) lives here; FindFirst/Next
  state lives in the kernel. `read_dir` omits `.`/`..`.
- Drives: `MemDrive` (tests), `HostDrive` (directory; also single-file
  mounts), `ZipDrive` (read-only; index at mount, entries inflated into memory
  on open), `OverlayDrive` (read-only lower + writable upper, whiteouts).
- 8.3 mapping (HostDrive/ZipDrive): valid 8.3 names are uppercased; others
  become `NAME~N.EXT`, `N` assigned by sorted host name per directory, so
  mapping is deterministic (`shortname::map_dir`). Lookup is
  case-insensitive on every host. HostDrive canonicalizes resolved paths and
  rejects anything outside the mount root (symlinks). Host times are UTC.
- `dosy_vfs::open_mount(path, &MountOptions) -> io::Result<Box<dyn Drive>>`
  picks dir / zip / file. The shell's `MOUNT` uses it.
- Seeks use DOS 32-bit wrapping (`dos_seek`): seeking before the start
  succeeds; the following read sees EOF and a write fails.

## Configuration (`dosy-config`)

Ghostty syntax: `key = value` per line, `#` comments, repeated keys append to
lists, empty value resets to default, `config-file = path` includes
(`?path` = optional). Location: `$XDG_CONFIG_HOME/dosy/config`
(default `~/.config/dosy/config`), `%APPDATA%\dosy\config` on Windows,
`~/Library/Application Support/dosy/config` on macOS. CLI `--key=value`
overrides the file. Unknown keys warn, never fail. Initial keys: `cycles`,
`mount` (`C:path`), `autoexec`, `scale`, `aspect`, `crt`, `theme`,
`background`, `window-padding-x/y`, `window-padding-color`, `text-render`
(`bitmap`/`vector`), `font-family`, `font-size`, `audio`, `audio-volume`,
`keybind`.

## Testing

- Unit tests per module; ALU and decoder against hand-written vectors.
- CPU conformance: SingleStepTests (8088 / 80286 JSON) downloaded on demand,
  run when `DOSY_CPU_TESTS` points at them, skipped otherwise.
- Headless integration tests drive `Machine` with a `MemDrive` and
  hand-assembled guest programs (byte arrays with comments); assert on
  `TextScreen`, memory or frame hashes.
- Private real-game smoke test reads a zip path from `DOSY_SMOKE_ZIP` and is
  skipped when unset. Never commit DOS software or name private test assets.
- Diagnostics use the `log` facade; unimplemented opcodes/interrupts warn
  once and surface as `RunExit::Fault` with a register dump.
