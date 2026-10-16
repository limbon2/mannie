//! The code that the build generates from the link's protobuf files, one
//! module for each package (`proto/mannie/link/`).

// The generated code follows prost's and tonic's style, not this
// workspace's: it qualifies its paths, allows lints with `#[allow]` and
// builds long functions.
#![expect(
    unused_qualifications,
    clippy::allow_attributes,
    clippy::allow_attributes_without_reason,
    clippy::clone_on_ref_ptr,
    clippy::default_trait_access,
    clippy::doc_markdown,
    clippy::empty_structs_with_brackets,
    clippy::too_many_lines,
    clippy::trivially_copy_pass_by_ref,
    reason = "generated code, which the protobuf files and the generators shape"
)]

/// `mannie.link.session.v1`.
pub mod session {
    tonic::include_proto!("mannie.link.session.v1");
}

/// `mannie.link.pairing.v1`.
pub mod pairing {
    tonic::include_proto!("mannie.link.pairing.v1");
}

/// `mannie.link.tasks.v1`.
pub mod tasks {
    tonic::include_proto!("mannie.link.tasks.v1");
}

/// `mannie.link.events.v1`.
pub mod events {
    tonic::include_proto!("mannie.link.events.v1");
}

/// `mannie.link.approvals.v1`.
pub mod approvals {
    tonic::include_proto!("mannie.link.approvals.v1");
}

/// `mannie.link.leases.v1`.
pub mod leases {
    tonic::include_proto!("mannie.link.leases.v1");
}
