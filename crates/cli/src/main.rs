//! Fastcart administrative CLI.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use fastcart_infrastructure::{DatabaseOptions, StoreDatabaseManager};
use fastcart_kernel::{Pagination, StoreId, UserId};
use fastcart_platform::{
    AuthService, CreateStoreInput, CreateUserInput, PlatformStore, StoreProvisioningService,
    StoreStatus,
};

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
    /// Create a consistent backup of a store database.
    Backup {
        /// Store identifier.
        #[arg(long)]
        store: String,
        /// Destination file for the backup.
        #[arg(long)]
        output: PathBuf,
        /// Configuration file path.
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Verify a backup file without modifying it.
    Verify {
        /// Backup file to verify.
        #[arg(long)]
        file: PathBuf,
    },
    /// Restore a store database from a verified backup.
    Restore {
        /// Store identifier.
        #[arg(long)]
        store: String,
        /// Backup file to restore.
        #[arg(long)]
        file: PathBuf,
        /// Configuration file path.
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Manage stores.
    Store {
        /// Configuration file path.
        #[arg(long)]
        config: Option<PathBuf>,
        #[command(subcommand)]
        command: StoreCommand,
    },
    /// Manage platform users.
    User {
        /// Configuration file path.
        #[arg(long)]
        config: Option<PathBuf>,
        #[command(subcommand)]
        command: UserCommand,
    },
}

#[derive(Debug, Subcommand)]
enum StoreCommand {
    /// Create, provision and optionally assign an owner.
    Create {
        /// Display name.
        #[arg(long)]
        name: String,
        /// URL-safe slug.
        #[arg(long)]
        slug: String,
        /// Primary hostname.
        #[arg(long)]
        domain: String,
        /// Store locale.
        #[arg(long, default_value = "en-US")]
        locale: String,
        /// Display currency.
        #[arg(long, default_value = "USD")]
        currency: String,
        /// IANA time zone.
        #[arg(long, default_value = "UTC")]
        timezone: String,
        /// Existing user email to grant the owner role.
        #[arg(long)]
        owner_email: Option<String>,
    },
    /// List registered stores.
    List,
    /// Retry provisioning for an existing store.
    Provision {
        /// Store identifier.
        store_id: String,
        /// Existing user email to grant the owner role.
        #[arg(long)]
        owner_email: Option<String>,
    },
    /// Suspend a store.
    Suspend {
        /// Store identifier.
        store_id: String,
    },
    /// Resume a suspended store.
    Resume {
        /// Store identifier.
        store_id: String,
    },
}

#[derive(Debug, Subcommand)]
enum UserCommand {
    /// Create a platform user.
    Create {
        /// Email address.
        #[arg(long)]
        email: String,
        /// Initial password.
        #[arg(long, env = "FASTCART_PASSWORD", hide_env_values = true)]
        password: String,
        /// Given name.
        #[arg(long)]
        first_name: Option<String>,
        /// Family name.
        #[arg(long)]
        last_name: Option<String>,
    },
    /// Grant a user a role in a store.
    Grant {
        /// Email address.
        #[arg(long)]
        email: String,
        /// Store identifier.
        #[arg(long)]
        store_id: String,
        /// Roles to assign (repeat the flag for multiple roles).
        #[arg(long = "role", required = true)]
        roles: Vec<String>,
    },
}

struct CliContext {
    platform: PlatformStore,
    databases: Arc<StoreDatabaseManager>,
}

impl CliContext {
    async fn open(config: Option<PathBuf>) -> Result<Self> {
        let config = fastcart_server::Config::load_from(config)?;
        let options = DatabaseOptions {
            max_connections: config.database.max_connections,
            busy_timeout_ms: config.database.busy_timeout_ms,
            wal: config.database.journal_mode.eq_ignore_ascii_case("wal"),
        };
        let platform = PlatformStore::open(&config.database.platform_path, &options)
            .await
            .context("failed to open the platform database")?;
        let databases = Arc::new(StoreDatabaseManager::new(
            &config.database.stores_dir,
            options,
        ));
        Ok(Self {
            platform,
            databases,
        })
    }

    async fn user_id_by_email(&self, email: &str) -> Result<UserId> {
        self.platform
            .get_user_by_email(email)
            .await
            .context("failed to look up user")?
            .map(|user| user.id)
            .ok_or_else(|| {
                anyhow::anyhow!("user not found: {email}; create it with `fastcart user create`")
            })
    }
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
        Command::Backup {
            config,
            store,
            output,
        } => backup_command(config, store, output).await,
        Command::Verify { file } => verify_command(file).await,
        Command::Restore {
            config,
            store,
            file,
        } => restore_command(config, store, file).await,
        Command::Store { config, command } => store_command(config, command).await,
        Command::User { config, command } => user_command(config, command).await,
    }
}

async fn backup_command(config: Option<PathBuf>, store: String, output: PathBuf) -> Result<()> {
    let context = CliContext::open(config).await?;
    let store_id = StoreId::from_string(store);
    let store = context
        .platform
        .get_store(&store_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("store not found: {store_id}"))?;
    if store.status == StoreStatus::Deleted {
        anyhow::bail!("store {store_id} is deleted and cannot be backed up");
    }
    let database_path = context.databases.database_path(&store_id)?;
    if !tokio::fs::try_exists(&database_path).await.unwrap_or(false) {
        anyhow::bail!(
            "store database not found at {}; provision the store first",
            database_path.display()
        );
    }

    let manifest = context.databases.backup(&store_id, &output).await?;
    StoreDatabaseManager::verify_backup(&output).await?;

    println!("backup: {}", output.display());
    println!(
        "manifest: {}",
        StoreDatabaseManager::manifest_path(&output).display()
    );
    println!("checksum: sha256:{}", manifest.sha256);
    println!("bytes: {}", manifest.bytes);
    Ok(())
}

