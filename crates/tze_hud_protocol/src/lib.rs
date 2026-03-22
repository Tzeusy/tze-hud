//! # tze_hud_protocol
//!
//! gRPC protocol layer for tze_hud. Defines the service, server implementation,
//! and client helpers for agent communication.

#[deprecated(note = "Use session_server module with HudSession streaming protocol instead")]
pub mod server;
pub mod session;
pub mod convert;
pub mod session_server;

/// Generated protobuf types and gRPC service definitions (unary RPC, transitional).
///
/// The `session` submodule contains the new bidirectional streaming session
/// protocol types (RFC 0005). Use `proto::session::*` for the new protocol.
pub mod proto {
    tonic::include_proto!("tze_hud.protocol.v1");

    /// Generated types for the bidirectional streaming session protocol (RFC 0005).
    pub mod session {
        tonic::include_proto!("tze_hud.protocol.v1.session");
    }
}
