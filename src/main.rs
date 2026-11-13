// Release builds are GUI applications without a console window; debug builds keep the
// console for development.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod instance;
mod logging;
mod paths;

use std::process::ExitCode;

fn main() -> ExitCode {
    logging::init(&paths::local_data_dir().join("logs"));

    let Some(_instance) = instance::acquire() else {
        tracing::info!("another instance is already running");
        return ExitCode::SUCCESS;
    };

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "Lilt started");
    ExitCode::SUCCESS
}
