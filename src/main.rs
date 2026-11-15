// Release builds are GUI applications without a console window; debug builds keep the
// console for development.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod ambient;
mod app;
mod appearance;
mod artwork;
mod calendar;
mod clipboard;
mod clock;
mod controls;
mod dialogs;
mod fonts;
mod hotkey;
mod identity;
mod instance;
mod logging;
mod media;
mod paths;
mod settings;
#[cfg(test)]
mod snapshots;
mod startup;
mod system;
mod tasks;
mod window;

mod ui {
    slint::include_modules!();
}

mod updates;

use std::process::ExitCode;

fn main() -> ExitCode {
    logging::init(&paths::local_data_dir().join("logs"));

    let Some(_instance) = instance::acquire() else {
        tracing::info!("another instance is already running");
        window::signal_running_instance();
        return ExitCode::SUCCESS;
    };

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "Lilt started");
    match app::run() {
        Ok(()) => {
            tracing::info!("Lilt exited");
            ExitCode::SUCCESS
        }
        Err(err) => {
            tracing::error!(%err, "Lilt stopped because of an error");
            ExitCode::FAILURE
        }
    }
}
