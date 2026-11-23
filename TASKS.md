# Tasks

Protocol: `docs/WORKFLOW.md`. Design: `docs/ARCHITECTURE.md`.
Status: `todo` · `in-progress` · `in-review` · `done`. Start a task only when
all its dependencies are `done`.

## M0 — Skeleton

| ID | Title | Status | Depends on | Owner | File |
|----|-------|--------|------------|-------|------|
| 000 | Architecture docs and workspace skeleton | done | — | architect | [000](tasks/000-architecture.md) |

## M1 — Boot to shell prompt in a window

| ID | Title | Status | Depends on | Owner | File |
|----|-------|--------|------------|-------|------|
| 001 | CPU ALU and flags | done | 000 | worker-2 | [001](tasks/001-cpu-alu.md) |
| 002 | CPU state, decoder and control flow | done | 000 | worker-1 | [002](tasks/002-cpu-core.md) |
| 003 | Memory, Hardware bus and scheduler | done | 000 | worker-3 | [003](tasks/003-memory-bus.md) |
| 004 | VFS traits, DOS paths and MemDrive | done | 000 | worker-3 | [004](tasks/004-vfs-core.md) |
| 005 | Ghostty-style config parser | done | 000 | worker-1 | [005](tasks/005-config.md) |
| 006 | wgpu presenter with integer scaling | done | 000 | worker-2 | [006](tasks/006-render-core.md) |
| 007 | Complete 8086 instruction set | done | 001, 002 | worker-1 | [007](tasks/007-cpu-8086.md) |
| 008 | 8259 PIC and 8254 PIT | done | 003 | worker-3 | [008](tasks/008-pic-pit.md) |
| 009 | 8042 keyboard controller, port 61h, CMOS/RTC | done | 003 | worker-3 | [009](tasks/009-kbd-cmos.md) |
| 010 | VGA registers, DAC and text mode | done | 003 | worker-1 | [010](tasks/010-vga-text.md) |
| 011 | Host directory and single-file drives | done | 004 | worker-4 | [011](tasks/011-vfs-host.md) |
| 012 | App window: winit + renderer + config | done | 005, 006 | worker-2 | [012](tasks/012-app-window.md) |
| 013 | Machine loop, interrupts and HLE dispatch | done | 003, 007, 008 | worker-1 | [013](tasks/013-machine.md) |
| 014 | CPU conformance tests (SingleStepTests) | todo | 007 | — | [014](tasks/014-cpu-conformance.md) |
| 015 | BIOS HLE core | done | 002 | worker-3 | [015](tasks/015-bios-hle.md) |
| 016 | INT 10h video BIOS | done | 010, 015 | worker-1 | [016](tasks/016-int10.md) |
| 017 | DOS kernel core | done | 002 | worker-4 | [017](tasks/017-dos-kernel.md) |
| 018 | DOS file and directory I/O | done | 004, 017 | worker-4 | [018](tasks/018-dos-file-io.md) |
| 019 | COM/EXE loader and EXEC | done | 018 | worker-4 | [019](tasks/019-dos-exec.md) |
| 020 | DOS console I/O and character devices | todo | 016, 017 | — | [020](tasks/020-dos-console.md) |
| 021 | Built-in shell and MOUNT | todo | 011, 013, 019, 020 | — | [021](tasks/021-shell.md) |
| 022 | Emulation thread, pacing and CLI | in-progress | 012, 013 | worker-2 | [022](tasks/022-app-emu-thread.md) |
| 023 | Host keyboard to scancodes | done | 009, 022 | worker-3 | [023](tasks/023-keyboard-input.md) |
| 024 | Debug tracing and fault reports | todo | 013 | — | [024](tasks/024-debug-tracing.md) |

## M2 — Run a real-mode game (graphics + keyboard)

| ID | Title | Status | Depends on | Owner | File |
|----|-------|--------|------------|-------|------|
| 025 | 80186 and 386 real-mode instructions | done | 007 | worker-2 | [025](tasks/025-cpu-386-real.md) |
| 026 | VGA mode 13h and palette | done | 010, 016 | worker-1 | [026](tasks/026-vga-mode13h.md) |
| 027 | EGA/VGA planar modes, mode X, CGA graphics | todo | 026 | — | [027](tasks/027-vga-planar.md) |
| 028 | Zip drives and writable overlay | todo | 011 | — | [028](tasks/028-vfs-zip-overlay.md) |
| 029 | Batch files | todo | 021 | — | [029](tasks/029-shell-batch.md) |
| 030 | Remaining DOS services (FCB, IOCTL, INT 2Fh) | todo | 019 | — | [030](tasks/030-dos-misc.md) |
| 031 | INT 33h mouse driver and host mouse | todo | 015, 022 | — | [031](tasks/031-mouse.md) |
| 032 | XMS and EMS | todo | 017 | — | [032](tasks/032-xms-ems.md) |
| 033 | Private game smoke test | todo | 021, 026, 028, 029 | — | [033](tasks/033-game-smoke-test.md) |
| 046 | DOS kernel polish | in-progress | 018, 019 | worker-3 | [046](tasks/046-dos-kernel-polish.md) |

## M3 — Sound

| ID | Title | Status | Depends on | Owner | File |
|----|-------|--------|------------|-------|------|
| 034 | Audio mixer and PC speaker | todo | 008, 013 | — | [034](tasks/034-audio-mixer-speaker.md) |
| 035 | Host audio output (cpal) | todo | 022, 034 | — | [035](tasks/035-app-audio.md) |
| 036 | AdLib ports, OPL timers, MPU-401 stub | todo | 003 | — | [036](tasks/036-opl-ports-timers.md) |
| 037 | OPL2 FM synthesis | todo | 034, 036 | — | [037](tasks/037-opl-synth.md) |
| 038 | Sound Blaster and 8237 DMA | todo | 034 | — | [038](tasks/038-sound-blaster.md) |

## M4 — Polish / Ghostty look & feel

| ID | Title | Status | Depends on | Owner | File |
|----|-------|--------|------------|-------|------|
| 039 | Optional CRT shader | todo | 006 | — | [039](tasks/039-crt-shader.md) |
| 040 | Themes and window chrome | todo | 012 | — | [040](tasks/040-themes-chrome.md) |
| 041 | Vector-font text mode | todo | 006, 010 | — | [041](tasks/041-vector-text.md) |
| 042 | Keybinds and actions | todo | 005, 022 | — | [042](tasks/042-keybinds-actions.md) |
| 044 | Config parser polish | todo | 005 | — | [044](tasks/044-config-polish.md) |
| 045 | Chipset accuracy polish (PIC, PIT, 8042, RTC) | todo | 008, 009 | — | [045](tasks/045-pic-pit-polish.md) |
| 047 | BIOS keyboard polish | todo | 015 | — | [047](tasks/047-bios-keyboard-polish.md) |
| 048 | Local time | todo | 009, 011 | — | [048](tasks/048-local-time.md) |
| 049 | VGA accuracy polish | todo | 010 | — | [049](tasks/049-vga-polish.md) |
| 050 | VFS polish | todo | 011 | — | [050](tasks/050-vfs-polish.md) |

## M5 — Linux/macOS polish

| ID | Title | Status | Depends on | Owner | File |
|----|-------|--------|------------|-------|------|
| 043 | Linux/macOS polish and release artifacts | todo | 023, 035 | — | [043](tasks/043-platform-polish.md) |

Critical path: 001/002 → 007 → 013 → 021 (with 015 → 016 → 020 and
017 → 018 → 019 in parallel) → 029 → 033.
