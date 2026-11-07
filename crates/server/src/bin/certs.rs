use std::{fs, path::PathBuf};

use clap::{Parser, Subcommand};
use color_eyre::Result;
use server::certs::{self, Bundle};

/// Generate the CA and certificates used for mTLS between server and client.
#[derive(Parser)]
struct Cli {
    /// Directory to read the CA from and write certificates to.
    #[arg(long, default_value = "certs", global = true)]
    dir: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
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

fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();
    fs::create_dir_all(&cli.dir)?;

    let (stem, bundle) = match cli.command {
        Command::Ca => ("ca", certs::generate_ca()?),
        Command::Server { sans } => ("server", certs::issue_server(&Bundle::load(&cli.dir, "ca")?, &sans)?),
        Command::Client { name } => ("client", certs::issue_client(&Bundle::load(&cli.dir, "ca")?, &name)?),
    };

    bundle.save(&cli.dir, stem)?;
    println!("wrote {0}/{stem}.pem and {0}/{stem}.key", cli.dir.display());

    Ok(())
}
