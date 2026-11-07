use std::{fmt, time::Duration};

use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};

/// JSON-RPC error code Deluge returns when the session is missing or expired.
const NOT_AUTHENTICATED: i64 = 1;

#[derive(Debug)]
pub enum Error {
    /// Deluge could not be reached, or the request timed out.
    Http(reqwest::Error),
    /// Deluge responded with a non-success HTTP status.
    Status(reqwest::StatusCode),
    /// Deluge rejected the password, or the session didn't stick after logging
    /// in.
    Auth,
    /// The Web UI isn't connected to a daemon and has none configured.
    NoHosts,
    /// Deluge responded with a JSON-RPC error.
    Rpc { code: i64, message: String },
    /// The response body was not a JSON-RPC response.
    Parse(String),
    /// The URI is not a magnet link or an http(s) URL.
    Uri,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(e) => write!(f, "failed to reach Deluge: {e}"),
            Self::Status(s) => write!(f, "Deluge responded with HTTP {s}"),
            Self::Auth => write!(f, "Deluge rejected the password"),
            Self::NoHosts => write!(f, "Deluge Web UI is not connected to a daemon and has none configured"),
            Self::Rpc { code, message } => write!(f, "Deluge error {code}: {message}"),
            Self::Parse(e) => write!(f, "failed to parse Deluge response: {e}"),
            Self::Uri => write!(f, "expected a magnet link or an http(s) URL"),
        }
    }
}

impl std::error::Error for Error {}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        Self::Http(err.without_url())
    }
}

#[derive(Deserialize)]
struct Response {
    #[serde(default)]
    result: Value,
    error: Option<RpcError>,
}

#[derive(Deserialize)]
struct RpcError {
    code: i64,
    message: String,
}

/// Returns the Deluge method that adds `uri`, or `None` when its scheme isn't
/// supported.
pub fn add_method(uri: &str) -> Option<&'static str> {
    let (scheme, _) = uri.split_once(':')?;

    match scheme.to_ascii_lowercase().as_str() {
        "magnet" => Some("core.add_torrent_magnet"),
        "http" | "https" => Some("core.add_torrent_url"),
        _ => None,
    }
}

/// Adds torrents through the Deluge Web UI's JSON-RPC API. The session cookie
/// is kept for the life of the client, and the client logs in again when it
/// expires.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    url: String,
    password: String,
}

impl Client {
    /// Creates a client for the Deluge Web UI at `base_url` (e.g.
    /// `http://localhost:8112`).
    ///
    /// A default rustls `CryptoProvider` must be installed first.
    pub fn new(base_url: &str, password: String) -> Result<Self, Error> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .cookie_store(true)
            .build()?;

        Ok(Self {
            http,
            url: format!("{}/json", base_url.trim_end_matches('/')),
            password,
        })
    }

    /// Adds the torrent at `uri` (a magnet link or an http(s) URL) with
    /// Deluge's default options.
    pub async fn add(&self, uri: &str) -> Result<(), Error> {
        let method = add_method(uri).ok_or(Error::Uri)?;

        if !self.connected().await? {
            self.connect().await?;
        }

        self.call::<Value>(method, json!([uri, {}])).await?;
        Ok(())
    }

    /// Reports whether the Web UI is connected to a daemon, logging in first
    /// when the session is missing or expired.
    async fn connected(&self) -> Result<bool, Error> {
        match self.call("web.connected", json!([])).await {
            Err(Error::Rpc {
                code: NOT_AUTHENTICATED,
                ..
            }) => {
                self.login().await?;
                self.call("web.connected", json!([])).await.map_err(|err| match err {
                    Error::Rpc {
                        code: NOT_AUTHENTICATED,
                        ..
                    } => Error::Auth,
                    err => err,
                })
            }
            result => result,
        }
    }

    async fn login(&self) -> Result<(), Error> {
        match self.call::<bool>("auth.login", json!([self.password])).await? {
            true => Ok(()),
            false => Err(Error::Auth),
        }
    }

    /// Connects the Web UI to the first configured daemon.
    async fn connect(&self) -> Result<(), Error> {
        // Each host is `[id, host, port, user]`.
        let hosts: Vec<Vec<Value>> = self.call("web.get_hosts", json!([])).await?;
        let id = hosts
            .first()
            .and_then(|host| host.first())
            .and_then(Value::as_str)
            .ok_or(Error::NoHosts)?;

        self.call::<Value>("web.connect", json!([id])).await?;
        Ok(())
    }

    async fn call<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<T, Error> {
        let resp = self
            .http
            .post(&self.url)
            .json(&json!({"method": method, "params": params, "id": 1}))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(Error::Status(resp.status()));
        }

        parse(&resp.text().await?)
    }
}

/// Converts a JSON-RPC response body into its result, or the error Deluge
/// returned.
pub fn parse<T: DeserializeOwned>(body: &str) -> Result<T, Error> {
    let resp: Response = serde_json::from_str(body).map_err(|e| Error::Parse(e.to_string()))?;

    if let Some(RpcError { code, message }) = resp.error {
        return Err(Error::Rpc { code, message });
    }

    serde_json::from_value(resp.result).map_err(|e| Error::Parse(e.to_string()))
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use wiremock::{
        Mock,
        MockBuilder,
        MockServer,
        ResponseTemplate,
        matchers::{any, body_partial_json, header, method, path},
    };

    use super::*;

    const MAGNET: &str = "magnet:?xt=urn:btih:abc";

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
}
