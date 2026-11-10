use serde_json::{Value, json};
use server::deluge::*;
use shared::api::download::{Torrent, torrent::State};
use wiremock::{
    Mock,
    MockBuilder,
    MockServer,
    ResponseTemplate,
    matchers::{any, body_partial_json, header, method, path},
};

const MAGNET: &str = "magnet:?xt=urn:btih:abc";
const TORRENTS: &str = include_str!("../testdata/deluge_torrents_status.json");

fn by_name<'a>(torrents: &'a [Torrent], name: &str) -> &'a Torrent {
    torrents.iter().find(|t| t.name == name).unwrap()
}

fn rpc(result: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({"id": 1, "result": result, "error": null}))
}

fn rpc_error(code: i64, message: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "id": 1,
        "result": null,
        "error": {"code": code, "message": message},
    }))
}

/// Matches a JSON-RPC call to `rpc_method`.
fn call(rpc_method: &str) -> MockBuilder {
    Mock::given(method("POST"))
        .and(path("/json"))
        .and(body_partial_json(json!({"method": rpc_method})))
}

fn client_for(server: &MockServer) -> Client {
    let _ = rustls::crypto::ring::default_provider().install_default();
    Client::new(&server.uri(), "pw".into()).unwrap()
}

/// Mounts a `web.connected` that reports a logged-in session and a
/// connected daemon.
async fn connected(server: &MockServer) {
    call("web.connected").respond_with(rpc(json!(true))).mount(server).await;
}

#[tokio::test]
async fn add_sends_magnets_to_add_torrent_magnet() {
    let server = MockServer::start().await;
    connected(&server).await;
    call("core.add_torrent_magnet")
        .and(body_partial_json(json!({"params": [MAGNET, {}]})))
        .respond_with(rpc(json!("abc")))
        .expect(1)
        .mount(&server)
        .await;

    client_for(&server).add(MAGNET).await.unwrap();
}

#[tokio::test]
async fn add_sends_links_to_add_torrent_url() {
    let server = MockServer::start().await;
    connected(&server).await;
    call("core.add_torrent_url")
        .and(body_partial_json(json!({"params": ["http://jackett/dl/a", {}]})))
        .respond_with(rpc(json!("abc")))
        .expect(1)
        .mount(&server)
        .await;

    client_for(&server).add("http://jackett/dl/a").await.unwrap();
}

#[tokio::test]
async fn add_logs_in_once_and_reuses_the_session() {
    let server = MockServer::start().await;
    // With the session cookie, Deluge reports a connected daemon...
    call("web.connected")
        .and(header("cookie", "_session_id=abc"))
        .respond_with(rpc(json!(true)))
        .with_priority(1)
        .mount(&server)
        .await;
    // ...and without it, the request is rejected.
    call("web.connected")
        .respond_with(rpc_error(1, "Not authenticated"))
        .mount(&server)
        .await;
    call("auth.login")
        .and(body_partial_json(json!({"params": ["pw"]})))
        .respond_with(rpc(json!(true)).insert_header("set-cookie", "_session_id=abc; Path=/"))
        .expect(1)
        .mount(&server)
        .await;
    call("core.add_torrent_magnet")
        .and(header("cookie", "_session_id=abc"))
        .respond_with(rpc(json!("abc")))
        .expect(2)
        .mount(&server)
        .await;

    let client = client_for(&server);
    client.add(MAGNET).await.unwrap();
    client.add(MAGNET).await.unwrap();
}

#[tokio::test]
async fn add_with_wrong_password_returns_auth() {
    let server = MockServer::start().await;
    call("web.connected")
        .respond_with(rpc_error(1, "Not authenticated"))
        .mount(&server)
        .await;
    call("auth.login").respond_with(rpc(json!(false))).mount(&server).await;

    let err = client_for(&server).add(MAGNET).await.unwrap_err();
    assert!(matches!(err, Error::Auth), "{err:?}");
}

#[tokio::test]
async fn add_returns_auth_when_the_session_does_not_stick() {
    let server = MockServer::start().await;
    call("web.connected")
        .respond_with(rpc_error(1, "Not authenticated"))
        .expect(2)
        .mount(&server)
        .await;
    // Login "succeeds" but sets no cookie, so the retry is still rejected.
    call("auth.login")
        .respond_with(rpc(json!(true)))
        .expect(1)
        .mount(&server)
        .await;

    let err = client_for(&server).add(MAGNET).await.unwrap_err();
    assert!(matches!(err, Error::Auth), "{err:?}");
}

