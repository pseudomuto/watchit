use std::path::PathBuf;

use clap::Parser;
use client::api::Client;
use color_eyre::Result;
use shared::api::search::SearchRequest;

#[derive(Parser)]
struct Cli {
    /// DNS name or IP address of the server. Must match a SAN on its
    /// certificate.
    #[arg(long, default_value = "localhost")]
    host: String,

    /// Port the server listens on.
    #[arg(long, default_value_t = 50051)]
    port: u16,

    /// Directory containing ca.pem, client.pem and client.key.
    #[arg(long, default_value = "certs")]
    certs_dir: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();

    let mut client = Client::connect(&cli.host, cli.port, &cli.certs_dir).await?;
    let resp = client.search.search(SearchRequest { term: "Hello!".into() }).await?;

    println!("Items for term Hello!: {:?}", resp.into_inner().results.len());

    Ok(())
}
