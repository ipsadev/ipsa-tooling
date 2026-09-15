fn main() {
    let protos = ["proto/stellar_gateway.proto", "proto/proofapi.proto"];

    for proto in &protos {
        println!("cargo:rerun-if-changed={proto}");
    }

    tonic_prost_build::configure()
        .build_client(true)
        .build_server(false)
        .compile_protos(&protos, &["proto/"])
        .unwrap();
}
