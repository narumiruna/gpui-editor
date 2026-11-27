use std::{env, process::ExitCode};

const USAGE: &str = "Usage: gpui-editor\nLaunch the desktop app. Choose a folder inside the app.";

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    match args.next() {
        None => launch(),
        Some(arg) if (arg == "--help" || arg == "-h") && args.next().is_none() => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(_) => {
            eprintln!("This command does not accept file or folder paths.\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn launch() -> ExitCode {
    gpui_editor::app::run();
    ExitCode::SUCCESS
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn launch() -> ExitCode {
    eprintln!("GPUI Editor requires macOS or Windows.");
    ExitCode::FAILURE
}
