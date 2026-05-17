fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The frozen M1 contract (phase-4-m1.md Δ1–Δ9) plus the canonical
    // Apache Arrow Flight IDL, vendored verbatim (Apache-2.0). The
    // Flight service is the bulk-geometry transport (phase-4-m6.md
    // Decision 26); it is a separate, standard service co-served next
    // to MiliViz — mili_viz.proto itself is unchanged/frozen.
    let mili_viz = "proto/mili_viz.proto";
    let flight = "proto/Flight.proto";
    println!("cargo:rerun-if-changed={mili_viz}");
    println!("cargo:rerun-if-changed={flight}");

    // Pure-Rust compile: protox parses the .proto files into a
    // FileDescriptorSet (no protoc binary needed — the parity/web
    // runners do not have protoc; protox bundles the google.protobuf
    // well-known types Flight.proto imports), then tonic-prost-build
    // emits the Rust service + message types from that descriptor set.
    let fds = protox::compile([mili_viz, flight], ["proto"])?;

    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_fds(fds)?;

    Ok(())
}
