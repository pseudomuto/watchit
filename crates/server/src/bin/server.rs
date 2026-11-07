use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
};

use clap::Parser;
use color_eyre::Result;

#[derive(Parser)]
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
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();

    let addr = SocketAddr::new(cli.host, cli.port);
    let router = server::router(server::tls_config(&cli.certs_dir)?)?;

    println!("listening on {addr}");
    router.serve(addr).await?;

    Ok(())
}
