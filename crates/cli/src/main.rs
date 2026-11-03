//! Fastcart administrative CLI.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "fastcart", version, about = "Fastcart commerce platform CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Start the HTTP server.
    Serve {
        /// Configuration file path.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Override the configured bind address.
        #[arg(long)]
        host: Option<String>,
        /// Override the configured bind port.
        #[arg(long)]
        port: Option<u16>,
    },
    /// Create local directories and an initial configuration file.
    Init {
        /// Target installation directory.
        #[arg(long, default_value = ".")]
        dir: PathBuf,
    },
    /// Validate configuration and print environment details.
    Doctor {
        /// Configuration file path.
        #[arg(long)]
        config: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Serve { config, host, port } => {
            let mut config = fastcart_server::Config::load_from(config)?;
            if let Some(host) = host {
                config.server.host = host;
            }
            if let Some(port) = port {
                config.server.port = port;
            }
            fastcart_server::run(config).await
        }
        Command::Init { dir } => init(&dir),
        Command::Doctor { config } => doctor(config),
    }
}

fn init(dir: &Path) -> Result<()> {
    for path in [
        "data",
        "data/stores",
        "data/assets",
        "data/backups",
        "config",
    ] {
        fs::create_dir_all(dir.join(path))
            .with_context(|| format!("failed to create {}", dir.join(path).display()))?;
    }

    let config_path = dir.join("config/config.toml");
    if !config_path.exists() {
        fs::write(
            &config_path,
            include_str!("../../../config/config.example.toml"),
        )
        .with_context(|| format!("failed to write {}", config_path.display()))?;
        println!("created {}", config_path.display());
    }

    println!("initialized {}", dir.display());
    Ok(())
}

fn doctor(config: Option<PathBuf>) -> Result<()> {
    let config = fastcart_server::Config::load_from(config)?;

    println!("fastcart {}", env!("CARGO_PKG_VERSION"));
    println!("listen: {}", config.listen_address());
    println!(
        "storefront: {} ({} workers, {} queued)",
        config.storefront.runtime, config.storefront.worker_count, config.storefront.queue_capacity
    );
    println!("configuration ok");
    Ok(())
}
