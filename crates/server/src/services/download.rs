use shared::api::download::{AddRequest, AddResponse, DownloadService};
use tonic::{Request, Response, Status};

use crate::deluge;

pub struct Server {
    deluge: deluge::Client,
}

impl Server {
    pub fn new(deluge: deluge::Client) -> Self {
        Self { deluge }
    }
}

#[tonic::async_trait]
impl DownloadService for Server {
    async fn add(&self, request: Request<AddRequest>) -> Result<Response<AddResponse>, Status> {
        let req = request.into_inner();
        let uri = req.uri.trim();
        if deluge::add_method(uri).is_none() {
            return Err(Status::invalid_argument("uri must be a magnet link or an http(s) URL"));
        }

        self.deluge.add(uri).await.map_err(to_status)?;

        Ok(Response::new(AddResponse {}))
    }
}

fn to_status(err: deluge::Error) -> Status {
    eprintln!("add failed: {err}");

    match err {
        deluge::Error::Http(_) | deluge::Error::Status(_) => Status::unavailable(err.to_string()),
        deluge::Error::Auth | deluge::Error::NoHosts | deluge::Error::Rpc { .. } => {
            Status::failed_precondition(err.to_string())
        }
        deluge::Error::Parse(_) => Status::internal(err.to_string()),
        deluge::Error::Uri => Status::invalid_argument(err.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn to_status_maps_deluge_errors() {
        use tonic::Code;

        let cases = [
            (
                deluge::Error::Status(reqwest::StatusCode::BAD_GATEWAY),
                Code::Unavailable,
            ),
            (deluge::Error::Auth, Code::FailedPrecondition),
            (deluge::Error::NoHosts, Code::FailedPrecondition),
            (
                deluge::Error::Rpc {
                    code: 2,
                    message: "Torrent already in session".into(),
                },
                Code::FailedPrecondition,
            ),
            (deluge::Error::Parse("bad".into()), Code::Internal),
            (deluge::Error::Uri, Code::InvalidArgument),
        ];

        for (err, code) in cases {
            assert_eq!(to_status(err).code(), code);
        }
    }
}
