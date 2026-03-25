fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Compile the four-file proto layout (v1 normative + legacy compat):
    //   types.proto         — shared geometry, node, mutation, and zone types
    //   events.proto        — current RFC 0004 event types (imports types.proto)
    //   events_legacy.proto — DEPRECATED legacy wire messages (InputEvent, SceneEvent, etc.);
    //                         same package as events.proto so generated types land in the
    //                         same Rust module. Import only for backwards-compatibility.
    //   session.proto       — HudSession gRPC service (imports types.proto, events.proto,
    //                         and events_legacy.proto for SceneDelta)
    //
    // All four files use package tze_hud.protocol.v1 (or its sub-package for session),
    // so generated types live in the same Rust module.
    // session.proto uses package tze_hud.protocol.v1.session.
    tonic_build::configure().compile_protos(
        &[
            "proto/types.proto",
            "proto/events.proto",
            "proto/events_legacy.proto",
            "proto/session.proto",
        ],
        &["proto"],
    )?;
    Ok(())
}
