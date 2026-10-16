//! Generates the link's Rust code from its protobuf files (ADR-0016). protox
//! compiles them without a system `protoc`, and the code goes to Cargo's
//! output folder; it is never committed.

use std::error::Error;
use std::path::PathBuf;

/// The link's packages, one file each.
const FILES: [&str; 6] = [
    "mannie/link/session/v1/session.proto",
    "mannie/link/pairing/v1/pairing.proto",
    "mannie/link/tasks/v1/tasks.proto",
    "mannie/link/events/v1/events.proto",
    "mannie/link/approvals/v1/approvals.proto",
    "mannie/link/leases/v1/leases.proto",
];

fn main() -> Result<(), Box<dyn Error>> {
    let proto = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?).join("../../proto");
    println!("cargo:rerun-if-changed={}", proto.display());
    let descriptors = protox::compile(FILES, [&proto])?;
    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .compile_fds(descriptors)?;
    Ok(())
}
