fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Compile the three-file proto layout (v1 normative):
    //   types.proto   — shared geometry, node, mutation, and zone types
    //   events.proto  — event types (imports types.proto)
    //   session.proto — HudSession gRPC service (imports types.proto and events.proto)
    //
    // types.proto and events.proto both use package tze_hud.protocol.v1, so their
    // generated types live in the same Rust module as session.proto's parent package.
    // session.proto uses package tze_hud.protocol.v1.session.
    tonic_build::configure().compile_protos(
        &[
            "proto/types.proto",
            "proto/events.proto",
            "proto/session.proto",
        ],
        &["proto"],
    )?;
    Ok(())
}
