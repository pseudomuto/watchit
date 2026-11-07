pub mod certs;
pub mod jackett;
pub mod services;

use std::{io, path::Path};

use shared::api::search::SearchServiceServer;
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig, server::Router};

use crate::{certs::Bundle, services::SearchServer};

/// Loads `ca.pem`, `server.pem` and `server.key` from `dir`. Clients must
/// present a certificate signed by the CA.
pub fn tls_config(dir: &Path) -> io::Result<ServerTlsConfig> {
    let server = Bundle::load(dir, "server")?;
    let ca = certs::read(&dir.join("ca.pem"))?;

    Ok(ServerTlsConfig::new()
        .identity(Identity::from_pem(server.cert_pem, server.key_pem))
        .client_ca_root(Certificate::from_pem(ca)))
}

/// Builds the gRPC router with TLS enabled and all services registered.
pub fn router(tls: ServerTlsConfig, jackett: jackett::Client) -> Result<Router, tonic::transport::Error> {
    Ok(Server::builder()
        .tls_config(tls)?
        .add_service(SearchServiceServer::new(SearchServer::new(jackett))))
}
