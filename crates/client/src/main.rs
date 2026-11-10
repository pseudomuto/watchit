use std::{
    fs,
    io::{self, IsTerminal},
    path::{Path, PathBuf},
};

use clap::{Parser, Subcommand};
use client::{api::Client, tui};
use color_eyre::{Result, eyre::bail};
use shared::certs::{self, Bundle};

#[derive(Parser)]
struct Cli {
    /// DNS name or IP address of the server. Must match a SAN on its
    /// certificate.
    #[arg(long, default_value = "localhost")]
    host: String,

    /// Port the server listens on.
    #[arg(long, default_value_t = 50051)]
    port: u16,

    /// Directory containing ca.pem, client.pem and client.key. The `certs`
    /// command reads the CA from and writes certificates to it.
    #[arg(long, default_value = "certs", global = true)]
    certs_dir: PathBuf,

    /// Optional initial search, run as soon as the TUI opens.
    term: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Generate the CA and certificates used for mTLS between server and
    /// client.
    #[command(subcommand)]
    Certs(CertsCommand),
}

#[derive(Subcommand)]
enum CertsCommand {
    /// Generate a new CA (ca.pem, ca.key).
    Ca,
    /// Issue a server certificate signed by the CA (server.pem, server.key).
    Server {
        /// DNS name or IP address clients use to reach the server. Repeatable.
        #[arg(long = "san", required = true)]
        sans: Vec<String>,
    },
    /// Issue a client certificate signed by the CA (client.pem, client.key).
    Client {
        /// Common name identifying this client.
        #[arg(long, default_value = "client")]
        name: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();

    if let Some(Command::Certs(command)) = cli.command {
        return generate_certs(&cli.certs_dir, command);
    }

    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("the client needs a terminal: stdin and stdout must both be a TTY");
    }

    let client = Client::connect(&cli.host, cli.port, &cli.certs_dir).await?;
    ratatui::run(|terminal| tui::run(terminal, client, cli.term))
}

fn generate_certs(dir: &Path, command: CertsCommand) -> Result<()> {
    fs::create_dir_all(dir)?;

    let (stem, bundle) = match command {
        CertsCommand::Ca => ("ca", certs::generate_ca()?),
        CertsCommand::Server { sans } => ("server", certs::issue_server(&Bundle::load(dir, "ca")?, &sans)?),
        CertsCommand::Client { name } => ("client", certs::issue_client(&Bundle::load(dir, "ca")?, &name)?),
    };

    bundle.save(dir, stem)?;
    println!("wrote {0}/{stem}.pem and {0}/{stem}.key", dir.display());

    Ok(())
}
