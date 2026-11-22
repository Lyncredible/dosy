//! dosy app shell: window, input, audio device, threads and CLI.
//! Placeholder until tasks 012 / 022.

fn main() {
    let version = env!("CARGO_PKG_VERSION");
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("dosy {version}");
        return;
    }
    println!(
        "dosy {version} (core {}, vfs {}, config {}, render {})",
        dosy_core::VERSION,
        dosy_vfs::VERSION,
        dosy_config::VERSION,
        dosy_render::VERSION,
    );
}
