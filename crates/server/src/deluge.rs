use std::{collections::HashMap, fmt, time::Duration};

use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use shared::api::download::{Torrent, torrent::State};

/// JSON-RPC error code Deluge returns when the session is missing or expired.
const NOT_AUTHENTICATED: i64 = 1;

/// Fields requested from `core.get_torrents_status`.
const TORRENT_FIELDS: [&str; 7] = [
    "name",
    "state",
    "progress",
    "total_size",
    "download_payload_rate",
    "upload_payload_rate",
    "eta",
];

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

/// One torrent in a `core.get_torrents_status` response. Rates and ETA are
/// read as floats so either integers or floats parse.
#[derive(Deserialize)]
struct TorrentStatus {
    name: String,
    state: String,
    progress: f64,
    total_size: i64,
    download_payload_rate: f64,
    upload_payload_rate: f64,
    eta: f64,
}

impl TorrentStatus {
    fn into_torrent(self, id: String) -> Torrent {
        let mut torrent = Torrent {
            id,
            name: self.name,
            progress: self.progress,
            size_bytes: self.total_size,
            download_rate: self.download_payload_rate as i64,
            upload_rate: self.upload_payload_rate as i64,
            eta_seconds: (self.eta as i64).max(0),
            ..Default::default()
        };
        torrent.set_state(state(&self.state));
        torrent
    }
}

/// Maps a Deluge state string to `State`. Anything else, including `Moving`
/// and `Allocating`, is `Unspecified`.
fn state(s: &str) -> State {
    match s {
        "Downloading" => State::Downloading,
        "Queued" => State::Queued,
        "Checking" => State::Checking,
        "Error" => State::Error,
        "Paused" => State::Paused,
        "Seeding" => State::Seeding,
        _ => State::Unspecified,
    }
}

/// Sorts in-progress torrents first: by state group, then progress
/// ascending, then name.
fn sort_torrents(torrents: &mut [Torrent]) {
    torrents.sort_by(|a, b| {
        rank(a.state())
            .cmp(&rank(b.state()))
            .then(a.progress.total_cmp(&b.progress))
            .then_with(|| a.name.cmp(&b.name))
    });
}

fn rank(state: State) -> u8 {
    match state {
        State::Downloading => 0,
        State::Queued => 1,
        State::Checking => 2,
        State::Error => 3,
        State::Unspecified => 4,
        State::Paused => 5,
        State::Seeding => 6,
    }
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

/// Adds and lists torrents through the Deluge Web UI's JSON-RPC API. The
/// session cookie is kept for the life of the client, and the client logs in
/// again when it expires.
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

    /// Lists every torrent in Deluge, in-progress ones first.
    pub async fn list(&self) -> Result<Vec<Torrent>, Error> {
        if !self.connected().await? {
            self.connect().await?;
        }

        let statuses: HashMap<String, TorrentStatus> = self
            .call("core.get_torrents_status", json!([{}, TORRENT_FIELDS]))
            .await?;

        let mut torrents: Vec<Torrent> = statuses
            .into_iter()
            .map(|(id, status)| status.into_torrent(id))
            .collect();
        sort_torrents(&mut torrents);
        Ok(torrents)
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
