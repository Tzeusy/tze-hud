fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Compile both proto files. session.proto imports scene_service.proto.
    // session.proto uses package `tze_hud.protocol.v1.session` so its
    // generated types nest inside the existing `tze_hud.protocol.v1` module
    // and can reference scene_service.proto types via `super::`.
    tonic_build::configure().compile_protos(
        &[
            "proto/scene_service.proto",
            "proto/session.proto",
        ],
        &["proto"],
    )?;
    Ok(())
}
