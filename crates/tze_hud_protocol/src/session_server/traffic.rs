// ─── Traffic Class ───────────────────────────────────────────────────────────

use crate::proto::session::MutationBatch;
use crate::proto::session::server_message::Payload as ServerPayload;

/// Traffic class for outbound server messages (RFC 0005 §3.1, §3.2).
///
/// Each class has different delivery guarantees:
/// - Transactional: at-least-once, ordered, never dropped.
/// - StateStream: at-least-once with coalescing; intermediate states may be skipped.
/// - Ephemeral: at-most-once, latest-wins, dropped under backpressure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrafficClass {
    /// Reliable, ordered, never dropped. MutationResult, LeaseResponse, SessionEstablished, etc.
    Transactional,
    /// Coalesced under pressure; intermediate states may be skipped. SceneSnapshot, TelemetryFrame.
    StateStream,
    /// Droppable under backpressure; latest value wins. Heartbeat echo, ephemeral ZonePublish.
    Ephemeral,
}

/// Classify an outbound `ServerMessage` payload into its traffic class.
///
/// Per RFC 0005 §3.1 and §3.2:
/// - Session lifecycle responses, MutationResult, LeaseResponse, LeaseStateChange,
///   SubscriptionChangeResult, ZonePublishResult, RuntimeError, BackpressureSignal,
///   SessionSuspended, SessionResumed, and input-control responses are Transactional.
/// - SceneSnapshot, SceneDelta, EventBatch, TelemetryFrame are StateStream.
/// - Heartbeat echoes are Ephemeral.
pub fn classify_server_payload(payload: &ServerPayload) -> TrafficClass {
    match payload {
        // Session lifecycle — always transactional
        ServerPayload::SessionEstablished(_)
        | ServerPayload::SessionError(_)
        | ServerPayload::SessionResumeResult(_)
        | ServerPayload::SessionSuspended(_)
        | ServerPayload::SessionResumed(_)
        | ServerPayload::RuntimeError(_) => TrafficClass::Transactional,

        // Mutation / lease responses — transactional
        ServerPayload::MutationResult(_)
        | ServerPayload::LeaseResponse(_)
        | ServerPayload::LeaseStateChange(_)
        | ServerPayload::CapabilityNotice(_)
        | ServerPayload::SubscriptionChangeResult(_)
        | ServerPayload::ZonePublishResult(_)
        | ServerPayload::InputFocusResponse(_)
        | ServerPayload::InputCaptureResponse(_) => TrafficClass::Transactional,

        // Widget and resource-upload responses — transactional.
        ServerPayload::WidgetPublishResult(_)
        | ServerPayload::WidgetAssetRegisterResult(_)
        | ServerPayload::ResourceUploadAccepted(_)
        | ServerPayload::ResourceStored(_)
        | ServerPayload::ResourceErrorResponse(_) => TrafficClass::Transactional,

        // Backpressure signal — transactional (must not be dropped)
        ServerPayload::BackpressureSignal(_) => TrafficClass::Transactional,

        // Degradation notice — transactional (RFC 0005 §3.4; never dropped)
        ServerPayload::DegradationNotice(_) => TrafficClass::Transactional,

        // Scene state / events / runtime telemetry — state-stream
        ServerPayload::SceneSnapshot(_)
        | ServerPayload::SceneDelta(_)
        | ServerPayload::EventBatch(_)
        | ServerPayload::RuntimeTelemetry(_) => TrafficClass::StateStream,

        // Heartbeat echo — ephemeral (droppable, latest-wins)
        ServerPayload::Heartbeat(_) => TrafficClass::Ephemeral,

        // Agent event emission result — transactional (always delivered)
        ServerPayload::EmitSceneEventResult(_) | ServerPayload::ListElementsResponse(_) => {
            TrafficClass::Transactional
        }

        // Element repositioned event — transactional (drag completion / reset-to-default)
        ServerPayload::ElementRepositioned(_) => TrafficClass::Transactional,

        // ── Media plane (RFC 0014 §2.2.2) ────────────────────────────────────
        // Transactional: admission, teardown, degradation, pause/resume notices,
        // SDP offer — never dropped, must be reliably delivered.
        // NOTE: ServerPayload::MediaEgressOpenResult (field 66) is plain `reserved`
        // in the proto — no variant exists until phase 4 egress is defined.
        ServerPayload::MediaIngressOpenResult(_)
        | ServerPayload::MediaIngressCloseNotice(_)
        | ServerPayload::MediaSdpOffer(_)
        | ServerPayload::MediaDegradationNotice(_)
        | ServerPayload::MediaPauseNotice(_)
        | ServerPayload::MediaResumeNotice(_) => TrafficClass::Transactional,

        // State-stream: per-stream health/degradation updates (coalescible, latest-wins)
        ServerPayload::MediaIngressState(_) => TrafficClass::StateStream,

        // Ephemeral realtime: ICE candidates (latest-wins per candidate family)
        ServerPayload::MediaIceCandidate(_) => TrafficClass::Ephemeral,

        // ── Phase 4b cloud-relay (RFC 0018 §4.3) ─────────────────────────────
        // Transactional: relay open result and close notice
        ServerPayload::CloudRelayOpenResult(_) | ServerPayload::CloudRelayCloseNotice(_) => {
            TrafficClass::Transactional
        }

        // State-stream: relay path health (coalescible, latest-wins)
        ServerPayload::CloudRelayStateUpdate(_) => TrafficClass::StateStream,
    }
}

// ─── Inbound mutation traffic class ──────────────────────────────────────────

/// Traffic class for an **inbound** `MutationBatch`.
///
/// Classify an inbound `MutationBatch` by examining its contained mutations.
///
/// Any structural/identity-changing mutation makes the batch Transactional;
/// otherwise content mutations are StateStream; empty batch is Ephemeral.
/// Uses the same `TrafficClass` enum as outbound classification (RFC 0005 §3).
pub(super) fn classify_inbound_batch(batch: &MutationBatch) -> TrafficClass {
    for m in &batch.mutations {
        if let Some(ref mutation) = m.mutation {
            use crate::proto::mutation_proto::Mutation;
            match mutation {
                Mutation::CreateTile(_) => return TrafficClass::Transactional,
                // AddNode is structural — marks the batch as Transactional.
                Mutation::AddNode(_) => return TrafficClass::Transactional,
                // SetTileRoot, UpdateTileOpacity, UpdateTileInputMode are StateStream.
                Mutation::SetTileRoot(_) => {}
                Mutation::UpdateTileOpacity(_) => {}
                Mutation::UpdateTileInputMode(_) => {}
                Mutation::PublishToZone(_) => {}
                Mutation::PublishToTile(_) => {}
                Mutation::ClearZone(_) => {}
                Mutation::ClearWidget(_) => {}
                // UpdateNodeContent is a content update — StateStream
                Mutation::UpdateNodeContent(_) => {}
                // Scroll mutations: config register is Transactional (structural),
                // offset updates are StateStream (rate-limited local feedback).
                Mutation::RegisterTileScroll(_) => {
                    return TrafficClass::Transactional;
                }
                Mutation::SetScrollOffset(_) => {}
            }
        }
    }
    // If we found any mutation at all, it's StateStream (content update)
    if batch.mutations.is_empty() {
        TrafficClass::Ephemeral
    } else {
        TrafficClass::StateStream
    }
}
