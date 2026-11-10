use prost_types::Timestamp;
use server::jackett::*;
use shared::api::search::Result as SearchResult;
use wiremock::{
    Mock,
    MockServer,
    ResponseTemplate,
    matchers::{method, path, query_param},
};

const SEARCH: &str = include_str!("../testdata/torznab_search.xml");

#[test]
fn parse_maps_item_fields() {
    let results = parse(SEARCH).unwrap();

    assert_eq!(
        results,
        vec![SearchResult {
            name: "Ubuntu 24.04 Desktop amd64".into(),
            uri: "magnet:?xt=urn:btih:abc&dn=Ubuntu".into(),
            seeders: 120,
            leechers: 15,
            size_bytes: 6114656256,
            created_at: Some(Timestamp {
                seconds: 1713175200,
                nanos: 0
            }),
        }]
    );
}

fn feed(items: &str) -> String {
    format!(r#"<rss xmlns:torznab="http://torznab.com/schemas/2015/feed"><channel>{items}</channel></rss>"#)
}

#[test]
fn parse_falls_back_to_link_without_magnet() {
    let xml = feed("<item><title>a</title><link>http://jackett/dl/a</link></item>");

    assert_eq!(parse(&xml).unwrap()[0].uri, "http://jackett/dl/a");
}

#[test]
fn parse_skips_items_without_title_or_uri() {
    let xml = feed(
        "<item><link>http://jackett/dl/no-title</link></item>\
         <item><title>no uri</title></item>\
         <item><title>ok</title><link>http://jackett/dl/ok</link></item>",
    );

    let names: Vec<_> = parse(&xml).unwrap().into_iter().map(|r| r.name).collect();
    assert_eq!(names, vec!["ok"]);
}

#[test]
fn parse_returns_torznab_errors() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?><error code="100" description="Invalid API Key" />"#;

    let err = parse(xml).unwrap_err();
    assert!(
        matches!(&err, Error::Torznab { code, description } if code == "100" && description == "Invalid API Key"),
        "{err:?}"
    );
}

async fn client_for(server: &MockServer) -> Client {
    let _ = rustls::crypto::ring::default_provider().install_default();
    Client::new(&server.uri(), "key".into(), "all".into()).unwrap()
}

#[tokio::test]
async fn search_queries_torznab_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2.0/indexers/all/results/torznab/api"))
        .and(query_param("apikey", "key"))
        .and(query_param("t", "search"))
        .and(query_param("q", "ubuntu 24.04"))
        .respond_with(ResponseTemplate::new(200).set_body_string(SEARCH))
        .expect(1)
        .mount(&server)
        .await;

    let results = client_for(&server).await.search("ubuntu 24.04").await.unwrap();
    assert_eq!(results.len(), 1);
}

#[tokio::test]
async fn search_maps_server_errors_to_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(502))
        .mount(&server)
        .await;

    let err = client_for(&server).await.search("x").await.unwrap_err();
    assert!(matches!(err, Error::Status(s) if s == 502), "{err:?}");
}

#[tokio::test]
async fn search_parses_torznab_errors_on_client_error_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(400).set_body_string(r#"<error code="100" description="Invalid API Key" />"#),
        )
        .mount(&server)
        .await;

    let err = client_for(&server).await.search("x").await.unwrap_err();
    assert!(matches!(err, Error::Torznab { .. }), "{err:?}");
}

#[test]
fn parse_rejects_non_torznab_documents() {
    let err = parse("<html><body>Login</body></html>").unwrap_err();
    assert!(matches!(err, Error::Parse(_)), "{err:?}");
}
