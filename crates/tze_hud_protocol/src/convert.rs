//! Conversion between protobuf types and scene graph types.

use crate::proto;
use tze_hud_scene::*;

/// Convert a protobuf Rect to a scene Rect.
pub fn proto_rect_to_scene(r: &proto::Rect) -> Rect {
    Rect::new(r.x, r.y, r.width, r.height)
}

/// Convert a protobuf Rgba to a scene Rgba.
pub fn proto_rgba_to_scene(c: &proto::Rgba) -> Rgba {
    Rgba::new(c.r, c.g, c.b, c.a)
}

/// Convert a protobuf NodeProto to a scene Node.
pub fn proto_node_to_scene(n: &proto::NodeProto) -> Option<Node> {
    let id = if n.id.is_empty() {
        SceneId::new()
    } else {
        // Parse UUID from string
        uuid::Uuid::parse_str(&n.id)
            .map(SceneId::from_uuid)
            .unwrap_or_else(|_| SceneId::new())
    };

    let data = match &n.data {
        Some(proto::node_proto::Data::SolidColor(sc)) => {
            let color = sc.color.as_ref().map(proto_rgba_to_scene).unwrap_or(Rgba::WHITE);
            let bounds = sc.bounds.as_ref().map(proto_rect_to_scene).unwrap_or(Rect::new(0.0, 0.0, 100.0, 100.0));
            NodeData::SolidColor(SolidColorNode { color, bounds })
        }
        Some(proto::node_proto::Data::TextMarkdown(tm)) => {
            let color = tm.color.as_ref().map(proto_rgba_to_scene).unwrap_or(Rgba::WHITE);
            let bg = tm.background.as_ref().map(proto_rgba_to_scene);
            let bounds = tm.bounds.as_ref().map(proto_rect_to_scene).unwrap_or(Rect::new(0.0, 0.0, 100.0, 100.0));
            NodeData::TextMarkdown(TextMarkdownNode {
                content: tm.content.clone(),
                bounds,
                font_size_px: if tm.font_size_px > 0.0 { tm.font_size_px } else { 16.0 },
                font_family: FontFamily::SystemSansSerif,
                color,
                background: bg,
                alignment: TextAlign::Start,
                overflow: TextOverflow::Clip,
            })
        }
        Some(proto::node_proto::Data::HitRegion(hr)) => {
            let bounds = hr.bounds.as_ref().map(proto_rect_to_scene).unwrap_or(Rect::new(0.0, 0.0, 100.0, 50.0));
            NodeData::HitRegion(HitRegionNode {
                bounds,
                interaction_id: hr.interaction_id.clone(),
                accepts_focus: hr.accepts_focus,
                accepts_pointer: hr.accepts_pointer,
            })
        }
        None => return None,
    };

    Some(Node {
        id,
        children: vec![],
        data,
    })
}

// ─── Zone conversions ─────────────────────────────────────────────────────────

/// Convert a protobuf ZoneContent to a scene ZoneContent.
pub fn proto_zone_content_to_scene(c: &proto::ZoneContent) -> Option<ZoneContent> {
    use proto::zone_content::Payload;
    match c.payload.as_ref()? {
        Payload::StreamText(s) => Some(ZoneContent::StreamText(s.clone())),
        Payload::Notification(n) => Some(ZoneContent::Notification(NotificationPayload {
            text: n.text.clone(),
            icon: n.icon.clone(),
            urgency: n.urgency,
        })),
        Payload::StatusBar(sb) => Some(ZoneContent::StatusBar(StatusBarPayload {
            entries: sb.entries.clone(),
        })),
        Payload::SolidColor(c) => Some(ZoneContent::SolidColor(proto_rgba_to_scene(c))),
    }
}

/// Convert a scene ZoneContent to a protobuf ZoneContent.
pub fn scene_zone_content_to_proto(c: &ZoneContent) -> proto::ZoneContent {
    use proto::zone_content::Payload;
    let payload = match c {
        ZoneContent::StreamText(s) => Payload::StreamText(s.clone()),
        ZoneContent::Notification(n) => Payload::Notification(proto::NotificationPayload {
            text: n.text.clone(),
            icon: n.icon.clone(),
            urgency: n.urgency,
        }),
        ZoneContent::StatusBar(sb) => Payload::StatusBar(proto::StatusBarPayload {
            entries: sb.entries.clone(),
        }),
        ZoneContent::SolidColor(c) => Payload::SolidColor(proto::Rgba {
            r: c.r,
            g: c.g,
            b: c.b,
            a: c.a,
        }),
    };
    proto::ZoneContent { payload: Some(payload) }
}