#[tokio::test]
async fn add_connects_to_the_first_host_when_disconnected() {
    let server = MockServer::start().await;
    call("web.connected")
        .respond_with(rpc(json!(false)))
        .mount(&server)
        .await;
    call("web.get_hosts")
        .respond_with(rpc(json!([
            ["host1", "127.0.0.1", 58846, "localclient"],
            ["host2", "10.0.0.5", 58846, "other"],
        ])))
        .mount(&server)
        .await;
    call("web.connect")
        .and(body_partial_json(json!({"params": ["host1"]})))
        .respond_with(rpc(json!([])))
        .expect(1)
        .mount(&server)
        .await;
    call("core.add_torrent_magnet")
        .respond_with(rpc(json!("abc")))
        .expect(1)
        .mount(&server)
        .await;

    client_for(&server).add(MAGNET).await.unwrap();
}

#[tokio::test]
async fn add_without_hosts_returns_no_hosts() {
    let server = MockServer::start().await;
    call("web.connected")
        .respond_with(rpc(json!(false)))
        .mount(&server)
        .await;
    call("web.get_hosts").respond_with(rpc(json!([]))).mount(&server).await;

    let err = client_for(&server).add(MAGNET).await.unwrap_err();
    assert!(matches!(err, Error::NoHosts), "{err:?}");
}

#[tokio::test]
async fn add_passes_deluge_errors_through() {
    let server = MockServer::start().await;
    connected(&server).await;
    call("core.add_torrent_magnet")
        .respond_with(rpc_error(2, "Torrent already in session (abc)."))
        .mount(&server)
        .await;

    let err = client_for(&server).add(MAGNET).await.unwrap_err();
    assert!(err.to_string().contains("Torrent already in session"), "{err}");
}

#[tokio::test]
async fn add_maps_server_errors_to_status() {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(502))
        .mount(&server)
        .await;

    let err = client_for(&server).add(MAGNET).await.unwrap_err();
    assert!(matches!(err, Error::Status(s) if s == 502), "{err:?}");
}

#[tokio::test]
async fn add_rejects_responses_that_are_not_json() {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_string("<html><body>Login</body></html>"))
        .mount(&server)
        .await;

    let err = client_for(&server).add(MAGNET).await.unwrap_err();
    assert!(matches!(err, Error::Parse(_)), "{err:?}");
}

#[tokio::test]
async fn add_rejects_unsupported_uris_without_calling_deluge() {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(rpc(json!(true)))
        .expect(0)
        .mount(&server)
        .await;

    let err = client_for(&server).add("ftp://host/a.torrent").await.unwrap_err();
    assert!(matches!(err, Error::Uri), "{err:?}");
}

#[tokio::test]
async fn add_reports_unreachable_deluge_without_leaking_the_password() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    // Nothing listens here.
    let client = Client::new("http://127.0.0.1:1", "secret".into()).unwrap();

    let err = client.add(MAGNET).await.unwrap_err();
    assert!(matches!(err, Error::Http(_)), "{err:?}");
    assert!(!err.to_string().contains("secret"), "{err}");
}

#[tokio::test]
async fn list_maps_fields() {
    let server = MockServer::start().await;
    connected(&server).await;
    call("core.get_torrents_status")
        .and(body_partial_json(json!({"params": [{}, [
            "name",
            "state",
            "progress",
            "total_size",
            "download_payload_rate",
            "upload_payload_rate",
            "eta",
        ]]})))
        .respond_with(ResponseTemplate::new(200).set_body_string(TORRENTS))
        .expect(1)
        .mount(&server)
        .await;

    let torrents = client_for(&server).list().await.unwrap();
    assert_eq!(torrents.len(), 6);

    let mut want = Torrent {
        id: "aaaa000000000000000000000000000000000002".into(),
        name: "ubuntu-24.04.iso".into(),
        progress: 62.5,
        size_bytes: 6_114_656_256,
        download_rate: 4_404_019,
        upload_rate: 122_880,
        eta_seconds: 180,
        ..Default::default()
    };
    want.set_state(State::Downloading);
    assert_eq!(by_name(&torrents, "ubuntu-24.04.iso"), &want);

    // A negative ETA means unknown.
    assert_eq!(by_name(&torrents, "alpine.iso").eta_seconds, 0);
    assert_eq!(by_name(&torrents, "mint.iso").state(), State::Unspecified);
}

