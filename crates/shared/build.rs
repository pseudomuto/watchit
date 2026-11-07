fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_prost_build::compile_protos("../../proto/download/v1/download.proto")?;
    tonic_prost_build::compile_protos("../../proto/search/v1/search.proto")?;
    Ok(())
}
