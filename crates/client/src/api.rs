use std::{fs, net::Ipv6Addr, path::Path};

use color_eyre::eyre::WrapErr;
use shared::api::search::SearchServiceClient;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity};

#[derive(Debug)]
pub struct Client {
    pub search: SearchServiceClient<Channel>,
}

impl Client {
    /// Connects to the server at `host:port` over mTLS using `ca.pem`,
    /// `client.pem` and `client.key` from `certs_dir`. The server's certificate
    /// must be valid for `host`.
    pub async fn connect(host: &str, port: u16, certs_dir: &Path) -> color_eyre::Result<Self> {
        let read = |name: &str| {
            let path = certs_dir.join(name);
            fs::read_to_string(&path).wrap_err_with(|| format!("failed to read {}", path.display()))
        };

        let tls = ClientTlsConfig::new()
            .ca_certificate(Certificate::from_pem(read("ca.pem")?))
            .identity(Identity::from_pem(read("client.pem")?, read("client.key")?))
            .domain_name(host);

        let authority = match host.parse::<Ipv6Addr>() {
            Ok(_) => format!("[{host}]:{port}"),
            Err(_) => format!("{host}:{port}"),
        };

        let channel = Endpoint::from_shared(format!("https://{authority}"))?
            .tls_config(tls)?
            .connect()
            .await?;

        Ok(Self {
            search: SearchServiceClient::new(channel),
        })
    }
}
