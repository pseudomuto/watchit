pub mod download {
    tonic::include_proto!("download.v1");

    pub use download_service_client::DownloadServiceClient;
    pub use download_service_server::{DownloadService, DownloadServiceServer};
}

pub mod search {
    tonic::include_proto!("search.v1");

    pub use search_service_client::SearchServiceClient;
    pub use search_service_server::{SearchService, SearchServiceServer};
}
