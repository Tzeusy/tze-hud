//! Conversion between protobuf types and scene graph types.

use crate::proto;
use tze_hud_scene::*;

// ─── Identity round-trips ─────────────────────────────────────────────────────

/// Encode a `SceneId` as a `SceneIdProto` (16 bytes, little-endian).
pub fn scene_id_to_proto(id: SceneId) -> proto::SceneIdProto {
    proto::SceneIdProto { bytes: id.to_bytes_le().to_vec() }
}

/// Decode a `SceneIdProto` back to a `SceneId`.
///
/// Returns `None` if the `bytes` field is not exactly 16 bytes.
pub fn proto_to_scene_id(p: &proto::SceneIdProto) -> Option<SceneId> {
    SceneId::from_bytes_le(&p.bytes)
}

/// Encode a `ResourceId` as a `ResourceIdProto` (32 raw bytes, never hex).
pub fn resource_id_to_proto(id: ResourceId) -> proto::ResourceIdProto {
    proto::ResourceIdProto { bytes: id.as_bytes().to_vec() }
}

/// Decode a `ResourceIdProto` back to a `ResourceId`.
///
/// Returns `None` if the `bytes` field is not exactly 32 bytes.
pub fn proto_to_resource_id(p: &proto::ResourceIdProto) -> Option<ResourceId> {
    ResourceId::from_slice(&p.bytes)
}

