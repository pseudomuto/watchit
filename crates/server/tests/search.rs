use server::{jackett, services::SearchServer as Server};
use shared::api::search::{SearchRequest, SearchService};
use tonic::Request;

fn server() -> Server {
    let _ = rustls::crypto::ring::default_provider().install_default();
    // Nothing listens here, so every request to Jackett fails.
    Server::new(jackett::Client::new("http://127.0.0.1:1", "secret".into(), "all".into()).unwrap())
}

#[tokio::test]
async fn search_reports_unreachable_jackett_without_leaking_api_key() {
    let err = server()
        .search(Request::new(SearchRequest { term: "x".into() }))
        .await
        .unwrap_err();

    assert_eq!(err.code(), tonic::Code::Unavailable, "{err:?}");
    assert!(!err.message().contains("secret"), "{err:?}");
}

#[tokio::test]
async fn search_rejects_blank_terms() {
    let err = server()
        .search(Request::new(SearchRequest { term: "  ".into() }))
        .await
        .unwrap_err();

    assert_eq!(err.code(), tonic::Code::InvalidArgument, "{err:?}");
}
