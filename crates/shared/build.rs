fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The protos live outside this crate, so cargo won't notice edits to them
    // on its own.
    println!("cargo:rerun-if-changed=../../proto");
    tonic_prost_build::compile_protos("../../proto/download/v1/download.proto")?;
    tonic_prost_build::compile_protos("../../proto/search/v1/search.proto")?;
    Ok(())
}
