fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Only compile if proto file exists
    let proto_path = "proto/claudebot_bridge.proto";
    if std::path::Path::new(proto_path).exists() {
        tonic_build::configure()
            .build_server(true)
            .build_client(true)
            .out_dir("src/bridge/generated")
            .compile_protos(&[proto_path], &["proto"])?;

        println!("cargo:rerun-if-changed={}", proto_path);
    }

    Ok(())
}
