//! dosy app shell: window, input, audio device, threads and CLI.
//! Kept thin: emulation lives in `dosy-core`, drawing in `dosy-render`.

mod app;
mod cli;
mod pattern;

use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = match cli::parse(std::env::args().skip(1)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("dosy: {e}\n\n{}", cli::USAGE);
            return ExitCode::from(2);
        }
    };
    if cli.version {
        println!("dosy {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if cli.help {
        println!("{}", cli::USAGE);
        return ExitCode::SUCCESS;
    }
    let (config, diags) = cli::load_config(&cli);
    for d in &diags {
        eprintln!("dosy: warning: {d}");
    }
    match app::run(config) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("dosy: {e}");
            ExitCode::FAILURE
        }
    }
}
