use shared::api::search::{SearchRequest, SearchResponse, SearchService};
use tonic::{Request, Response, Status};

use crate::jackett;

pub struct Server {
    jackett: jackett::Client,
}

impl Server {
    pub fn new(jackett: jackett::Client) -> Self {
        Self { jackett }
    }
}

#[tonic::async_trait]
impl SearchService for Server {
    async fn search(&self, request: Request<SearchRequest>) -> Result<Response<SearchResponse>, Status> {
        let req = request.into_inner();
        let term = req.term.trim();
        if term.is_empty() {
            return Err(Status::invalid_argument("term must not be blank"));
        }

        let results = self.jackett.search(term).await.map_err(to_status)?;

        Ok(Response::new(SearchResponse { results }))
    }
}

fn to_status(err: jackett::Error) -> Status {
    eprintln!("search failed: {err}");

    match err {
        jackett::Error::Http(_) | jackett::Error::Status(_) => Status::unavailable(err.to_string()),
        jackett::Error::Torznab { .. } => Status::failed_precondition(err.to_string()),
        jackett::Error::Parse(_) => Status::internal(err.to_string()),
    }
}
