use chrono::Local;
use clap::Parser;
use derive_more::Debug;
use log::{debug, info};

mod dbus;
mod freedesktop;
mod iced;
mod markup;
mod measuring_container;
mod notification;

/// A notification server using Eww to display notifications
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Log level: can be Off, Error, Warn, Info, Debug, or Trace
    #[arg(long, default_value_t = log::LevelFilter::Debug)]
    log: log::LevelFilter,
}

fn setup_logger(log_level: log::LevelFilter) -> Result<(), fern::InitError> {
    // Log to stderr and ~/.local/state/baelyks-notification-server.log
    let log_path = dirs::home_dir()
        .expect("Unable to get the home dir")
        .join(".local/state/")
        .join(env!("CARGO_PKG_NAME"))
        .with_extension("log");

    fern::Dispatch::new()
        .filter(|metadata| {
            metadata
                .target()
                .contains(&env!("CARGO_PKG_NAME").replace("-", "_"))
        })
        .format(|out, message, record| {
            out.finish(format_args!(
                "[{}] [{}] {}",
                record.level(),
                record.target(),
                message
            ))
        })
        .level(log_level)
        .chain(std::io::stderr())
        .chain(
            fern::Dispatch::new()
                .format(|out, message, _| {
                    out.finish(format_args!(
                        "[{}] {}",
                        Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                        message
                    ))
                })
                .chain(fern::log_file(&log_path)?),
        )
        .apply()?;

    info!(
        "Starting {} v{} with log level: {}",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        log_level
    );

    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    setup_logger(args.log)?;

    debug!("Command line arguments: {:#?}", args);
    iced::run()?;

    Ok(())
}