async fn verify_command(file: PathBuf) -> Result<()> {
    let manifest = StoreDatabaseManager::verify_backup_with_manifest(&file).await?;
    println!("verified: {}", file.display());
    match manifest {
        Some(manifest) => {
            println!("store: {}", manifest.store_id);
            println!("created_at: {}", manifest.created_at);
            println!("bytes: {}", manifest.bytes);
            println!("checksum: sha256:{}", manifest.sha256);
            println!(
                "manifest: {}",
                StoreDatabaseManager::manifest_path(&file).display()
            );
        }
        None => println!("manifest: not found (integrity check passed)"),
    }
    Ok(())
}

async fn restore_command(config: Option<PathBuf>, store: String, file: PathBuf) -> Result<()> {
    let context = CliContext::open(config).await?;
    let store_id = StoreId::from_string(store);
    let store = context
        .platform
        .get_store(&store_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("store not found: {store_id}"))?;
    let destination = context.databases.database_path(&store_id)?;
    let provisioning =
        StoreProvisioningService::new(context.platform.clone(), context.databases.clone());

    let suspended = provisioning.suspend(&store_id).await?;
    println!("suspended store {} (was {})", suspended.id, store.status);
    context.databases.close(&store_id).await;
    println!("closed store database pool for {}", store_id);

    context
        .databases
        .restore(&store_id, &file)
        .await
        .with_context(|| {
            format!(
                "failed to restore store {store_id} from {}; it remains suspended",
                file.display()
            )
        })?;

    let previous = destination.with_extension("sqlite.pre-restore");
    println!("restored {} from {}", destination.display(), file.display());
    if previous.exists() {
        println!("previous database saved to {}", previous.display());
    }
    println!(
        "store {} is {}; resume it with `fastcart store resume {}`",
        suspended.id, suspended.status, suspended.id
    );
    Ok(())
}

async fn store_command(config: Option<PathBuf>, command: StoreCommand) -> Result<()> {
    let context = CliContext::open(config).await?;
    let provisioning =
        StoreProvisioningService::new(context.platform.clone(), context.databases.clone());
    match command {
        StoreCommand::Create {
            name,
            slug,
            domain,
            locale,
            currency,
            timezone,
            owner_email,
        } => {
            let input =
                CreateStoreInput::new(name, slug, domain).with_region(locale, currency, timezone);
            let store = context.platform.create_store(input).await?;
            let owner = match owner_email {
                Some(email) => Some(context.user_id_by_email(&email).await?),
                None => None,
            };
            let report = provisioning.provision(&store.id, owner.as_ref()).await?;
            println!(
                "created store {} ({}) status={}",
                report.store.id, report.store.slug, report.store.status
            );
        }
        StoreCommand::List => {
            let mut offset = 0;
            loop {
                let page = context
                    .platform
                    .list_stores(Pagination { limit: 100, offset })
                    .await?;
                for store in &page.data {
                    println!(
                        "{}\t{}\t{}\t{}",
                        store.id,
                        store.slug,
                        store.status,
                        store.primary_domain.as_deref().unwrap_or("-")
                    );
                }
                offset += page.data.len() as i64;
                if offset >= page.count || page.data.is_empty() {
                    break;
                }
            }
        }
        StoreCommand::Provision {
            store_id,
            owner_email,
        } => {
            let store_id = StoreId::from_string(store_id);
            let owner = match owner_email {
                Some(email) => Some(context.user_id_by_email(&email).await?),
                None => None,
            };
            let report = provisioning.provision(&store_id, owner.as_ref()).await?;
            println!(
                "provisioned {} status={} steps={:?}",
                report.store.id, report.store.status, report.steps
            );
        }
        StoreCommand::Suspend { store_id } => {
            let store = provisioning
                .suspend(&StoreId::from_string(store_id))
                .await?;
            println!("store {} status={}", store.id, store.status);
        }
        StoreCommand::Resume { store_id } => {
            let store = provisioning.resume(&StoreId::from_string(store_id)).await?;
            println!("store {} status={}", store.id, store.status);
        }
    }
    Ok(())
}

async fn user_command(config: Option<PathBuf>, command: UserCommand) -> Result<()> {
    let context = CliContext::open(config).await?;
    match command {
        UserCommand::Create {
            email,
            password,
            first_name,
            last_name,
        } => {
            let auth = AuthService::new(context.platform.clone());
            let mut input = CreateUserInput::new(email);
            input.first_name = first_name;
            input.last_name = last_name;
            let user = auth.register(input, &password).await?;
            println!("created user {} ({})", user.id, user.email);
        }
        UserCommand::Grant {
            email,
            store_id,
            roles,
        } => {
            let user_id = context.user_id_by_email(&email).await?;
            let store_id = StoreId::from_string(store_id);
            let role_refs: Vec<&str> = roles.iter().map(String::as_str).collect();
            let membership = context
                .platform
                .upsert_membership(&user_id, &store_id, &role_refs)
                .await?;
            println!(
                "granted user {} roles {:?} in store {}",
                user_id, membership.roles, store_id
            );
        }
    }
    Ok(())
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
    println!("configuration ok");
    Ok(())
}
