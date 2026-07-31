use std::process::ExitCode;

use vibe_easytier_service::service::{HostMode, ServiceOptions};

fn main() -> ExitCode {
    let options = match ServiceOptions::parse(std::env::args_os().skip(1)) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };

    let result: Result<(), Box<dyn std::error::Error>> = match options.mode {
        HostMode::Console => {
            vibe_easytier_service::service::run_console(options).map_err(Into::into)
        }
        HostMode::Service => run_service_mode(options),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Vibe EasyTier service failed: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(windows)]
fn run_service_mode(options: ServiceOptions) -> Result<(), Box<dyn std::error::Error>> {
    vibe_easytier_service::service::windows::dispatch_service_with_options(options)?;
    Ok(())
}

#[cfg(not(windows))]
fn run_service_mode(_options: ServiceOptions) -> Result<(), Box<dyn std::error::Error>> {
    Err(Box::new(
        vibe_easytier_service::service::ServiceError::UnsupportedPlatform,
    ))
}
