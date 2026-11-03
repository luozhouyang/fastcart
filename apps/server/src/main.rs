//! Fastcart HTTP server binary.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    fastcart_server::run_from_env().await
}
