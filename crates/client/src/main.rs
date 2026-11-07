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

    /// What to search for.
    term: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();

    let mut client = Client::connect(&cli.host, cli.port, &cli.certs_dir).await?;
    let resp = client.search.search(SearchRequest { term: cli.term }).await?;

    for r in resp.into_inner().results {
        println!("{}", r.name);
        println!(
            "  seeders: {}  leechers: {}  size: {}",
            r.seeders,
            r.leechers,
            format_size(r.size_bytes)
        );
        println!("  {}", r.uri);
    }

    Ok(())
}

fn format_size(bytes: i64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];

    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }

    match unit {
        0 => format!("{bytes} B"),
        _ => format!("{size:.1} {}", UNITS[unit]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_size_uses_binary_units() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1536), "1.5 KiB");
        assert_eq!(format_size(6_114_656_256), "5.7 GiB");
    }
}
