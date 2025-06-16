use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = PathBuf::from("src/generated");
    std::fs::create_dir_all(&out_dir)?;

    tonic_build::configure()
        .out_dir(out_dir)
        .compile_protos(&["proto/mcproc.proto"], &["proto"])?;

    Ok(())
}