//! CPU-testable fixture, pixel observation, and fail-closed evaluation for the
//! reference-Windows VerticalFlow proof.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tze_hud_compositor::fonts::bundled_font_system;
use tze_hud_compositor::markdown::MarkdownTokens;
use tze_hud_compositor::vertical_flow::resolve_tile_flow_offsets;
use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::types::{
    Capability, FontFamily, Node, NodeData, NodeLayout, Rect, Rgba, SceneId, SolidColorNode,
    TextAlign, TextColorRun, TextMarkdownNode, TextOverflow,
};

pub const REFERENCE_TAG: &str = "TzeHouse";
pub const REFERENCE_WIDTH: u32 = 4096;
pub const REFERENCE_HEIGHT: u32 = 2160;
pub const PROOF_SCHEMA_VERSION: u32 = 1;
pub const CLEAR_SRGB: [u8; 4] = [64, 64, 89, 255];
pub const CHILD_COUNT: usize = 3;
pub const TILE_X: f32 = 410.0;
pub const TILE_Y: f32 = 216.0;
pub const TILE_WIDTH: f32 = 1200.0;
pub const TILE_HEIGHT: f32 = 1200.0;
pub const PARENT_Y: f32 = 90.0;
pub const CHILD_X: f32 = 48.0;
pub const CHILD_WIDTH: f32 = 880.0;
pub const CHILD_HEIGHT: f32 = 220.0;
pub const DECLARED_Y_SENTINEL: f32 = 1050.0;
pub const SECTION_GAP_PX: f32 = 8.0;
pub const CHILD_NAMES: [&str; CHILD_COUNT] =
    ["assistant_markdown", "tool_attributed", "assistant_wrap"];
const REFERENCE_PREFLIGHT_CHECKS: [&str; 4] = [
    "reference_hardware_tag",
    "reference_hardware_metadata",
    "reference_display",
    "display_surface_equal",
];

const CHILD_BACKGROUNDS_LINEAR: [[f32; 4]; CHILD_COUNT] = [
    [0.10, 0.04, 0.04, 1.0],
    [0.04, 0.10, 0.04, 1.0],
    [0.04, 0.04, 0.12, 1.0],
];

const CHILD_CONTENT: [&str; CHILD_COUNT] = [
    "## Assistant turn\nThe runtime measures this parsed markdown.\nBold structure remains **styled** after parsing.\nThe first band has several shaped rows.\nIts explicit source y is a sentinel only.\nThe compositor must replace that y.\nThe following child starts after this measured row.",
    "Tool turn\nThe attributed path keeps raw byte offsets stable.\nIts color run forces raw-content shaping.\nThe runtime still owns the vertical position.\nBackground and glyph origins must agree.\nThe next child clears this measured row.\nNo publisher-computed y participates.",
    "### Assistant wrap proof\nA final markdown child closes the stack.\nIt contains enough rows to exceed its backdrop height.\nThat makes non-overlap depend on measured content.\nThe source y remains deliberately wrong.\nOnly the resolved compositor y may paint.\nThe sentinel region must remain clear.",
];

