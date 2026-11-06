use shared::api::search::SearchRequest;

mod api;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "[::1]:50051".parse().unwrap();

    let mut client = api::Client::new(addr).await?;
    let resp = client.search.search(SearchRequest { term: "Hello!".into() }).await?;

    println!("Items for term Hello!: {:?}", resp.into_inner().results.len());

    Ok(())
}
