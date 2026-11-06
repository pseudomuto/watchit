use shared::api::search::{SearchRequest, SearchResponse, SearchService};
use tonic::{Request, Response, Status};

#[derive(Default)]
pub struct Server {}

#[tonic::async_trait]
impl SearchService for Server {
    async fn search(&self, request: Request<SearchRequest>) -> Result<Response<SearchResponse>, Status> {
        let req = request.into_inner();
        println!("Got: {:?}", req.term);

        let resp = SearchResponse { results: Vec::new() };

        Ok(Response::new(resp))
    }
}
