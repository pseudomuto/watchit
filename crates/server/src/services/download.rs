#[cfg(test)]
mod tests;

use shared::api::download::{AddRequest, AddResponse, DownloadService, ListRequest, ListResponse};
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

        self.deluge.add(uri).await.map_err(|err| to_status("add", err))?;

        Ok(Response::new(AddResponse {}))
    }

    async fn list(&self, _request: Request<ListRequest>) -> Result<Response<ListResponse>, Status> {
        let torrents = self.deluge.list().await.map_err(|err| to_status("list", err))?;

        Ok(Response::new(ListResponse { torrents }))
    }
}

fn to_status(op: &str, err: deluge::Error) -> Status {
    eprintln!("{op} failed: {err}");

    match err {
        deluge::Error::Http(_) | deluge::Error::Status(_) => Status::unavailable(err.to_string()),
        deluge::Error::Auth | deluge::Error::NoHosts | deluge::Error::Rpc { .. } => {
            Status::failed_precondition(err.to_string())
        }
        deluge::Error::Parse(_) => Status::internal(err.to_string()),
        deluge::Error::Uri => Status::invalid_argument(err.to_string()),
    }
}
