use server::{deluge, services::DownloadServer as Server};
use shared::api::download::{AddRequest, DownloadService, ListRequest};
use tonic::{Request, Status};

fn server() -> Server {
    let _ = rustls::crypto::ring::default_provider().install_default();
    // Nothing listens here, so every request to Deluge fails.
    Server::new(deluge::Client::new("http://127.0.0.1:1", "secret".into()).unwrap())
}

async fn add(uri: &str) -> Status {
    server()
        .add(Request::new(AddRequest { uri: uri.into() }))
        .await
        .unwrap_err()
}

#[tokio::test]
async fn add_rejects_blank_uris() {
    let err = add("  ").await;
    assert_eq!(err.code(), tonic::Code::InvalidArgument, "{err:?}");
}

#[tokio::test]
async fn add_rejects_unsupported_schemes() {
    let err = add("file:///etc/passwd").await;
    assert_eq!(err.code(), tonic::Code::InvalidArgument, "{err:?}");
}

#[tokio::test]
async fn add_reports_unreachable_deluge_without_leaking_the_password() {
    let err = add("magnet:?xt=urn:btih:abc").await;

    assert_eq!(err.code(), tonic::Code::Unavailable, "{err:?}");
    assert!(!err.message().contains("secret"), "{err:?}");
}

#[tokio::test]
async fn list_reports_unreachable_deluge_without_leaking_the_password() {
    let err = server().list(Request::new(ListRequest {})).await.unwrap_err();

    assert_eq!(err.code(), tonic::Code::Unavailable, "{err:?}");
    assert!(!err.message().contains("secret"), "{err:?}");
}
