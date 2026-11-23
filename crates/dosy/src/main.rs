//! dosy app shell: window, input, audio device, threads and CLI.
//! Kept thin: emulation lives in `dosy-core`, drawing in `dosy-render`.

mod app;
mod cli;
mod emu;
mod input;
mod mailbox;
mod pacing;
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
    // RTC: host UTC for now (std has no local-offset API).
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let setup = match cli::emu_setup(&config, &cli, now) {
        Ok((s, warnings)) => {
            for w in &warnings {
                eprintln!("dosy: warning: {w}");
            }
            s
        }
        Err(e) => {
            eprintln!("dosy: {e}");
            return ExitCode::from(2);
        }
    };
    match app::run(config, setup) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("dosy: {e}");
            ExitCode::FAILURE
        }
    }
}
