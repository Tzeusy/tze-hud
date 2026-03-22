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
