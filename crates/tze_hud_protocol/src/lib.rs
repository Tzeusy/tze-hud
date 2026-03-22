//! # tze_hud_protocol
//!
//! gRPC protocol layer for tze_hud. Defines the service, server implementation,
//! and client helpers for agent communication.

pub mod server;
pub mod session;
pub mod convert;

/// Generated protobuf types and gRPC service definitions.
pub mod proto {
    tonic::include_proto!("tze_hud.protocol.v1");
}
