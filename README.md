# dosy

A GPU-rendered MS-DOS emulator in Rust with a modern look and feel.

dosy is DOSBox-style: x86 real-mode CPU, PC hardware (PIC, PIT, keyboard,
CMOS, VGA, PC speaker, AdLib, Sound Blaster), BIOS and a built-in DOS kernel
and shell, all emulated at a high level. It mounts host directories, files
and `.zip` archives as DOS drives, and presents through wgpu with crisp
integer scaling, an optional CRT shader and a Ghostty-style config file.

## Status

Early development: workspace skeleton only. See [TASKS.md](TASKS.md) for
the roadmap and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the design.

## Build

Requires stable Rust (`rustup`); the toolchain is selected by
`rust-toolchain.toml`.

```
cargo build --workspace
cargo test --workspace
cargo run -p dosy
```

Contributing: [docs/WORKFLOW.md](docs/WORKFLOW.md).

## License

MIT
