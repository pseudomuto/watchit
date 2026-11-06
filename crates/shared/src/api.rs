pub mod search {
    tonic::include_proto!("search.v1");

    pub use search_service_client::SearchServiceClient;
    pub use search_service_server::{SearchService, SearchServiceServer};
}