#[tokio::test]
async fn list_sorts_in_progress_torrents_first() {
    let server = MockServer::start().await;
    connected(&server).await;
    call("core.get_torrents_status")
        .respond_with(ResponseTemplate::new(200).set_body_string(TORRENTS))
        .mount(&server)
        .await;

    let torrents = client_for(&server).list().await.unwrap();
    let names: Vec<&str> = torrents.iter().map(|t| t.name.as_str()).collect();

    assert_eq!(
        names,
        [
            "alpine.iso",       // Downloading, 10%
            "ubuntu-24.04.iso", // Downloading, 62.5%
            "debian-12.iso",    // Queued
            "mint.iso",         // Moving (Unspecified)
            "arch.iso",         // Paused
            "fedora-41.iso",    // Seeding
        ]
    );
}

#[tokio::test]
async fn list_maps_deluge_states() {
    let cases = [
        ("Downloading", State::Downloading),
        ("Queued", State::Queued),
        ("Checking", State::Checking),
        ("Error", State::Error),
        ("Paused", State::Paused),
        ("Seeding", State::Seeding),
        ("Moving", State::Unspecified),
        ("Allocating", State::Unspecified),
        ("downloading", State::Unspecified),
    ];

    // One torrent per state, named after the state it's in.
    let statuses: serde_json::Map<String, Value> = cases
        .iter()
        .enumerate()
        .map(|(i, (state, _))| {
            let status = json!({
                "name": state,
                "state": state,
                "progress": 0.0,
                "total_size": 0,
                "download_payload_rate": 0,
                "upload_payload_rate": 0,
                "eta": 0,
            });
            (format!("{i:040}"), status)
        })
        .collect();

    let server = MockServer::start().await;
    connected(&server).await;
    call("core.get_torrents_status")
        .respond_with(rpc(Value::Object(statuses)))
        .mount(&server)
        .await;

    let torrents = client_for(&server).list().await.unwrap();
    for (state, want) in cases {
        assert_eq!(by_name(&torrents, state).state(), want, "{state}");
    }
}

#[tokio::test]
async fn list_logs_in_when_the_session_is_missing() {
    let server = MockServer::start().await;
    call("web.connected")
        .and(header("cookie", "_session_id=abc"))
        .respond_with(rpc(json!(true)))
        .with_priority(1)
        .mount(&server)
        .await;
    call("web.connected")
        .respond_with(rpc_error(1, "Not authenticated"))
        .mount(&server)
        .await;
    call("auth.login")
        .respond_with(rpc(json!(true)).insert_header("set-cookie", "_session_id=abc; Path=/"))
        .expect(1)
        .mount(&server)
        .await;
    call("core.get_torrents_status")
        .and(header("cookie", "_session_id=abc"))
        .respond_with(rpc(json!({})))
        .expect(1)
        .mount(&server)
        .await;

    assert_eq!(client_for(&server).list().await.unwrap(), vec![]);
}

#[test]
fn parse_returns_the_result() {
    assert!(parse::<bool>(r#"{"id": 1, "result": true, "error": null}"#).unwrap());
}

#[test]
fn parse_returns_rpc_errors() {
    let err = parse::<Value>(r#"{"id": 1, "result": null, "error": {"message": "Not authenticated", "code": 1}}"#)
        .unwrap_err();

    assert!(
        matches!(&err, Error::Rpc { code: 1, message } if message == "Not authenticated"),
        "{err:?}"
    );
}

#[test]
fn parse_rejects_non_json() {
    let err = parse::<Value>("<html><body>Login</body></html>").unwrap_err();
    assert!(matches!(err, Error::Parse(_)), "{err:?}");
}

#[test]
fn parse_rejects_unexpected_result_types() {
    let err = parse::<bool>(r#"{"id": 1, "result": "yes", "error": null}"#).unwrap_err();
    assert!(matches!(err, Error::Parse(_)), "{err:?}");
}

#[test]
fn add_method_picks_the_method_by_scheme() {
    assert_eq!(add_method("magnet:?xt=urn:btih:abc"), Some("core.add_torrent_magnet"));
    assert_eq!(add_method("MAGNET:?xt=urn:btih:abc"), Some("core.add_torrent_magnet"));
    assert_eq!(add_method("http://jackett/dl/a"), Some("core.add_torrent_url"));
    assert_eq!(add_method("https://jackett/dl/a"), Some("core.add_torrent_url"));
    assert_eq!(add_method("ftp://host/a.torrent"), None);
    assert_eq!(add_method("file:///etc/passwd"), None);
    assert_eq!(add_method("not a uri"), None);
    assert_eq!(add_method(""), None);
}
