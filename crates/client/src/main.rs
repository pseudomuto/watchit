use std::{
    io::{self, IsTerminal},
    path::PathBuf,
};

use clap::Parser;
use client::{api::Client, tui};
use color_eyre::{Result, eyre::bail};

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

    /// Optional initial search, run as soon as the TUI opens.
    term: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();

    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("the client needs a terminal: stdin and stdout must both be a TTY");
    }

    let client = Client::connect(&cli.host, cli.port, &cli.certs_dir).await?;
    ratatui::run(|terminal| tui::run(terminal, client.search, cli.term))
}
