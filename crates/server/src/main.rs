use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
};

use clap::Parser;
use color_eyre::{Result, eyre::eyre};

#[derive(Parser)]
#[command(name = "watchit-server")]
struct Cli {
    /// IP address to bind to.
    #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::UNSPECIFIED))]
    host: IpAddr,

    /// Port to listen on.
    #[arg(long, default_value_t = 50051)]
    port: u16,

    /// Directory containing ca.pem, server.pem and server.key.
    #[arg(long, default_value = "certs")]
    certs_dir: PathBuf,

    /// Base URL of the Jackett instance, e.g. http://localhost:9117.
    #[arg(long)]
    jackett_url: String,

    /// Jackett API key, shown on the Jackett dashboard.
    #[arg(long, env = "WATCHIT_JACKETT_API_KEY", hide_env_values = true)]
    jackett_api_key: String,

    /// Jackett indexer to search, or "all" for every configured indexer.
    #[arg(long, default_value = "all")]
    jackett_indexer: String,

    /// Base URL of the Deluge Web UI.
    #[arg(long, default_value = "http://localhost:8112")]
    deluge_url: String,

    /// Deluge Web UI password.
    #[arg(long, env = "WATCHIT_DELUGE_PASSWORD", hide_env_values = true)]
    deluge_password: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();

    // reqwest uses the process-wide rustls provider. Match tonic's ring.
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| eyre!("failed to install the rustls crypto provider"))?;

    let addr = SocketAddr::new(cli.host, cli.port);
    let jackett = server::jackett::Client::new(&cli.jackett_url, cli.jackett_api_key, cli.jackett_indexer)?;
    let deluge = server::deluge::Client::new(&cli.deluge_url, cli.deluge_password)?;
    let router = server::router(server::tls_config(&cli.certs_dir)?, jackett, deluge)?;

    println!("listening on {addr}");
    router.serve(addr).await?;

    Ok(())
}
