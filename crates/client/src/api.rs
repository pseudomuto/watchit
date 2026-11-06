use std::net::SocketAddr;

use shared::api::search::SearchServiceClient;
use tonic::transport::Channel;

#[derive(Debug)]
pub struct Client {
    pub search: SearchServiceClient<Channel>,
}

impl Client {
    pub async fn new(addr: SocketAddr) -> color_eyre::Result<Self> {
        let client = SearchServiceClient::connect(format!("http://{addr}")).await?;

        Ok(Self { search: client })
    }
}
