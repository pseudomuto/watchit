use std::{fmt, time::Duration};

use prost_types::Timestamp;
use serde::Deserialize;
use shared::api::search::Result as SearchResult;

#[derive(Debug)]
pub enum Error {
    /// Jackett could not be reached, or the request timed out.
    Http(reqwest::Error),
    /// Jackett responded with a non-success HTTP status.
    Status(reqwest::StatusCode),
    /// Jackett responded with a Torznab `<error>` element.
    Torznab { code: String, description: String },
    /// The response body was not a Torznab feed.
    Parse(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(e) => write!(f, "failed to reach Jackett: {e}"),
            Self::Status(s) => write!(f, "Jackett responded with HTTP {s}"),
            Self::Torznab { code, description } => write!(f, "Jackett error {code}: {description}"),
            Self::Parse(e) => write!(f, "failed to parse Jackett response: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<reqwest::Error> for Error {
    // Request URLs carry the API key, so they must not end up in messages.
    fn from(err: reqwest::Error) -> Self {
        Self::Http(err.without_url())
    }
}

/// Searches a Jackett instance through its Torznab API.
#[derive(Debug, Clone)]
pub struct Client {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    indexer: String,
}

impl Client {
    /// Creates a client for the Jackett instance at `base_url` (e.g.
    /// `http://localhost:9117`). `indexer` is a Jackett indexer ID, or `all`
    /// to search every configured indexer.
    ///
    /// A default rustls `CryptoProvider` must be installed first.
    pub fn new(base_url: &str, api_key: String, indexer: String) -> Result<Self, Error> {
        let http = reqwest::Client::builder().timeout(Duration::from_secs(30)).build()?;

        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_owned(),
            api_key,
            indexer,
        })
    }

    pub async fn search(&self, term: &str) -> Result<Vec<SearchResult>, Error> {
        let url = format!(
            "{}/api/v2.0/indexers/{}/results/torznab/api",
            self.base_url, self.indexer
        );
        let resp = self
            .http
            .get(url)
            .query(&[("apikey", self.api_key.as_str()), ("t", "search"), ("q", term)])
            .send()
            .await?;

        // Torznab errors (e.g. a bad API key) can come back with a 4xx status
        // and an `<error>` body, so leave those to `parse`.
        if resp.status().is_server_error() {
            return Err(Error::Status(resp.status()));
        }

        parse(&resp.text().await?)
    }
}

// A Torznab response is either an `<rss>` feed or an `<error>` element.
// quick-xml doesn't check the root element's name, so one struct covers both.
#[derive(Deserialize)]
struct Doc {
    #[serde(rename = "@code")]
    code: Option<String>,
    #[serde(rename = "@description")]
    description: Option<String>,
    channel: Option<Channel>,
}

#[derive(Deserialize)]
struct Channel {
    #[serde(default, rename = "item")]
    items: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    title: Option<String>,
    link: Option<String>,
    size: Option<i64>,
    #[serde(rename = "pubDate")]
    pub_date: Option<String>,
    // quick-xml matches on local names, so this is `<torznab:attr>`.
    #[serde(default, rename = "attr")]
    attrs: Vec<Attr>,
}

#[derive(Deserialize)]
struct Attr {
    #[serde(rename = "@name")]
    name: String,
    #[serde(rename = "@value")]
    value: String,
}

impl Item {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|a| a.name == name).map(|a| a.value.as_str())
    }

    fn int_attr(&self, name: &str) -> i32 {
        self.attr(name).and_then(|v| v.parse().ok()).unwrap_or_default()
    }
}

/// Converts a Torznab search response into search results. Items without a
/// title or a URI are skipped.
pub fn parse(xml: &str) -> Result<Vec<SearchResult>, Error> {
    let doc: Doc = quick_xml::de::from_str(xml).map_err(|e| Error::Parse(e.to_string()))?;

    if let Some(code) = doc.code {
        return Err(Error::Torznab {
            code,
            description: doc.description.unwrap_or_default(),
        });
    }

    let channel = doc.channel.ok_or_else(|| Error::Parse("missing <channel>".into()))?;

    Ok(channel.items.into_iter().filter_map(to_result).collect())
}

fn to_result(item: Item) -> Option<SearchResult> {
    let uri = item.attr("magneturl").map(str::to_owned).or_else(|| item.link.clone());
    let (Some(name), Some(uri)) = (item.title.clone(), uri) else {
        eprintln!("skipping Torznab item without a title or URI: {:?}", item.title);
        return None;
    };

    let seeders = item.int_attr("seeders");
    let created_at = item
        .pub_date
        .as_deref()
        .and_then(|d| jiff::fmt::rfc2822::parse(d).ok())
        .map(|z| Timestamp {
            seconds: z.timestamp().as_second(),
            nanos: 0,
        });

    Some(SearchResult {
        name,
        uri,
        seeders,
        leechers: (item.int_attr("peers") - seeders).max(0),
        size_bytes: item.size.unwrap_or_default(),
        created_at,
    })
}

#[cfg(test)]
mod tests {
    use wiremock::{
        Mock,
        MockServer,
        ResponseTemplate,
        matchers::{method, path, query_param},
    };

    use super::*;

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
}