// ─── Geometry ─────────────────────────────────────────────────────────────────

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
        Some(proto::node_proto::Data::StaticImage(si)) => {
            let bounds = si.bounds.as_ref().map(proto_rect_to_scene).unwrap_or(Rect::new(0.0, 0.0, 100.0, 100.0));
            let fit_mode = match proto::ImageFitModeProto::try_from(si.fit_mode).unwrap_or(proto::ImageFitModeProto::ImageFitModeUnspecified) {
                proto::ImageFitModeProto::ImageFitModeContain | proto::ImageFitModeProto::ImageFitModeUnspecified => ImageFitMode::Contain,
                proto::ImageFitModeProto::ImageFitModeCover => ImageFitMode::Cover,
                proto::ImageFitModeProto::ImageFitModeFill => ImageFitMode::Fill,
                proto::ImageFitModeProto::ImageFitModeScaleDown => ImageFitMode::ScaleDown,
            };
            NodeData::StaticImage(StaticImageNode {
                image_data: si.image_data.clone(),
                width: si.width,
                height: si.height,
                content_hash: si.content_hash.clone(),
                fit_mode,
                bounds,
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
        ephemeral: z.ephemeral,
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

/// Convert a scene Node to a protobuf NodeProto.
pub fn scene_node_to_proto(n: &Node) -> proto::NodeProto {
    let data = match &n.data {
        NodeData::SolidColor(sc) => Some(proto::node_proto::Data::SolidColor(proto::SolidColorNodeProto {
            color: Some(proto::Rgba { r: sc.color.r, g: sc.color.g, b: sc.color.b, a: sc.color.a }),
            bounds: Some(proto::Rect { x: sc.bounds.x, y: sc.bounds.y, width: sc.bounds.width, height: sc.bounds.height }),
        })),
        NodeData::TextMarkdown(tm) => Some(proto::node_proto::Data::TextMarkdown(proto::TextMarkdownNodeProto {
            content: tm.content.clone(),
            bounds: Some(proto::Rect { x: tm.bounds.x, y: tm.bounds.y, width: tm.bounds.width, height: tm.bounds.height }),
            font_size_px: tm.font_size_px,
            color: Some(proto::Rgba { r: tm.color.r, g: tm.color.g, b: tm.color.b, a: tm.color.a }),
            background: tm.background.map(|c| proto::Rgba { r: c.r, g: c.g, b: c.b, a: c.a }),
        })),
        NodeData::HitRegion(hr) => Some(proto::node_proto::Data::HitRegion(proto::HitRegionNodeProto {
            bounds: Some(proto::Rect { x: hr.bounds.x, y: hr.bounds.y, width: hr.bounds.width, height: hr.bounds.height }),
            interaction_id: hr.interaction_id.clone(),
            accepts_focus: hr.accepts_focus,
            accepts_pointer: hr.accepts_pointer,
        })),
        NodeData::StaticImage(si) => {
            let fit_mode = match si.fit_mode {
                ImageFitMode::Contain => proto::ImageFitModeProto::ImageFitModeContain as i32,
                ImageFitMode::Cover => proto::ImageFitModeProto::ImageFitModeCover as i32,
                ImageFitMode::Fill => proto::ImageFitModeProto::ImageFitModeFill as i32,
                ImageFitMode::ScaleDown => proto::ImageFitModeProto::ImageFitModeScaleDown as i32,
            };
            Some(proto::node_proto::Data::StaticImage(proto::StaticImageNodeProto {
                image_data: si.image_data.clone(),
                width: si.width,
                height: si.height,
                content_hash: si.content_hash.clone(),
                fit_mode,
                bounds: Some(proto::Rect { x: si.bounds.x, y: si.bounds.y, width: si.bounds.width, height: si.bounds.height }),
            }))
        }
    };
    proto::NodeProto {
        id: n.id.to_string(),
        data,
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── SceneId / ResourceId proto round-trips ───────────────────────────────

    #[test]
    fn scene_id_proto_round_trip() {
        let id = SceneId::new();
        let proto = scene_id_to_proto(id);
        assert_eq!(proto.bytes.len(), 16, "SceneId proto must be 16 bytes");
        let restored = proto_to_scene_id(&proto).expect("must decode 16 bytes");
        assert_eq!(id, restored, "SceneId proto round-trip must be lossless");
    }

    #[test]
    fn scene_id_null_proto_round_trip() {
        let null = SceneId::null();
        let proto = scene_id_to_proto(null);
        assert_eq!(proto.bytes, vec![0u8; 16]);
        let restored = proto_to_scene_id(&proto).unwrap();
        assert!(restored.is_null());
    }

    #[test]
    fn scene_id_proto_rejects_wrong_length() {
        let bad = crate::proto::SceneIdProto { bytes: vec![0u8; 15] };
        assert!(proto_to_scene_id(&bad).is_none());
    }

    #[test]
    fn resource_id_proto_round_trip() {
        let id = ResourceId::of(b"proto round-trip test content");
        let proto = resource_id_to_proto(id);
        assert_eq!(proto.bytes.len(), 32, "ResourceId proto must be 32 bytes");
        let restored = proto_to_resource_id(&proto).expect("must decode 32 bytes");
        assert_eq!(id, restored, "ResourceId proto round-trip must be lossless");
    }

    #[test]
    fn resource_id_proto_bytes_are_raw_not_hex() {
        let id = ResourceId::of(b"no hex on the wire");
        let proto = resource_id_to_proto(id);
        // The bytes field must not be a hex string — verify it has exactly 32 bytes
        // and that it matches the raw hash bytes.
        assert_eq!(&proto.bytes[..], id.as_bytes());
    }

    #[test]
    fn resource_id_proto_rejects_wrong_length() {
        let bad = crate::proto::ResourceIdProto { bytes: vec![0u8; 31] };
        assert!(proto_to_resource_id(&bad).is_none());
    }

    fn make_static_image_node(fit_mode: ImageFitMode) -> Node {
        let pixel_count = 4u32 * 4u32;
        let image_data: Vec<u8> = (0..pixel_count).flat_map(|_| [255u8, 128, 0, 255]).collect();
        Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::StaticImage(StaticImageNode {
                image_data,
                width: 4,
                height: 4,
                content_hash: "deadbeef".to_string(),
                fit_mode,
                bounds: Rect::new(10.0, 20.0, 80.0, 60.0),
            }),
        }
    }

    #[test]
    fn test_static_image_proto_roundtrip_contain() {
        let original = make_static_image_node(ImageFitMode::Contain);
        let proto = scene_node_to_proto(&original);
        let restored = proto_node_to_scene(&proto).expect("conversion must succeed");

        if let NodeData::StaticImage(si) = &restored.data {
            assert_eq!(si.width, 4);
            assert_eq!(si.height, 4);
            assert_eq!(si.content_hash, "deadbeef");
            assert_eq!(si.fit_mode, ImageFitMode::Contain);
            assert_eq!(si.bounds.x, 10.0);
            assert_eq!(si.bounds.y, 20.0);
            assert_eq!(si.image_data.len(), 4 * 4 * 4);
        } else {
            panic!("expected StaticImage variant after proto roundtrip");
        }
    }

    #[test]
    fn test_static_image_proto_roundtrip_all_fit_modes() {
        for (fit_mode, label) in [
            (ImageFitMode::Contain, "Contain"),
            (ImageFitMode::Cover, "Cover"),
            (ImageFitMode::Fill, "Fill"),
            (ImageFitMode::ScaleDown, "ScaleDown"),
        ] {
            let original = make_static_image_node(fit_mode);
            let proto = scene_node_to_proto(&original);
            let restored = proto_node_to_scene(&proto)
                .unwrap_or_else(|| panic!("conversion failed for {}", label));
            if let NodeData::StaticImage(si) = &restored.data {
                assert_eq!(si.fit_mode, fit_mode, "fit_mode mismatch for {}", label);
            } else {
                panic!("wrong variant for {}", label);
            }
        }
    }

    #[test]
    fn test_static_image_proto_preserves_pixel_data() {
        // Verify pixel data survives proto encode/decode.
        let image_data: Vec<u8> = (0..16u8).flat_map(|i| [i, i * 2, 255 - i, 128]).collect();
        let node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::StaticImage(StaticImageNode {
                image_data: image_data.clone(),
                width: 4,
                height: 1,
                content_hash: "test-hash".to_string(),
                fit_mode: ImageFitMode::Fill,
                bounds: Rect::new(0.0, 0.0, 100.0, 25.0),
            }),
        };

        let proto = scene_node_to_proto(&node);
        let restored = proto_node_to_scene(&proto).unwrap();
        if let NodeData::StaticImage(si) = &restored.data {
            assert_eq!(si.image_data, image_data);
        } else {
            panic!("wrong variant");
        }
    }
}