#[derive(Debug)]
/// Scene graph and stable identifiers used by the production-path readback.
pub struct VerticalFlowFixture {
    pub scene: SceneGraph,
    pub tile_id: SceneId,
    pub parent_id: SceneId,
    pub child_ids: [SceneId; CHILD_COUNT],
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
/// Reference-host metadata supplied by the guarded Windows controller.
pub struct ReferenceHardware {
    pub tag: String,
    pub hostname: String,
    pub gpu: String,
    pub gpu_driver: String,
    pub os: String,
    pub display_width: u32,
    pub display_height: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
/// Integer pixel bounds in the full readback surface.
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl PixelRect {
    /// Returns the exclusive lower edge of the rectangle.
    pub fn bottom(self) -> u32 {
        self.y + self.height
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
/// One named RGBA observation from the proof surface.
pub struct PixelSample {
    pub name: String,
    pub x: u32,
    pub y: u32,
    pub rgba: [u8; 4],
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
/// Background and glyph observations attributed to one flowed child.
pub struct ChildObservation {
    pub name: String,
    pub background_rect: PixelRect,
    pub expected_background_srgb: [u8; 4],
    pub background_sample: Option<PixelSample>,
    pub glyph_bounds: Option<PixelRect>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
/// Raw reference metadata and pixel-derived observations.
pub struct ProofEvidence {
    pub reference_hardware: ReferenceHardware,
    pub render_width: u32,
    pub render_height: u32,
    pub pixel_buffer_len: usize,
    pub children: Vec<ChildObservation>,
    pub gap_samples: Vec<PixelSample>,
    pub sentinel_sample: Option<PixelSample>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
/// One independently diagnosable proof-contract assertion.
pub struct ContractCheck {
    pub code: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
/// Aggregate proof result after every contract check is evaluated.
pub enum ProofVerdict {
    Pass,
    Fail,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
/// Serializable proof artifact written beside the unmodified readback frame.
pub struct ProofReport {
    pub schema_version: u32,
    pub artifact: String,
    pub verdict: ProofVerdict,
    pub checks: Vec<ContractCheck>,
    pub evidence: ProofEvidence,
}

/// Builds the isolated three-child VerticalFlow scene used by the proof.
pub fn build_fixture() -> Result<VerticalFlowFixture, Box<dyn std::error::Error>> {
    let mut scene = SceneGraph::new(REFERENCE_WIDTH as f32, REFERENCE_HEIGHT as f32);
    let tab_id = scene.create_tab("Vertical Flow Pixel Proof", 0)?;
    let lease_id = scene.grant_lease(
        "vertical-flow-proof",
        120_000,
        vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
    );
    let tile_id = scene.create_tile(
        tab_id,
        "vertical-flow-proof",
        lease_id,
        Rect::new(TILE_X, TILE_Y, TILE_WIDTH, TILE_HEIGHT),
        160,
    )?;

    let parent_id = fixed_scene_id(1);
    let child_ids = [fixed_scene_id(2), fixed_scene_id(3), fixed_scene_id(4)];
    let parent = Node {
        id: parent_id,
        children: child_ids.to_vec(),
        layout: NodeLayout::VerticalFlow,
        data: NodeData::SolidColor(SolidColorNode {
            color: Rgba::new(0.0, 0.0, 0.0, 0.0),
            bounds: Rect::new(0.0, PARENT_Y, CHILD_X + CHILD_WIDTH, 0.0),
            radius: None,
        }),
    };
    let children = child_ids
        .into_iter()
        .zip(CHILD_CONTENT)
        .zip(CHILD_BACKGROUNDS_LINEAR)
        .enumerate()
        .map(|(index, ((id, content), background))| {
            let color_runs = if index == 1 {
                Box::from([TextColorRun {
                    start_byte: 0,
                    end_byte: "Tool".len() as u32,
                    color: Rgba::new(1.0, 0.84, 0.30, 1.0),
                }])
            } else {
                Box::default()
            };
            Node {
                id,
                children: vec![],
                layout: NodeLayout::Absolute,
                data: NodeData::TextMarkdown(TextMarkdownNode {
                    content: content.to_string(),
                    bounds: Rect::new(CHILD_X, DECLARED_Y_SENTINEL, CHILD_WIDTH, CHILD_HEIGHT),
                    font_size_px: 28.0,
                    font_family: FontFamily::SystemSansSerif,
                    color: Rgba::new(0.96, 0.97, 1.0, 1.0),
                    background: Some(Rgba::new(
                        background[0],
                        background[1],
                        background[2],
                        background[3],
                    )),
                    alignment: TextAlign::Start,
                    overflow: TextOverflow::Clip,
                    color_runs,
                }),
            }
        })
        .collect();
    scene.set_tile_root_tree(tile_id, parent, children)?;

    Ok(VerticalFlowFixture {
        scene,
        tile_id,
        parent_id,
        child_ids,
    })
}

/// Resolves the fixture's child background rectangles through production layout code.
pub fn resolve_background_rects(
    fixture: &VerticalFlowFixture,
) -> Result<[PixelRect; CHILD_COUNT], Box<dyn std::error::Error>> {
    let mut font_system = bundled_font_system();
    let offsets = resolve_tile_flow_offsets(
        &mut font_system,
        &fixture.scene.nodes,
        SECTION_GAP_PX,
        &MarkdownTokens::default(),
        &HashMap::new(),
    );
    let tile = fixture
        .scene
        .tiles
        .get(&fixture.tile_id)
        .ok_or("proof tile missing from scene")?;
    let rects: Vec<PixelRect> = fixture
        .child_ids
        .iter()
        .map(|id| -> Result<PixelRect, Box<dyn std::error::Error>> {
            let node = fixture.scene.nodes.get(id).ok_or("proof child missing")?;
            let bounds = node.data.bounds();
            let resolved_y = offsets
                .get(id)
                .ok_or("proof child has no resolved flow y")?;
            Ok(pixel_rect(
                tile.bounds.x + bounds.x,
                tile.bounds.y + resolved_y,
                bounds.width,
                bounds.height,
            ))
        })
        .collect::<Result<_, _>>()?;
    rects
        .try_into()
        .map_err(|_| "proof fixture must resolve exactly three child rects".into())
}

/// Returns the fixture background colors encoded as sRGB bytes.
pub fn expected_background_srgb() -> [[u8; 4]; CHILD_COUNT] {
    CHILD_BACKGROUNDS_LINEAR.map(linear_rgba_to_srgb)
}

/// Extracts background, gap, sentinel, and glyph-bound evidence from an RGBA frame.
pub fn observe_pixels(
    reference_hardware: ReferenceHardware,
    fixture: &VerticalFlowFixture,
    render_width: u32,
    render_height: u32,
    pixels: &[u8],
) -> Result<ProofEvidence, Box<dyn std::error::Error>> {
    let rects = resolve_background_rects(fixture)?;
    let expected_colors = expected_background_srgb();
    let children = rects
        .iter()
        .copied()
        .zip(expected_colors)
        .zip(CHILD_NAMES)
        .enumerate()
        .map(|(index, ((rect, expected_background_srgb), name))| {
            let sample_x = rect.x + rect.width.saturating_sub(8);
            let sample_y = rect.y + 8;
            let scan_top = if index == 0 {
                rect.y.saturating_sub(32)
            } else {
                let previous_bottom = rects[index - 1].bottom();
                previous_bottom + rect.y.saturating_sub(previous_bottom) / 2
            };
            let scan_bottom = if index + 1 == CHILD_COUNT {
                rect.bottom().saturating_add(32).min(render_height)
            } else {
                let gap = rects[index + 1].y.saturating_sub(rect.bottom());
                rect.bottom() + gap.div_ceil(2)
            };
            ChildObservation {
                name: name.to_string(),
                background_rect: rect,
                expected_background_srgb,
                background_sample: read_sample(
                    format!("{name}_background"),
                    sample_x,
                    sample_y,
                    render_width,
                    render_height,
                    pixels,
                ),
                glyph_bounds: scan_glyph_bounds(
                    rect,
                    scan_top,
                    scan_bottom,
                    render_width,
                    render_height,
                    pixels,
                ),
            }
        })
        .collect::<Vec<_>>();

    let gap_samples = children
        .windows(2)
        .enumerate()
        .filter_map(|(index, pair)| {
            let before = pair[0].background_rect;
            let after = pair[1].background_rect;
            let gap_start = before.bottom();
            let gap_height = after.y.saturating_sub(gap_start);
            read_sample(
                format!("gap_{}_{}", index + 1, index + 2),
                before.x + before.width.saturating_sub(8),
                gap_start + gap_height / 2,
                render_width,
                render_height,
                pixels,
            )
        })
        .collect();
    let sentinel_sample = read_sample(
        "declared_y_sentinel".to_string(),
        (TILE_X + CHILD_X + CHILD_WIDTH - 8.0) as u32,
        (TILE_Y + DECLARED_Y_SENTINEL + 8.0) as u32,
        render_width,
        render_height,
        pixels,
    );

    Ok(ProofEvidence {
        reference_hardware,
        render_width,
        render_height,
        pixel_buffer_len: pixels.len(),
        children,
        gap_samples,
        sentinel_sample,
    })
}

/// Applies every fail-closed reference and pixel contract to raw evidence.
pub fn evaluate_evidence(evidence: ProofEvidence) -> ProofReport {
    let mut checks = Vec::new();
    let reference = &evidence.reference_hardware;
    push_check(
        &mut checks,
        "reference_hardware_tag",
        reference.tag == REFERENCE_TAG,
        format!("observed={:?} required={REFERENCE_TAG:?}", reference.tag),
    );
    push_check(
        &mut checks,
        "reference_hardware_metadata",
        !reference.hostname.trim().is_empty()
            && !reference.gpu.trim().is_empty()
            && !reference.gpu_driver.trim().is_empty()
            && !reference.os.trim().is_empty(),
        format!(
            "hostname={:?} gpu={:?} driver={:?} os={:?}",
            reference.hostname, reference.gpu, reference.gpu_driver, reference.os
        ),
    );
    push_check(
        &mut checks,
        "reference_display",
        reference.display_width == REFERENCE_WIDTH && reference.display_height == REFERENCE_HEIGHT,
        format!(
            "observed={}x{} required={}x{}",
            reference.display_width, reference.display_height, REFERENCE_WIDTH, REFERENCE_HEIGHT
        ),
    );
    push_check(
        &mut checks,
        "display_surface_equal",
        reference.display_width == evidence.render_width
            && reference.display_height == evidence.render_height,
        format!(
            "display={}x{} render={}x{}",
            reference.display_width,
            reference.display_height,
            evidence.render_width,
            evidence.render_height
        ),
    );
    let expected_len = (REFERENCE_WIDTH * REFERENCE_HEIGHT * 4) as usize;
    push_check(
        &mut checks,
        "render_surface",
        evidence.render_width == REFERENCE_WIDTH
            && evidence.render_height == REFERENCE_HEIGHT
            && evidence.pixel_buffer_len == expected_len,
        format!(
            "surface={}x{} bytes={} required={}x{} bytes={expected_len}",
            evidence.render_width,
            evidence.render_height,
            evidence.pixel_buffer_len,
            REFERENCE_WIDTH,
            REFERENCE_HEIGHT
        ),
    );

    let complete_samples = evidence.children.len() == CHILD_COUNT
        && evidence
            .children
            .iter()
            .all(|child| child.background_sample.is_some() && child.glyph_bounds.is_some())
        && evidence.gap_samples.len() == CHILD_COUNT - 1
        && evidence.sentinel_sample.is_some();
    push_check(
        &mut checks,
        "sample_completeness",
        complete_samples,
        format!(
            "children={} gaps={} sentinel={}",
            evidence.children.len(),
            evidence.gap_samples.len(),
            evidence.sentinel_sample.is_some()
        ),
    );

    let bands_do_not_overlap = evidence.children.len() == CHILD_COUNT
        && evidence
            .children
            .windows(2)
            .all(|pair| pair[0].background_rect.bottom() < pair[1].background_rect.y);
    push_check(
        &mut checks,
        "flow_bands_non_overlapping",
        bands_do_not_overlap,
        format!(
            "bands={:?}",
            evidence
                .children
                .iter()
                .map(|child| child.background_rect)
                .collect::<Vec<_>>()
        ),
    );

    for (index, child) in evidence.children.iter().enumerate() {
        let background_ok = child.background_sample.as_ref().is_some_and(|sample| {
            child.background_rect.contains_point(sample.x, sample.y)
                && rgba_near(sample.rgba, child.expected_background_srgb, 8)
        });
        push_check(
            &mut checks,
            format!("{}_background_at_resolved_y", child.name),
            background_ok,
            format!(
                "rect={:?} sample={:?} expected={:?}",
                child.background_rect, child.background_sample, child.expected_background_srgb
            ),
        );
        let glyph_ok = child.glyph_bounds.is_some_and(|glyphs| {
            glyphs.width > 0 && glyphs.height > 0 && child.background_rect.contains_rect(glyphs)
        });
        push_check(
            &mut checks,
            format!("{}_background_glyph_y_alignment", child.name),
            glyph_ok,
            format!(
                "background={:?} glyphs={:?} index={index}",
                child.background_rect, child.glyph_bounds
            ),
        );
    }

    for (index, sample) in evidence.gap_samples.iter().enumerate() {
        let between_bands = evidence
            .children
            .get(index)
            .zip(evidence.children.get(index + 1));
        let gap_ok = between_bands.is_some_and(|(before, after)| {
            sample.y >= before.background_rect.bottom()
                && sample.y < after.background_rect.y
                && rgba_near(sample.rgba, CLEAR_SRGB, 8)
        });
        push_check(
            &mut checks,
            format!("gap_{}_clear", index + 1),
            gap_ok,
            format!("sample={sample:?}"),
        );
    }

    let sentinel_x = (TILE_X + CHILD_X + CHILD_WIDTH - 8.0) as u32;
    let sentinel_y = (TILE_Y + DECLARED_Y_SENTINEL + 8.0) as u32;
    let sentinel_ok = evidence.sentinel_sample.as_ref().is_some_and(|sample| {
        sample.x == sentinel_x && sample.y == sentinel_y && rgba_near(sample.rgba, CLEAR_SRGB, 8)
    });
    push_check(
        &mut checks,
        "declared_y_sentinel_remains_clear",
        sentinel_ok,
        format!(
            "sample={:?} expected_coordinate=({sentinel_x},{sentinel_y})",
            evidence.sentinel_sample
        ),
    );

    let verdict = if checks.iter().all(|check| check.passed) {
        ProofVerdict::Pass
    } else {
        ProofVerdict::Fail
    };
    ProofReport {
        schema_version: PROOF_SCHEMA_VERSION,
        artifact: "vertical-flow-reference-windows-pixel-proof".to_string(),
        verdict,
        checks,
        evidence,
    }
}

/// Reports whether every reference-host preflight check is present and passing.
pub fn reference_preflight_passes(report: &ProofReport) -> bool {
    REFERENCE_PREFLIGHT_CHECKS.iter().all(|required| {
        report
            .checks
            .iter()
            .find(|check| check.code == *required)
            .is_some_and(|check| check.passed)
    })
}

impl PixelRect {
    fn contains_point(self, x: u32, y: u32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.bottom()
    }

    fn contains_rect(self, other: PixelRect) -> bool {
        self.contains_point(other.x, other.y)
            && other.x + other.width <= self.x + self.width
            && other.bottom() <= self.bottom()
    }
}

fn fixed_scene_id(value: u8) -> SceneId {
    SceneId::from_bytes_le(&[value; 16]).expect("fixed proof id has exactly sixteen bytes")
}

fn pixel_rect(x: f32, y: f32, width: f32, height: f32) -> PixelRect {
    let left = x.floor().max(0.0) as u32;
    let top = y.floor().max(0.0) as u32;
    let right = (x + width).ceil().max(0.0) as u32;
    let bottom = (y + height).ceil().max(0.0) as u32;
    PixelRect {
        x: left,
        y: top,
        width: right.saturating_sub(left),
        height: bottom.saturating_sub(top),
    }
}

fn linear_rgba_to_srgb(linear: [f32; 4]) -> [u8; 4] {
    fn encode(value: f32) -> u8 {
        let value = value.clamp(0.0, 1.0);
        let srgb = if value <= 0.003_130_8 {
            value * 12.92
        } else {
            1.055 * value.powf(1.0 / 2.4) - 0.055
        };
        (srgb * 255.0).round() as u8
    }
    [
        encode(linear[0]),
        encode(linear[1]),
        encode(linear[2]),
        encode(linear[3]),
    ]
}

fn rgba_near(actual: [u8; 4], expected: [u8; 4], tolerance: u8) -> bool {
    actual
        .into_iter()
        .zip(expected)
        .all(|(actual, expected)| actual.abs_diff(expected) <= tolerance)
}

fn read_sample(
    name: String,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> Option<PixelSample> {
    read_pixel(x, y, width, height, pixels).map(|rgba| PixelSample { name, x, y, rgba })
}

fn read_pixel(x: u32, y: u32, width: u32, height: u32, pixels: &[u8]) -> Option<[u8; 4]> {
    if x >= width || y >= height || pixels.len() != (width * height * 4) as usize {
        return None;
    }
    let offset = ((y * width + x) * 4) as usize;
    pixels.get(offset..offset + 4)?.try_into().ok()
}

fn scan_glyph_bounds(
    rect: PixelRect,
    scan_top: u32,
    scan_bottom: u32,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> Option<PixelRect> {
    let left = rect.x.saturating_add(4);
    let top = scan_top.min(height);
    let right = (rect.x + rect.width).saturating_sub(4).min(width);
    let bottom = scan_bottom.min(height);
    let mut min_x = u32::MAX;
    let mut min_y = u32::MAX;
    let mut max_x = 0;
    let mut max_y = 0;
    let mut found = false;
    for y in top..bottom {
        for x in left..right {
            let pixel = read_pixel(x, y, width, height, pixels)?;
            let glyph_colored = pixel[0] >= 150 && pixel[1] >= 150 && pixel[2] >= 120;
            if pixel[3] >= 240 && glyph_colored {
                found = true;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    found.then_some(PixelRect {
        x: min_x,
        y: min_y,
        width: max_x - min_x + 1,
        height: max_y - min_y + 1,
    })
}

fn push_check(
    checks: &mut Vec<ContractCheck>,
    code: impl Into<String>,
    passed: bool,
    detail: impl Into<String>,
) {
    checks.push(ContractCheck {
        code: code.into(),
        passed,
        detail: detail.into(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tze_hud_scene::types::{NodeData, NodeLayout};

    fn sample(name: &str, x: u32, y: u32, rgba: [u8; 4]) -> PixelSample {
        PixelSample {
            name: name.to_string(),
            x,
            y,
            rgba,
        }
    }

    fn passing_evidence() -> ProofEvidence {
        let rects = [
            PixelRect {
                x: 300,
                y: 300,
                width: 880,
                height: 220,
            },
            PixelRect {
                x: 300,
                y: 560,
                width: 880,
                height: 220,
            },
            PixelRect {
                x: 300,
                y: 820,
                width: 880,
                height: 220,
            },
        ];
        let colors = [[89, 56, 56, 255], [56, 89, 56, 255], [56, 56, 97, 255]];
        ProofEvidence {
            reference_hardware: ReferenceHardware {
                tag: REFERENCE_TAG.to_string(),
                hostname: "reference-windows".to_string(),
                gpu: "NVIDIA GeForce RTX 3080".to_string(),
                gpu_driver: "32.0.15.9636".to_string(),
                os: "Windows 11 Pro 10.0.26200".to_string(),
                display_width: REFERENCE_WIDTH,
                display_height: REFERENCE_HEIGHT,
            },
            render_width: REFERENCE_WIDTH,
            render_height: REFERENCE_HEIGHT,
            pixel_buffer_len: (REFERENCE_WIDTH * REFERENCE_HEIGHT * 4) as usize,
            children: rects
                .into_iter()
                .zip(colors)
                .enumerate()
                .map(|(index, (rect, color))| ChildObservation {
                    name: format!("turn_{}", index + 1),
                    background_rect: rect,
                    expected_background_srgb: color,
                    background_sample: Some(sample(
                        &format!("turn_{}_background", index + 1),
                        rect.x + rect.width - 8,
                        rect.y + 8,
                        color,
                    )),
                    glyph_bounds: Some(PixelRect {
                        x: rect.x + 8,
                        y: rect.y + 8,
                        width: 300,
                        height: 120,
                    }),
                })
                .collect(),
            gap_samples: vec![
                sample("gap_1_2", 400, 540, CLEAR_SRGB),
                sample("gap_2_3", 400, 800, CLEAR_SRGB),
            ],
            sentinel_sample: Some(sample(
                "declared_y_sentinel",
                (TILE_X + CHILD_X + CHILD_WIDTH - 8.0) as u32,
                (TILE_Y + DECLARED_Y_SENTINEL + 8.0) as u32,
                CLEAR_SRGB,
            )),
        }
    }

    #[test]
    fn fixture_atomically_materializes_three_markdown_flow_children() {
        let fixture = build_fixture().expect("fixture must build");
        let parent = &fixture.scene.nodes[&fixture.parent_id];
        assert_eq!(parent.layout, NodeLayout::VerticalFlow);
        assert_eq!(parent.children, fixture.child_ids);
        for child_id in fixture.child_ids {
            let child = &fixture.scene.nodes[&child_id];
            assert!(matches!(child.data, NodeData::TextMarkdown(_)));
        }
        assert_eq!(
            fixture.scene.tiles[&fixture.tile_id].root_node,
            Some(fixture.parent_id)
        );
    }

    #[test]
    fn resolved_background_rects_do_not_overlap() {
        let fixture = build_fixture().expect("fixture must build");
        let rects = resolve_background_rects(&fixture).expect("flow must resolve");
        for pair in rects.windows(2) {
            assert!(
                pair[0].bottom() < pair[1].y,
                "flowed backgrounds overlap: {pair:?}"
            );
        }
    }

    #[test]
    fn complete_evidence_passes() {
        assert_eq!(
            evaluate_evidence(passing_evidence()).verdict,
            ProofVerdict::Pass
        );
    }

    #[test]
    fn wrong_reference_tag_fails_with_dedicated_check() {
        let mut evidence = passing_evidence();
        evidence.reference_hardware.tag = "not-tzehouse".to_string();
        let report = evaluate_evidence(evidence);
        let check = report
            .checks
            .iter()
            .find(|check| check.code == "reference_hardware_tag")
            .expect("report must expose the reference tag contract separately");
        assert!(!check.passed, "wrong reference tag must fail closed");
        assert_eq!(report.verdict, ProofVerdict::Fail);
    }

    #[test]
    fn unequal_display_and_render_surfaces_fail_with_dedicated_check() {
        let mut evidence = passing_evidence();
        evidence.render_width -= 1;
        evidence.pixel_buffer_len = (evidence.render_width * evidence.render_height * 4) as usize;
        let report = evaluate_evidence(evidence);
        let check = report
            .checks
            .iter()
            .find(|check| check.code == "display_surface_equal")
            .expect("report must expose display/surface equality separately");
        assert!(!check.passed, "unequal surfaces must fail closed");
        assert_eq!(report.verdict, ProofVerdict::Fail);
    }

    #[test]
    fn missing_reference_tag_fails_closed() {
        let mut evidence = passing_evidence();
        evidence.reference_hardware.tag.clear();
        assert_eq!(evaluate_evidence(evidence).verdict, ProofVerdict::Fail);
    }

    #[test]
    fn surface_mismatch_fails_closed() {
        let mut evidence = passing_evidence();
        evidence.render_width = 1920;
        assert_eq!(evaluate_evidence(evidence).verdict, ProofVerdict::Fail);
    }

    #[test]
    fn overlapping_bands_fail_closed() {
        let mut evidence = passing_evidence();
        evidence.children[1].background_rect.y = evidence.children[0].background_rect.bottom() - 1;
        assert_eq!(evaluate_evidence(evidence).verdict, ProofVerdict::Fail);
    }

    #[test]
    fn glyph_outside_its_background_fails_closed() {
        let mut evidence = passing_evidence();
        evidence.children[1].glyph_bounds.as_mut().unwrap().y = 500;
        assert_eq!(evaluate_evidence(evidence).verdict, ProofVerdict::Fail);
    }

    #[test]
    fn absent_samples_fail_closed() {
        let mut evidence = passing_evidence();
        evidence.children[2].background_sample = None;
        evidence.gap_samples.clear();
        evidence.sentinel_sample = None;
        assert_eq!(evaluate_evidence(evidence).verdict, ProofVerdict::Fail);
    }

    #[test]
    fn pixel_observer_extracts_background_gap_sentinel_and_glyph_evidence() {
        let fixture = build_fixture().expect("fixture must build");
        let rects = resolve_background_rects(&fixture).expect("flow must resolve");
        let mut pixels = vec![0; (REFERENCE_WIDTH * REFERENCE_HEIGHT * 4) as usize];
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.copy_from_slice(&CLEAR_SRGB);
        }
        for (rect, color) in rects.into_iter().zip(expected_background_srgb()) {
            paint_rect(&mut pixels, REFERENCE_WIDTH, rect, color);
            paint_rect(
                &mut pixels,
                REFERENCE_WIDTH,
                PixelRect {
                    x: rect.x + 12,
                    y: rect.y + 12,
                    width: 24,
                    height: 16,
                },
                [240, 240, 240, 255],
            );
        }
        let evidence = observe_pixels(
            passing_evidence().reference_hardware,
            &fixture,
            REFERENCE_WIDTH,
            REFERENCE_HEIGHT,
            &pixels,
        )
        .expect("pixel observation must succeed");
        assert_eq!(evaluate_evidence(evidence).verdict, ProofVerdict::Pass);
    }

    #[test]
    fn pixel_observer_reports_glyph_pixels_outside_their_background() {
        let fixture = build_fixture().expect("fixture must build");
        let rects = resolve_background_rects(&fixture).expect("flow must resolve");
        let mut pixels = vec![0; (REFERENCE_WIDTH * REFERENCE_HEIGHT * 4) as usize];
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.copy_from_slice(&CLEAR_SRGB);
        }
        for (rect, color) in rects.into_iter().zip(expected_background_srgb()) {
            paint_rect(&mut pixels, REFERENCE_WIDTH, rect, color);
            paint_rect(
                &mut pixels,
                REFERENCE_WIDTH,
                PixelRect {
                    x: rect.x + 12,
                    y: rect.y + 12,
                    width: 24,
                    height: 16,
                },
                [240, 240, 240, 255],
            );
        }
        paint_rect(
            &mut pixels,
            REFERENCE_WIDTH,
            PixelRect {
                x: rects[0].x + 12,
                y: rects[0].bottom() + 1,
                width: 4,
                height: 2,
            },
            [240, 240, 240, 255],
        );

        let evidence = observe_pixels(
            passing_evidence().reference_hardware,
            &fixture,
            REFERENCE_WIDTH,
            REFERENCE_HEIGHT,
            &pixels,
        )
        .expect("pixel observation must succeed");
        let report = evaluate_evidence(evidence);
        let alignment = report
            .checks
            .iter()
            .find(|check| check.code == "assistant_markdown_background_glyph_y_alignment")
            .expect("first child alignment check must be present");
        assert!(!alignment.passed, "out-of-band glyph must fail alignment");
        assert_eq!(report.verdict, ProofVerdict::Fail);
    }

    fn paint_rect(pixels: &mut [u8], width: u32, rect: PixelRect, color: [u8; 4]) {
        for y in rect.y..rect.bottom() {
            for x in rect.x..rect.x + rect.width {
                let offset = ((y * width + x) * 4) as usize;
                pixels[offset..offset + 4].copy_from_slice(&color);
            }
        }
    }
}