/// Convert a scene ZoneRegistrySnapshot to a protobuf ZoneRegistrySnapshotProto.
pub fn zone_registry_snapshot_to_proto(
    snap: &ZoneRegistrySnapshot,
) -> proto::ZoneRegistrySnapshotProto {
    let zones = snap.zones.iter().map(zone_definition_to_proto).collect();
    let active_publishes = snap
        .active_publishes
        .iter()
        .map(zone_publish_record_to_proto)
        .collect();
    proto::ZoneRegistrySnapshotProto {
        zones,
        active_publishes,
    }
}

/// Convert a scene ZoneDefinition to a protobuf ZoneDefinitionProto.
pub fn zone_definition_to_proto(z: &ZoneDefinition) -> proto::ZoneDefinitionProto {
    let geometry_policy = Some(geometry_policy_to_proto(&z.geometry_policy));
    let accepted_media_types = z
        .accepted_media_types
        .iter()
        .map(|mt| format!("{:?}", mt))
        .collect();
    let rendering_policy = Some(rendering_policy_to_proto(&z.rendering_policy));

    let (contention, stack_max_depth, merge_max_keys) = match z.contention_policy {
        ContentionPolicy::LatestWins => (
            proto::ContentionPolicyProto::ContentionPolicyLatestWins as i32,
            0,
            0,
        ),
        ContentionPolicy::Replace => (
            proto::ContentionPolicyProto::ContentionPolicyReplace as i32,
            0,
            0,
        ),
        ContentionPolicy::Stack { max_depth } => (
            proto::ContentionPolicyProto::ContentionPolicyStack as i32,
            max_depth as u32,
            0,
        ),
        ContentionPolicy::MergeByKey { max_keys } => (
            proto::ContentionPolicyProto::ContentionPolicyMergeByKey as i32,
            0,
            max_keys as u32,
        ),
    };

    proto::ZoneDefinitionProto {
        id: z.id.to_string(),
        name: z.name.clone(),
        description: z.description.clone(),
        geometry_policy,
        accepted_media_types,
        rendering_policy,
        contention_policy: contention,
        max_publishers: z.max_publishers,
        auto_clear_ms: z.auto_clear_ms.unwrap_or(0),
        stack_max_depth,
        merge_max_keys,
    }
}

/// Convert a scene GeometryPolicy to a protobuf GeometryPolicyProto.
pub fn geometry_policy_to_proto(gp: &GeometryPolicy) -> proto::GeometryPolicyProto {
    use proto::geometry_policy_proto::Policy;
    let policy = match gp {
        GeometryPolicy::Relative { x_pct, y_pct, width_pct, height_pct } => {
            Policy::Relative(proto::RelativeGeometryPolicy {
                x_pct: *x_pct,
                y_pct: *y_pct,
                width_pct: *width_pct,
                height_pct: *height_pct,
            })
        }
        GeometryPolicy::EdgeAnchored { edge, height_pct, width_pct, margin_px } => {
            let edge_proto = match edge {
                DisplayEdge::Top => proto::DisplayEdge::Top,
                DisplayEdge::Bottom => proto::DisplayEdge::Bottom,
                DisplayEdge::Left => proto::DisplayEdge::Left,
                DisplayEdge::Right => proto::DisplayEdge::Right,
            };
            Policy::EdgeAnchored(proto::EdgeAnchoredGeometryPolicy {
                edge: edge_proto as i32,
                height_pct: *height_pct,
                width_pct: *width_pct,
                margin_px: *margin_px,
            })
        }
    };
    proto::GeometryPolicyProto { policy: Some(policy) }
}

/// Convert a scene RenderingPolicy to a protobuf RenderingPolicyProto.
pub fn rendering_policy_to_proto(rp: &RenderingPolicy) -> proto::RenderingPolicyProto {
    proto::RenderingPolicyProto {
        font_size_px: rp.font_size_px.unwrap_or(0.0),
        backdrop: rp.backdrop.map(|c| proto::Rgba { r: c.r, g: c.g, b: c.b, a: c.a }),
        text_align: rp.text_align.map(|ta| match ta {
            TextAlign::Start => proto::TextAlignProto::Start as i32,
            TextAlign::Center => proto::TextAlignProto::Center as i32,
            TextAlign::End => proto::TextAlignProto::End as i32,
        }).unwrap_or(proto::TextAlignProto::Unspecified as i32),
        margin_px: rp.margin_px.unwrap_or(0.0),
    }
}

/// Convert a scene ZonePublishRecord to a protobuf ZonePublishRecordProto.
pub fn zone_publish_record_to_proto(r: &ZonePublishRecord) -> proto::ZonePublishRecordProto {
    proto::ZonePublishRecordProto {
        zone_name: r.zone_name.clone(),
        publisher_namespace: r.publisher_namespace.clone(),
        content: Some(scene_zone_content_to_proto(&r.content)),
        published_at_ms: r.published_at_ms,
        merge_key: r.merge_key.clone().unwrap_or_default(),
    }
}
