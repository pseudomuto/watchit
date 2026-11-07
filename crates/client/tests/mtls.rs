use std::{fs, path::Path};

use client::api::Client;
use server::certs;
use shared::api::search::{SearchRequest, SearchServiceClient};
use tokio::net::TcpListener;
use tonic::transport::{Certificate, ClientTlsConfig, Endpoint, server::TcpIncoming};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::any};

const HOST: &str = "127.0.0.1";

/// Writes a CA, a server cert for `sans` and a client cert into `dir`.
fn write_certs(dir: &Path, sans: &[&str]) {
    let ca = certs::generate_ca().unwrap();
    let sans: Vec<String> = sans.iter().map(|s| s.to_string()).collect();

    ca.save(dir, "ca").unwrap();
    certs::issue_server(&ca, &sans).unwrap().save(dir, "server").unwrap();
    certs::issue_client(&ca, "test").unwrap().save(dir, "client").unwrap();
}

/// Starts a server on an ephemeral port, backed by a fake Jackett that
/// returns no results. Returns the port and the fake Jackett, which must be
/// kept alive for the duration of the test.
async fn start_server(dir: &Path) -> (u16, MockServer) {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let jackett = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_string("<rss><channel></channel></rss>"))
        .mount(&jackett)
        .await;
    let client = server::jackett::Client::new(&jackett.uri(), "key".into(), "all".into()).unwrap();

    let listener = TcpListener::bind((HOST, 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let router = server::router(server::tls_config(dir).unwrap(), client).unwrap();

    tokio::spawn(router.serve_with_incoming(TcpIncoming::from(listener)));
    (port, jackett)
}

async fn search(host: &str, port: u16, certs_dir: &Path) -> color_eyre::Result<()> {
    let mut client = Client::connect(host, port, certs_dir).await?;
    client.search.search(SearchRequest { term: "test".into() }).await?;
    Ok(())
}

#[tokio::test]
async fn client_with_valid_cert_can_search() {
    let dir = tempfile::tempdir().unwrap();
    write_certs(dir.path(), &[HOST]);
    let (port, _jackett) = start_server(dir.path()).await;

    search(HOST, port, dir.path()).await.unwrap();
}

#[tokio::test]
async fn client_without_cert_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    write_certs(dir.path(), &[HOST]);
    let (port, _jackett) = start_server(dir.path()).await;
    search(HOST, port, dir.path())
        .await
        .expect("valid client should succeed");

    let tls = ClientTlsConfig::new()
        .ca_certificate(Certificate::from_pem(fs::read(dir.path().join("ca.pem")).unwrap()))
        .domain_name(HOST);

    let result = async {
        let channel = Endpoint::from_shared(format!("https://{HOST}:{port}"))?
            .tls_config(tls)?
            .connect()
            .await?;
        SearchServiceClient::new(channel)
            .search(SearchRequest { term: "test".into() })
            .await?;
        Ok::<_, color_eyre::Report>(())
    }
    .await;

    let err = result.expect_err("client without a cert should be rejected");
    eprintln!("rejected: {err:?}");
}

#[tokio::test]
async fn client_cert_from_another_ca_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    write_certs(dir.path(), &[HOST]);
    let (port, _jackett) = start_server(dir.path()).await;
    search(HOST, port, dir.path())
        .await
        .expect("valid client should succeed");

    // Trusts the real CA, but presents a cert signed by a different one.
    let rogue = tempfile::tempdir().unwrap();
    fs::copy(dir.path().join("ca.pem"), rogue.path().join("ca.pem")).unwrap();
    let rogue_ca = certs::generate_ca().unwrap();
    certs::issue_client(&rogue_ca, "test")
        .unwrap()
        .save(rogue.path(), "client")
        .unwrap();

    let err = search(HOST, port, rogue.path())
        .await
        .expect_err("client cert from another CA should be rejected");
    eprintln!("rejected: {err:?}");
}

#[tokio::test]
async fn server_cert_for_another_host_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    write_certs(dir.path(), &["other.example"]);
    let (port, _jackett) = start_server(dir.path()).await;

    let err = search(HOST, port, dir.path())
        .await
        .expect_err("server cert without a matching SAN should be rejected");
    assert!(format!("{err:?}").contains("not valid for name"), "{err:?}");
}
