fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto = "proto/mili_viz.proto";
    println!("cargo:rerun-if-changed={proto}");

    // Pure-Rust compile: protox parses the .proto into a
    // FileDescriptorSet (no protoc binary needed — the parity/web
    // runners do not have protoc), then tonic-prost-build emits the
    // Rust service + message types from that descriptor set.
    let fds = protox::compile([proto], ["proto"])?;

    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_fds(fds)?;

    Ok(())
}
