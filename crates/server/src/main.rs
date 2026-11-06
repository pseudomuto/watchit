use shared::api::search::SearchServiceServer;
use tonic::transport::Server;

mod services;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Yo server");

    let addr = "[::1]:50051".parse().unwrap();
    Server::builder()
        .add_service(SearchServiceServer::new(services::SearchServer::default()))
        .serve(addr)
        .await?;

    Ok(())
}
