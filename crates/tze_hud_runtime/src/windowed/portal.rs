use tze_hud_input::{
    DragEventOutcome, InputProcessor, PointerEvent, PointerEventKind, PortalRect,
    PortalResizeState, PortalWindowTokens, ResizeBounds, ResizeEdge, ResizeOutcome,
    apply_hotkey_resize, hit_affordance,
};
use tze_hud_scene::HitResult;
use tze_hud_scene::types::{DragHandleElementKind, ZoneInteractionKind};

use super::input_dispatch::{deliver_composer_batch, dispatch_portal_geometry_event};
use super::keyboard::ComposerDeliveryContext;
use super::lifecycle::{INTERACTION_LOCK_BUDGET, spin_acquire};
use super::{WindowedConfig, WinitApp};

pub(super) fn build_portal_projection_driver(
    config: &WindowedConfig,
) -> Result<
    crate::portal_projection_driver::InProcessPortalDriver,
    tze_hud_projection::ProjectionContractError,
> {
    let mut driver = crate::portal_projection_driver::InProcessPortalDriver::new();
    if let Some(operator_authority) = config.projection_operator_authority.as_deref() {
        driver
            .authority_mut()
            .set_operator_authority(operator_authority)?;
        tracing::info!("portal projection operator authority configured");
    }
    Ok(driver)
}

// ── Drag-to-move: data carried out of the scene-lock for post-lock work ──────

/// Payload returned by [`apply_drag_handle_pointer_event`] when a drag is
/// completed and the geometry must be persisted outside the scene lock.
pub(super) struct DragReleasedData {
    /// Scene-level ID of the element that was dragged.
    element_id: tze_hud_scene::SceneId,
    /// Final snapped+clamped top-left X in display pixels.
    final_x: f32,
    /// Final snapped+clamped top-left Y in display pixels.
    final_y: f32,
    /// Element width in display pixels (unchanged during drag).
    width: f32,
    /// Element height in display pixels (unchanged during drag).
    height: f32,
    /// Display width at time of release, used for `GeometryPolicy::Relative` normalisation.
    display_width: f32,
    /// Display height at time of release.
    display_height: f32,
    /// Agent namespace that owns the tile, used for `ElementRepositionedEvent` routing.
    namespace: String,
}

// ── Portal-resize pointer: geometry carried out of scene lock ────────────────

/// Per-member geometry update produced by a whole-portal resize step.
///
/// A portal is composed of several constituent surfaces; a single resize step
/// scales all of them, so the outcome carries one snapshot per member for the
/// caller to broadcast.
pub(super) struct PortalMemberGeometry {
    /// Constituent-surface tile whose bounds were updated.
    pub(super) tile_id: tze_hud_scene::SceneId,
    /// Geometry snapshot carrying this member's scaled rect.
    pub(super) snapshot: tze_hud_input::GeometrySnapshot,
}

/// Outcome of a pointer-driven portal resize step, carried out of the scene
/// lock so that [`dispatch_portal_geometry_event`] can be called without
/// holding locks (fire-and-forget gRPC send).
pub(super) struct PortalResizePointerOutcome {
    /// Per-member geometry updates to broadcast (includes the anchor/frame,
    /// whose snapshot carries the whole-portal rect).
    pub(super) members: Vec<PortalMemberGeometry>,
    /// Display width at the time of the event (for geometry normalisation).
    pub(super) display_w: f32,
    /// Display height at the time of the event.
    pub(super) display_h: f32,
}

// The input processor, pointer event, hit result, scene, and display dimensions
// are all separate concerns that cannot be bundled into a context struct without
// creating an artificial abstraction; the argument count is genuinely necessary.
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_drag_handle_pointer_event(
    input_processor: &mut InputProcessor,
    pointer_event: &PointerEvent,
    result_hit: &HitResult,
    scene: &mut tze_hud_scene::graph::SceneGraph,
    display_width: f32,
    display_height: f32,
) -> Option<DragReleasedData> {
    let device_id = pointer_event.device_id;

    // Determine which drag handle (if any) was hit on this event.
    let hit_drag_info: Option<(&str, tze_hud_scene::SceneId, DragHandleElementKind)> =
        match result_hit {
            HitResult::ZoneInteraction {
                interaction_id,
                kind:
                    ZoneInteractionKind::DragHandle {
                        element_id,
                        element_kind,
                    },
                ..
            } => Some((interaction_id.as_str(), *element_id, *element_kind)),
            _ => None,
        };

    // On PointerDown on a drag handle, start accumulating.
    if pointer_event.kind == PointerEventKind::Down {
        if let Some((interaction_id, element_id, element_kind)) = hit_drag_info {
            let element_bounds = scene
                .tiles
                .get(&element_id)
                .map(|t| t.bounds)
                .unwrap_or_else(|| tze_hud_scene::Rect::new(0.0, 0.0, 0.0, 0.0));
            let outcome = input_processor.process_drag_handle_pointer(
                pointer_event,
                interaction_id,
                element_id,
                element_kind,
                element_bounds,
                display_width,
                display_height,
            );
            tracing::trace!(
                element_id = %element_id,
                x = pointer_event.x,
                y = pointer_event.y,
                ?outcome,
                "drag-handle: PointerDown accumulating"
            );
        }
        return None;
    }

    // On PointerMove or PointerUp, check for an in-flight drag on this device.
    let drag_info = input_processor
        .drag_states
        .get(&device_id)
        .map(|s| (s.interaction_id.clone(), s.element_id, s.element_kind));

    let Some((interaction_id, element_id, element_kind)) = drag_info else {
        // No drag in progress for this device — nothing to do.
        return None;
    };

    // Snapshot element bounds; element_id is the tile being dragged.
    let element_bounds = scene
        .tiles
        .get(&element_id)
        .map(|t| t.bounds)
        .unwrap_or_else(|| tze_hud_scene::Rect::new(0.0, 0.0, 0.0, 0.0));

    let outcome = input_processor.process_drag_handle_pointer(
        pointer_event,
        &interaction_id,
        element_id,
        element_kind,
        element_bounds,
        display_width,
        display_height,
    );

    match outcome {
        DragEventOutcome::Idle | DragEventOutcome::Accumulating { .. } => {
            // Nothing to do locally.
            None
        }
        DragEventOutcome::Activated { element_id, .. } => {
            tracing::debug!(
                element_id = %element_id,
                "drag-handle: drag activated — element follows pointer"
            );
            None
        }
        DragEventOutcome::Cancelled => {
            tracing::trace!(
                element_id = %element_id,
                "drag-handle: drag cancelled (tap or moved beyond tolerance)"
            );
            None
        }
        DragEventOutcome::Moved {
            element_id: eid,
            new_x,
            new_y,
            ..
        } => {
            // Update tile bounds directly (chrome-layer bypass — no lease check).
            // For a text-stream portal the drag moves the WHOLE portal as one
            // unit (hud-lyqun): translate every group member by the same delta so
            // the constituents never fracture. Falls back to a single-tile move
            // for non-portal drags.
            let old = scene.tiles.get(&eid).map(|t| t.bounds);
            if let Some(old) = old {
                let dx = new_x - old.x;
                let dy = new_y - old.y;
                if !translate_portal_group_on_drag(scene, eid, dx, dy) {
                    let moved = if let Some(tile) = scene.tiles.get_mut(&eid) {
                        tile.bounds.x = new_x;
                        tile.bounds.y = new_y;
                        true
                    } else {
                        false
                    };
                    if moved {
                        // Position-only single-tile move: geometry epoch, not
                        // version — same reasoning as the group path (hud-uyhpn).
                        scene.bump_geometry_epoch();
                    }
                }
                tracing::trace!(
                    element_id = %eid,
                    old_x = old.x,
                    old_y = old.y,
                    new_x,
                    new_y,
                    "drag-handle: tile moved"
                );
            }
            None
        }
        DragEventOutcome::Released {
            element_id: eid,
            final_x,
            final_y,
            element_kind: _,
        } => {
            let (old_x, old_y, width, height) = scene
                .tiles
                .get(&eid)
                .map(|t| (t.bounds.x, t.bounds.y, t.bounds.width, t.bounds.height))
                .unwrap_or((0.0, 0.0, 0.0, 0.0));

            // Apply the final position. As with move, a portal drag relocates the
            // whole group coherently (hud-lyqun); non-portal drags move the single
            // tile.
            let dx = final_x - old_x;
            let dy = final_y - old_y;
            if !translate_portal_group_on_drag(scene, eid, dx, dy) {
                let moved = if let Some(tile) = scene.tiles.get_mut(&eid) {
                    tile.bounds.x = final_x;
                    tile.bounds.y = final_y;
                    true
                } else {
                    false
                };
                if moved {
                    // Position-only single-tile move: geometry epoch, not
                    // version — same reasoning as the group path (hud-uyhpn).
                    scene.bump_geometry_epoch();
                }
            }

            let namespace = scene
                .tiles
                .get(&eid)
                .map(|t| t.namespace.clone())
                .unwrap_or_default();

            tracing::debug!(
                element_id = %eid,
                final_x,
                final_y,
                width,
                height,
                "drag-handle: drag released — persisting geometry"
            );

            // Return data the caller will use to persist after releasing locks.
            Some(DragReleasedData {
                element_id: eid,
                final_x,
                final_y,
                width,
                height,
                display_width,
                display_height,
                namespace,
            })
        }
    }
}

/// Compute the maximum resize dimensions for a portal tile, combining the
/// display boundary clamp with the tile's lease-granted spatial budget.
///
/// The display boundary (`display_w`, `display_h`) is always the hard outer
/// clamp.  When the tile's lease carries a non-zero `max_tile_width_px` /
/// `max_tile_height_px`, whichever is smaller wins (most-restrictive policy).
///
/// `lease_max_width_px` and `lease_max_height_px` should be sourced from
/// the `Lease.resource_budget` in `scene.leases` (the authoritative location
/// for spatial budget limits).  Pass `0.0` for each to indicate unconstrained.
///
/// A lease budget value of `0.0` means unconstrained: only the display boundary
/// applies.
///
/// `min_*` values are from the portal tokens and serve as the floor (a portal
/// can always be grown to at least the minimum size even if the lease limit is
/// somehow smaller — the lease limit is not allowed to shrink a portal below the
/// token minimum, per §6b design intent).
///
/// ## Why `display_w`, not `display_w − tile.x`
///
/// `PortalRect::clamped` enforces that the portal fits on-screen by adjusting
/// both the size **and** the origin together: `x.clamp(0, display_w − w)`.
/// If the portal is at x=500 on a 1000px display, it can still grow to
/// 1000px wide — `clamped` will shift the origin left to x=0 so the right
/// edge lands exactly at the screen edge.  Using `display_w − tile.x` as the
/// max would prevent this shift and cap the portal at half the display width,
/// which is incorrect.
fn compute_portal_max_dims(
    lease_max_width_px: f32,
    lease_max_height_px: f32,
    display_w: f32,
    display_h: f32,
    min_width_px: f32,
    min_height_px: f32,
) -> (f32, f32) {
    // Display boundary: the portal cannot be wider/taller than the display.
    // The origin is clamped separately by PortalRect::clamped, so we do not
    // subtract tile.x / tile.y here (that was the pre-fix incorrect bound).
    let display_max_w = display_w.max(min_width_px);
    let display_max_h = display_h.max(min_height_px);

    // Lease budget: intersect with display boundary (most restrictive wins).
    // A lease value of 0.0 means unconstrained; skip the intersection.
    let max_w = if lease_max_width_px > 0.0 {
        display_max_w.min(lease_max_width_px).max(min_width_px)
    } else {
        display_max_w
    };
    let max_h = if lease_max_height_px > 0.0 {
        display_max_h.min(lease_max_height_px).max(min_height_px)
    } else {
        display_max_h
    };
    (max_w, max_h)
}

// ── Portal-group resolution (hud-fb3en) ──────────────────────────────────────

/// A resolved portal group: the constituent scene tiles that move and resize as
/// one coherent unit.
///
/// A text-stream portal is composed of several independent scene tiles that
/// share a single lease (frame, transcript/output pane, composer/input pane,
/// minimized icon, capture backstop, drag shield). The runtime has no explicit
/// portal-group field on [`tze_hud_scene::types::Tile`], so the group is resolved
/// structurally: the frame is the largest-area lease member (the portal-sized
/// frame / capture backstop), and its top-left is the fixed anchor for
/// grow/shrink (matching the top-left anchor semantics from PR #981). Members
/// are the lease's tiles whose bounds lie within the frame rect.
///
/// This deliberately EXCLUDES the drag shield, which the client parks in a far
/// display corner and which must not scale with the portal — mirroring the
/// client-side `portal_bounds_mutations` layout, which omits the drag shield
/// from the visible-portal geometry.
///
/// A single-tile lease resolves to a one-member group (the tile is its own
/// anchor), preserving the pre-fix single-surface behavior.
pub(super) struct PortalGroup {
    /// The frame/anchor tile — its bounds are the whole-portal rect.
    pub(super) anchor_tile_id: tze_hud_scene::SceneId,
    /// Whole-portal rect (anchor frame bounds) at resolution time.
    pub(super) portal_rect: PortalRect,
    /// All member tile ids that scale with the portal (includes the anchor).
    pub(super) member_ids: Vec<tze_hud_scene::SceneId>,
    /// Stable per-portal hash (from the anchor tile) for `PortalResizeState`.
    pub(super) portal_id_hash: u64,
}

/// Return true when `inner` lies within `outer`, allowing a small epsilon so
/// sub-pixel rounding at pane edges does not drop a legitimate member.
fn rect_contains(outer: &tze_hud_scene::Rect, inner: &tze_hud_scene::Rect, eps: f32) -> bool {
    inner.x >= outer.x - eps
        && inner.y >= outer.y - eps
        && inner.x + inner.width <= outer.x + outer.width + eps
        && inner.y + inner.height <= outer.y + outer.height + eps
}

/// Resolve the whole-portal group that owns `member_tile_id`.
///
/// Works from ANY member tile — a focused pane on the hotkey / pointer-down
/// path, or the stored anchor tile on the pointer-move / pointer-up path: it
/// looks up the shared lease, picks the largest-area lease member as the
/// frame/anchor, then collects the lease members spatially contained within the
/// frame rect (plus the seed tile itself, defensively).
///
/// Returns `None` if the tile does not exist.
pub(super) fn resolve_portal_group(
    scene: &tze_hud_scene::graph::SceneGraph,
    member_tile_id: tze_hud_scene::SceneId,
) -> Option<PortalGroup> {
    let seed = scene.tiles.get(&member_tile_id)?;
    let lease_id = seed.lease_id;

    // Largest-area lease member is the frame/anchor (ties → lowest id). Delegated
    // to the scene-crate resolver so the frame pick is a single source of truth
    // shared with the compositor's header-band drag handle (hud-643dv).
    let anchor_id = scene.portal_anchor_tile(member_tile_id)?;
    let anchor_bounds = scene.tiles.get(&anchor_id)?.bounds;

    // Members = lease tiles spatially within the frame rect. The far-corner
    // drag shield falls outside the frame and is excluded. The seed and anchor
    // are always included.
    let eps = 1.0_f32;
    let mut member_ids: Vec<tze_hud_scene::SceneId> = scene
        .tiles
        .iter()
        .filter(|(id, tile)| {
            tile.lease_id == lease_id
                && (**id == member_tile_id
                    || **id == anchor_id
                    || rect_contains(&anchor_bounds, &tile.bounds, eps))
        })
        .map(|(id, _)| *id)
        .collect();
    member_ids.sort();

    let portal_rect = PortalRect {
        x: anchor_bounds.x,
        y: anchor_bounds.y,
        width: anchor_bounds.width,
        height: anchor_bounds.height,
    };
    let portal_id_hash = anchor_id.as_uuid().as_u128() as u64;

    Some(PortalGroup {
        anchor_tile_id: anchor_id,
        portal_rect,
        member_ids,
        portal_id_hash,
    })
}

/// Compute new bounds for every group member under a top-left-anchored uniform
/// scale from `old_rect` → `new_rect`.
///
/// The portal's top-left (`new_rect` origin) is the fixed anchor. Each member's
/// offset from the anchor and its size scale by the per-axis ratio, preserving
/// relative layout. Returns `(tile_id, new_rect)` pairs for members that still
/// exist in the scene.
fn scale_portal_members(
    scene: &tze_hud_scene::graph::SceneGraph,
    group: &PortalGroup,
    old_rect: PortalRect,
    new_rect: PortalRect,
) -> Vec<(tze_hud_scene::SceneId, tze_hud_scene::Rect)> {
    let r_w = if old_rect.width > 0.0 {
        new_rect.width / old_rect.width
    } else {
        1.0
    };
    let r_h = if old_rect.height > 0.0 {
        new_rect.height / old_rect.height
    } else {
        1.0
    };
    let mut updates = Vec::with_capacity(group.member_ids.len());
    for &tile_id in &group.member_ids {
        let Some(tile) = scene.tiles.get(&tile_id) else {
            continue;
        };
        // The anchor/frame IS the whole-portal rect: assign it exactly rather
        // than round-tripping through the scale ratio. This keeps a single-tile
        // portal bit-identical to the new rect (no float drift), which matters
        // for the `scene.version` size-change guard driving the compositor's
        // truncation-cache re-prime cadence.
        let new_bounds = if tile_id == group.anchor_tile_id {
            tze_hud_scene::Rect::new(new_rect.x, new_rect.y, new_rect.width, new_rect.height)
        } else {
            let b = tile.bounds;
            tze_hud_scene::Rect::new(
                new_rect.x + (b.x - old_rect.x) * r_w,
                new_rect.y + (b.y - old_rect.y) * r_h,
                b.width * r_w,
                b.height * r_h,
            )
        };
        updates.push((tile_id, new_bounds));
    }
    updates
}

/// Scale a tile's node tree in place by the per-axis ratio `(r_w, r_h)`.
///
/// Node bounds are **tile-local** (origin + extent relative to the tile's
/// top-left) and independent of `Tile::bounds` — nothing derives them from the
/// tile size. So when the viewer resizes a tile, the node tree must scale by the
/// same ratio or content keeps laying out to the old geometry. In particular the
/// compositor wraps `TextMarkdownNode` text to `node.bounds.width` (the layout
/// column in `TextItem::from_text_markdown_cached` / `from_text_markdown_node`),
/// so without this the transcript/composer text keeps its attach-time wrap width
/// and does not re-flow to the resized pane (hud-rpmwt). Font size is untouched —
/// text reflows to the new width, it does not zoom.
fn scale_tile_node_tree(
    scene: &mut tze_hud_scene::graph::SceneGraph,
    tile_id: tze_hud_scene::SceneId,
    r_w: f32,
    r_h: f32,
) {
    let Some(root) = scene.tiles.get(&tile_id).and_then(|t| t.root_node) else {
        return;
    };
    // Collect the subtree ids with an immutable walk first, then mutate — a
    // tile's node tree is a small DAG-free tree (≤ MAX_NODES_PER_TILE) so this
    // avoids aliasing the node store while descending `children`.
    let mut stack = vec![root];
    let mut ids = Vec::new();
    while let Some(id) = stack.pop() {
        let Some(node) = scene.nodes.get(&id) else {
            continue;
        };
        ids.push(id);
        stack.extend(node.children.iter().copied());
    }
    for id in ids {
        if let Some(node) = scene.nodes.get_mut(&id) {
            let b = node.data.bounds_mut();
            b.x *= r_w;
            b.y *= r_h;
            b.width *= r_w;
            b.height *= r_h;
        }
    }
}

/// Apply a resolved whole-portal resize to the scene: write each member's scaled
/// bounds, bump the scene version once if any geometry changed, and build the
/// per-member geometry snapshots to broadcast.
///
/// `primary` is the whole-portal geometry snapshot produced by the resize state
/// machine (its `rect` is the new anchor/frame rect). Each returned member
/// snapshot carries the same sequence / gesture flag but with that member's own
/// scaled rect, so per-member `ElementRepositionedEvent`s report correct
/// geometry.
fn commit_portal_group_resize(
    scene: &mut tze_hud_scene::graph::SceneGraph,
    group: &PortalGroup,
    old_rect: PortalRect,
    primary: tze_hud_input::GeometrySnapshot,
) -> Vec<PortalMemberGeometry> {
    let new_rect = primary.rect;
    let updates = scale_portal_members(scene, group, old_rect, new_rect);
    let mut any_changed = false;
    let mut members = Vec::with_capacity(updates.len());
    for (tile_id, new_bounds) in updates {
        let old_tile_bounds = scene.tiles.get(&tile_id).map(|t| t.bounds);
        if let Some(tile) = scene.tiles.get_mut(&tile_id) {
            if tile.bounds.width != new_bounds.width || tile.bounds.height != new_bounds.height {
                any_changed = true;
            }
            tile.bounds = new_bounds;
        }
        // Scale the tile's node tree in lock-step with the tile so tile-local
        // node geometry — and the text wrap width the compositor reads from
        // `TextMarkdownNode::bounds.width` — re-resolves to the new pane. Use
        // each tile's OWN size ratio (not the whole-portal ratio) so the nodes
        // track exactly the tile they live in. Without this the frame scales but
        // the transcript/composer text stays wrapped at the old width (hud-rpmwt).
        if let Some(old) = old_tile_bounds {
            let node_r_w = if old.width > 0.0 {
                new_bounds.width / old.width
            } else {
                1.0
            };
            let node_r_h = if old.height > 0.0 {
                new_bounds.height / old.height
            } else {
                1.0
            };
            if node_r_w != 1.0 || node_r_h != 1.0 {
                scale_tile_node_tree(scene, tile_id, node_r_w, node_r_h);
            }
        }
        // The viewer now owns this member's geometry: take authority so an
        // adapter republishing its stale client-side layout on the next content
        // publish or drag cannot stomp the member back and fracture the portal
        // group (hud-lyqun). Viewer-driven resize writes `tile.bounds` directly
        // (above), so this only gates adapter-originated `UpdateTileBounds`.
        scene.lock_viewer_geometry(tile_id);
        let snapshot = tze_hud_input::GeometrySnapshot {
            rect: PortalRect {
                x: new_bounds.x,
                y: new_bounds.y,
                width: new_bounds.width,
                height: new_bounds.height,
            },
            ..primary
        };
        members.push(PortalMemberGeometry { tile_id, snapshot });
    }
    // The scene version drives the compositor's truncation-cache re-prime at the
    // new (intermediate) geometry; bump once per whole-portal step, guarded on a
    // real size change so a clamped-at-boundary press does not churn the cache.
    if any_changed {
        scene.version += 1;
    }
    members
}

/// Translate a whole portal group by `(dx, dy)` when the viewer drags one of its
/// constituent surfaces, preserving the group's relative layout and taking
/// viewer geometry authority over every member (hud-lyqun).
///
/// A text-stream portal is N independent tiles (frame + scrollable panes + drag
/// shield). Before this, the chrome drag handler moved only the single grabbed
/// tile, so dragging a portal fractured it (and, after a prior whole-portal
/// resize, the grabbed surface floated away from the rest). Here the dragged
/// tile's motion delta is applied to every group member so the portal moves as
/// one coherent unit — the completion of the whole-unit gesture work started for
/// resize in PR #984.
///
/// Gated to real portals: the resolved group must have more than one member and
/// contain at least one scrollable constituent surface. A plain single tile /
/// widget / zone drag resolves to a lone or non-portal group and is left to the
/// single-element move path (returns `false`). The far-corner drag shield is
/// excluded by `resolve_portal_group` and stays parked.
///
/// Returns `true` when a whole-portal translate was applied.
fn translate_portal_group_on_drag(
    scene: &mut tze_hud_scene::graph::SceneGraph,
    dragged_tile_id: tze_hud_scene::SceneId,
    dx: f32,
    dy: f32,
) -> bool {
    let Some(group) = resolve_portal_group(scene, dragged_tile_id) else {
        return false;
    };
    let is_portal = group.member_ids.len() > 1
        && group
            .member_ids
            .iter()
            .any(|id| scene.tile_scroll_config(*id).is_some());
    if !is_portal {
        return false;
    }

    for &tile_id in &group.member_ids {
        if let Some(tile) = scene.tiles.get_mut(&tile_id) {
            tile.bounds.x += dx;
            tile.bounds.y += dy;
        }
        // Viewer geometry authority — same as the resize path — so an adapter
        // republish cannot pull a member back to its stale layout.
        scene.lock_viewer_geometry(tile_id);
    }
    // Position-only mutation: bump the geometry epoch (re-arms the present-gate
    // so every member paints at its new position this frame) but NOT
    // scene.version — a translation changes no content and no size, so the
    // compositor's version-gated markdown/truncation caches must NOT re-prime.
    // Bumping version here forced a full per-frame re-hash/re-shape and made the
    // live drag low-fps / flickery (hud-uyhpn). #986 group coherence and #989
    // resize reflow both key off size and are unaffected.
    scene.bump_geometry_epoch();
    true
}

/// Pointer-driven portal resize state machine step.
///
/// Called from [`WinitApp::enqueue_pointer_event`] while the scene lock is held.
/// Drives `PortalResizeState` through the pointer-down / pointer-move / pointer-up
/// lifecycle for resize affordances (§6b.1 pointer resize scenario).
///
/// On **PointerDown**: performs a hit-test against the focused portal's resize
/// affordances.  If the pointer lands on an affordance, starts the gesture and
/// returns a [`PortalResizePointerOutcome`] with the initial snapshot so the
/// caller can apply local bounds and broadcast the geometry event.
///
/// On **PointerMove**: if a gesture is active for `device_id`, computes the new
/// intermediate rect and applies it to the scene immediately (local-first).
///
/// On **PointerUp**: ends the gesture, applies the final clamped rect, and
/// returns an outcome the caller must broadcast.
///
/// Returns `None` when there is nothing to do (no portal focused, pointer
/// outside affordances, no gesture active, or lock contention).
///
/// ## Gesture authority
///
/// The gesture epoch is advanced inside `PortalResizeState::on_pointer_down` (to
/// odd, blocking adapter publishes) and `on_pointer_up` (back to even, releasing
/// the block after the gesture ends).  The caller MUST propagate snapshots with
/// `gesture_active = true` to prevent adapter geometry from stomping the in-flight
/// resize.
// The scene, pointer, focus, resize-state, display-dims, and token arguments
// are all necessary and unrelated — merging them into a struct would create an
// ad-hoc context object with no benefit.
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_portal_resize_pointer_event(
    pointer_event: &PointerEvent,
    portal_resize_states: &mut std::collections::HashMap<tze_hud_scene::SceneId, PortalResizeState>,
    active_tab: Option<tze_hud_scene::SceneId>,
    focus_manager: &tze_hud_input::FocusManager,
    scene: &mut tze_hud_scene::graph::SceneGraph,
    display_w: f32,
    display_h: f32,
    tokens: PortalWindowTokens,
) -> Option<PortalResizePointerOutcome> {
    let device_id = pointer_event.device_id;
    let x = pointer_event.x;
    let y = pointer_event.y;

    // Resolve the focused portal tile for this tab (only portal tiles accept
    // pointer-affordance resize, same gate as hotkey resize).
    let tab_id = active_tab?;
    let focused_tile_id = focus_manager.current_owner(tab_id).tile_id()?;

    // Build the clamping bounds for a whole-portal rect owned by `anchor_lease`.
    let resize_bounds_for_lease =
        |scene: &tze_hud_scene::graph::SceneGraph, anchor_lease: tze_hud_scene::SceneId| {
            let (lease_max_w, lease_max_h) = scene
                .leases
                .get(&anchor_lease)
                .map(|l| {
                    (
                        l.spatial_budget.max_tile_width_px,
                        l.spatial_budget.max_tile_height_px,
                    )
                })
                .unwrap_or((0.0, 0.0));
            let (max_width_px, max_height_px) = compute_portal_max_dims(
                lease_max_w,
                lease_max_h,
                display_w,
                display_h,
                tokens.min_width_px,
                tokens.min_height_px,
            );
            ResizeBounds {
                tokens,
                max_width_px,
                max_height_px,
                display_w,
                display_h,
            }
        };

    match pointer_event.kind {
        PointerEventKind::Down => {
            // Only scrollable (portal) tiles accept resize affordances — same
            // gate as hotkey resize; keeps the drag shield / frame from acting
            // as the resize target.
            scene.tile_scroll_config(focused_tile_id)?;
            // Resolve the whole portal; the affordance strip lives on the frame.
            let group = resolve_portal_group(scene, focused_tile_id)?;
            let old_rect = group.portal_rect;
            let edge: ResizeEdge = hit_affordance(x, y, &old_rect, tokens.affordance_px)?;

            // Spatial budget from the anchor/frame's lease (the whole-portal
            // rect clamps against the portal's lease budget).
            let anchor_lease = scene.tiles.get(&group.anchor_tile_id)?.lease_id;
            let resize_bounds = resize_bounds_for_lease(scene, anchor_lease);

            let resize_state = portal_resize_states
                .entry(group.anchor_tile_id)
                .or_insert_with(|| PortalResizeState::new(group.portal_id_hash));

            let outcome =
                resize_state.on_pointer_down(device_id, edge, x, y, old_rect, &resize_bounds);
            let snapshot = match outcome {
                ResizeOutcome::GestureStarted { snapshot } => snapshot,
                _ => return None,
            };
            let gesture_epoch = resize_state.current_gesture_epoch();

            // Apply initial (clamped) rect to the whole portal — local-first.
            let members = commit_portal_group_resize(scene, &group, old_rect, snapshot);

            tracing::debug!(
                anchor_tile_id = ?group.anchor_tile_id,
                members = members.len(),
                ?edge,
                x,
                y,
                gesture_epoch,
                "portal resize: pointer-down on affordance — whole-portal gesture started"
            );

            Some(PortalResizePointerOutcome {
                members,
                display_w,
                display_h,
            })
        }

        PointerEventKind::Move => {
            let mut active_gesture = None;
            for (&anchor_id, resize_state) in portal_resize_states.iter_mut() {
                if !resize_state.gesture_active() {
                    continue;
                }
                let Some(anchor_lease) = scene.tiles.get(&anchor_id).map(|t| t.lease_id) else {
                    continue;
                };
                let resize_bounds = resize_bounds_for_lease(scene, anchor_lease);
                if let ResizeOutcome::GestureUpdate { snapshot } =
                    resize_state.on_pointer_move(device_id, x, y, &resize_bounds)
                {
                    active_gesture = Some((anchor_id, snapshot));
                    break;
                }
            }

            let (anchor_id, snapshot) = active_gesture?;
            let group = resolve_portal_group(scene, anchor_id)?;
            let old_rect = group.portal_rect;

            // Apply the updated whole-portal rect immediately (local-first).
            let members = commit_portal_group_resize(scene, &group, old_rect, snapshot);

            tracing::trace!(
                anchor_tile_id = ?group.anchor_tile_id,
                members = members.len(),
                x,
                y,
                new_w = snapshot.rect.width,
                new_h = snapshot.rect.height,
                "portal resize: pointer-move — whole-portal bounds updated locally"
            );

            Some(PortalResizePointerOutcome {
                members,
                display_w,
                display_h,
            })
        }

        PointerEventKind::Up => {
            let mut active_gesture = None;
            for (&anchor_id, resize_state) in portal_resize_states.iter_mut() {
                if !resize_state.gesture_active() {
                    continue;
                }
                let Some(anchor_lease) = scene.tiles.get(&anchor_id).map(|t| t.lease_id) else {
                    continue;
                };
                let resize_bounds = resize_bounds_for_lease(scene, anchor_lease);
                if let ResizeOutcome::GestureEnded { snapshot } =
                    resize_state.on_pointer_up(device_id, x, y, &resize_bounds)
                {
                    active_gesture =
                        Some((anchor_id, snapshot, resize_state.current_gesture_epoch()));
                    break;
                }
            }

            let (anchor_id, snapshot, gesture_epoch) = active_gesture?;
            let group = resolve_portal_group(scene, anchor_id)?;
            let old_rect = group.portal_rect;

            // Apply final clamped whole-portal rect (local-first).
            let members = commit_portal_group_resize(scene, &group, old_rect, snapshot);

            tracing::debug!(
                anchor_tile_id = ?group.anchor_tile_id,
                members = members.len(),
                x,
                y,
                final_w = snapshot.rect.width,
                final_h = snapshot.rect.height,
                gesture_epoch,
                "portal resize: pointer-up — whole-portal gesture ended, final bounds applied"
            );

            Some(PortalResizePointerOutcome {
                members,
                display_w,
                display_h,
            })
        }
    }
}

impl WinitApp {
    pub(super) fn route_and_deliver_composer_batch(
        &mut self,
        context: ComposerDeliveryContext,
        batch: tze_hud_input::DraftNotificationBatch,
    ) {
        self.route_portal_composer_batch(context.tile_id, &batch);
        deliver_composer_batch(
            &self.state.input_event_tx,
            context.namespace,
            &context.node_id_bytes,
            batch,
        );
    }

    /// Route submitted focused-portal composer text into the in-process
    /// projection authority before the legacy namespace broadcast is emitted.
    fn route_portal_composer_batch(
        &mut self,
        tile_id: tze_hud_scene::SceneId,
        batch: &tze_hud_input::DraftNotificationBatch,
    ) {
        let submitted_at_wall_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros()
            .min(u128::from(u64::MAX)) as u64;

        if let Some(feedback) = self
            .state
            .portal_projection_driver
            .submit_composer_batch_for_tile(
                tile_id,
                batch,
                submitted_at_wall_us.max(1),
                None,
                tze_hud_projection::ContentClassification::Private,
            )
        {
            tracing::debug!(
                tile_id = ?tile_id,
                pending_input_count = feedback.pending_input_count,
                feedback_state = ?feedback.feedback_state,
                "composer: routed portal submission to projection authority"
            );
        }
    }

    /// Persist the geometry override for a completed drag and broadcast an
    /// `ElementRepositionedEvent`.
    ///
    /// Called after all scene locks are released to avoid holding locks during
    /// disk I/O (element store atomic write + fsync).
    pub(super) fn persist_drag_release(&mut self, released: DragReleasedData) {
        use tze_hud_input::InputProcessor;
        use tze_hud_scene::element_store::ElementType;

        let (store_snapshot, persist_path, new_geometry) = {
            let Ok(mut state) = self.state.shared_state.try_lock() else {
                tracing::warn!("persist_drag_release: could not acquire shared_state lock");
                return;
            };

            let new_geometry = tze_hud_input::drag::final_position_to_geometry(
                released.final_x,
                released.final_y,
                released.width,
                released.height,
                released.display_width,
                released.display_height,
            );

            InputProcessor::persist_drag_geometry(
                &mut state.element_store,
                ElementType::Tile,
                &released.namespace,
                released.final_x,
                released.final_y,
                released.width,
                released.height,
                released.display_width,
                released.display_height,
            );

            let store_snapshot = state.element_store.clone();
            let persist_path = state.element_store_path.clone();
            (store_snapshot, persist_path, new_geometry)
        };

        // Persist element store on a background thread (avoids blocking the
        // winit event loop with sync disk I/O).
        if let Some(path) = persist_path {
            std::thread::spawn(move || {
                if let Err(e) =
                    crate::element_store::persist_element_store_to_path(&store_snapshot, &path)
                {
                    tracing::warn!(
                        error = %e,
                        "persist_drag_release: element store persist failed"
                    );
                }
            });
        }

        // Broadcast ElementRepositionedEvent so gRPC subscribers are notified.
        if let Some(ref tx) = self.state.element_repositioned_tx {
            let event = tze_hud_protocol::proto::ElementRepositionedEvent {
                element_id: released.element_id.as_uuid().as_bytes().to_vec(),
                new_geometry: Some(tze_hud_protocol::convert::geometry_policy_to_proto(
                    &new_geometry,
                )),
                previous_geometry: None,
            };
            // Errors (no receivers, channel lagged) are silently ignored —
            // ElementRepositionedEvent is an ephemeral-realtime notification;
            // subscribers that are not present at the moment of a drag release
            // will learn the final geometry on next subscription refresh.
            tx.send(event).unwrap_or_default();
            tracing::debug!(
                element_id = %released.element_id,
                final_x = released.final_x,
                final_y = released.final_y,
                "ElementRepositionedEvent broadcast after drag release"
            );
        }
    }
    /// Run the in-process portal projection drain loop (hud-2iup7).
    ///
    /// Drain pending [`PortalOp`] messages from the MCP channel (hud-bq0gl.2).
    ///
    /// Called from `about_to_wait` BEFORE `drain_portal_projection` so that
    /// content published in the same event-loop tick is fed into the cadence
    /// coalescer and materialised by the immediately-following drain call.
    ///
    /// Uses `try_recv` in a non-blocking loop — never blocks the event-loop
    /// thread.  Each dispatched op calls `InProcessPortalDriver::dispatch_portal_op`
    /// which synchronously feeds the operation into `ProjectionAuthority`.
    pub(super) fn drain_portal_ops(&mut self) {
        let Some(ref mut rx) = self.state.portal_op_rx else {
            return;
        };
        loop {
            match rx.try_recv() {
                Ok(op) => {
                    self.state.portal_projection_driver.dispatch_portal_op(op);
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    tracing::warn!(
                        "portal_op channel disconnected — MCP portal tools will no longer function"
                    );
                    // The ingress that feeds portal ops went away without per-
                    // projection clean Detach ops — an ungraceful upstream drop.
                    // Latch every still-attached projection to the degraded
                    // treatment so the surfaces stop looking live (hud-5i16d).
                    self.state
                        .portal_projection_driver
                        .mark_all_projections_disconnected();
                    self.state.portal_op_rx = None;
                    break;
                }
            }
        }
    }

    /// Called from `about_to_wait` after composer-draft flush.  Drives
    /// `InProcessPortalDriver::drain` which calls
    /// `InputProcessor::notify_tile_content_appended` for every `RenderPortal`
    /// drain record that carries append geometry (spec §3.2 / §3.3).
    ///
    /// Uses `try_lock` on the shared scene to avoid blocking the main thread.
    /// If the scene lock is busy, the drain is silently deferred to the next
    /// `about_to_wait` call (NOT silent fail-open — the pending work is picked
    /// up on the very next iteration).
    pub(super) fn drain_portal_projection(&mut self) {
        let Ok(state) = self.state.shared_state.try_lock() else {
            tracing::trace!("portal drain deferred: shared_state lock busy");
            return;
        };
        let Ok(mut scene) = state.scene.try_lock() else {
            tracing::trace!("portal drain deferred: scene lock busy");
            return;
        };
        let tab_id = scene.active_tab;
        self.state.portal_projection_driver.drain(
            &mut scene,
            &mut self.state.input_processor,
            tab_id,
        );
        // The driver activates a tab when a cooperative portal needs to render
        // and none was active (hud-obw3q). Keep the lock-free keyboard-dispatch
        // mirror in sync so keyboard routing targets the newly active tab.
        if scene.active_tab != tab_id {
            state.refresh_active_tab_mirror(&scene);
        }
    }

    /// Remove stale entries from `portal_resize_states` for tiles that no
    /// longer exist in the scene.
    ///
    /// Called once per `about_to_wait` iteration.  Uses a two-phase approach:
    ///
    /// 1. **Eager drain** — drains `SceneGraph::recently_removed_tile_ids`
    ///    (populated by `remove_tile_and_nodes` on every `DeleteTile` mutation)
    ///    and removes each returned ID from `portal_resize_states` immediately.
    ///    This is O(removed) and requires only the scene lock.
    ///
    /// 2. **Fallback sweep** — if `portal_resize_states` is still non-empty
    ///    after the drain (e.g. entries that predated the drain queue, or
    ///    entries whose tiles were removed while the lock was busy), a full
    ///    `retain` sweep validates all remaining entries against the live tile
    ///    set.  Uses `try_lock`; deferred to next iteration if lock is busy.
    ///
    /// The eager path handles the common case (each `DeleteTile` generates
    /// exactly one removal notification).  The fallback prevents unbounded
    /// accumulation in pathological cases.
    pub(super) fn prune_portal_resize_states(&mut self) {
        let Ok(state) = self.state.shared_state.try_lock() else {
            tracing::trace!("portal resize prune deferred: shared_state lock busy");
            return;
        };
        let Ok(mut scene) = state.scene.try_lock() else {
            tracing::trace!("portal resize prune deferred: scene lock busy");
            return;
        };

        // Phase 1: eager drain — O(removed), handles the common `DeleteTile` path.
        let removed_ids = scene.drain_removed_tile_ids();
        let mut eagerly_removed = 0usize;
        for tile_id in removed_ids {
            if self.state.portal_resize_states.remove(&tile_id).is_some() {
                eagerly_removed += 1;
            }
        }
        if eagerly_removed > 0 {
            tracing::debug!(
                removed = eagerly_removed,
                remaining = self.state.portal_resize_states.len(),
                "portal resize: eagerly pruned resize-state entries for removed tiles"
            );
        }

        // Phase 2: fallback sweep — catches any entries that slipped through
        // (e.g. tiles removed before the drain queue existed, or during a
        // prior lock-busy deferral).
        if self.state.portal_resize_states.is_empty() {
            return;
        }
        let before = self.state.portal_resize_states.len();
        self.state
            .portal_resize_states
            .retain(|tile_id, _| scene.tiles.contains_key(tile_id));
        let swept = before - self.state.portal_resize_states.len();
        if swept > 0 {
            tracing::debug!(
                removed = swept,
                remaining = self.state.portal_resize_states.len(),
                "portal resize: sweep-pruned stale resize-state entries for removed tiles"
            );
        }
    }
    /// Apply a Ctrl-gated portal resize hotkey to the focused portal tile.
    ///
    /// Looks up the currently focused tile in `tab_id`. If the focused tile is
    /// a portal tile (has a registered scroll config), applies the resize step
    /// locally (local-first per §6b.2), updates the scene tile bounds, and
    /// broadcasts an `ElementRepositionedEvent` on the `SCENE_TOPOLOGY` channel
    /// via `element_repositioned_tx` so gRPC subscribers receive the updated
    /// portal geometry (relative %).
    ///
    /// Returns `true` when the hotkey was consumed (applied to a focused portal
    /// tile) so the caller knows to stop propagating the key event.
    /// Returns `false` when no focused portal tile was found (the key must not
    /// be consumed and should fall through to the normal dispatch path).
    pub(super) fn apply_portal_resize_hotkey(
        &mut self,
        tab_id: tze_hud_scene::SceneId,
        dir: tze_hud_input::HotkeyResizeDir,
    ) -> bool {
        // Resolve the focused tile from the focus manager.
        let focused_tile_id = match self.state.focus_manager.current_owner(tab_id).tile_id() {
            Some(id) => id,
            None => return false,
        };

        let display_w = self.state.config.window.width as f32;
        let display_h = self.state.config.window.height as f32;

        // Acquire scene + check if the focused tile is a portal surface (has a
        // scroll config), resolve the whole-portal group, and build the bounds
        // for clamping the whole-portal rect.
        let (group, old_rect, bounds) = {
            // A resize hotkey is a deliberate user action that must produce
            // visible feedback this frame; acquire with a bounded spin rather
            // than a single try_lock so a contended scene lock (compositor /
            // streaming publish) cannot silently swallow the resize.
            let Some(state) = spin_acquire(&self.state.shared_state, INTERACTION_LOCK_BUDGET)
            else {
                return false;
            };
            let Some(scene) = spin_acquire(&state.scene, INTERACTION_LOCK_BUDGET) else {
                return false;
            };
            // Only scrollable portal tiles accept resize hotkeys.
            if scene.tile_scroll_config(focused_tile_id).is_none() {
                return false;
            }
            // Resolve the whole portal (frame anchor + constituent surfaces)
            // that owns the focused surface, so the step scales the unit rather
            // than the single focused tile (hud-fb3en).
            let Some(group) = resolve_portal_group(&scene, focused_tile_id) else {
                return false;
            };
            let Some(anchor_lease) = scene.tiles.get(&group.anchor_tile_id).map(|t| t.lease_id)
            else {
                return false;
            };
            let old_rect = group.portal_rect;
            let portal_part = tze_hud_config::resolve_portal_tokens(&self.state.global_tokens);
            let tokens = PortalWindowTokens {
                min_width_px: portal_part.window_min_width_px,
                min_height_px: portal_part.window_min_height_px,
                resize_step_px: portal_part.window_resize_step_px,
                affordance_px: portal_part.window_resize_affordance_px,
            };
            // Resolve spatial budget from the anchor/frame's lease (the
            // whole-portal rect clamps against the portal's lease budget).
            let (lease_max_w, lease_max_h) = scene
                .leases
                .get(&anchor_lease)
                .map(|l| {
                    (
                        l.spatial_budget.max_tile_width_px,
                        l.spatial_budget.max_tile_height_px,
                    )
                })
                .unwrap_or((0.0, 0.0));
            let (max_width_px, max_height_px) = compute_portal_max_dims(
                lease_max_w,
                lease_max_h,
                display_w,
                display_h,
                tokens.min_width_px,
                tokens.min_height_px,
            );
            let resize_bounds = ResizeBounds {
                tokens,
                max_width_px,
                max_height_px,
                display_w,
                display_h,
            };
            (group, old_rect, resize_bounds)
        };

        // Get or lazily create the per-portal resize state, keyed by the anchor
        // (frame) tile so one gesture state tracks the whole portal.
        let resize_state = self
            .state
            .portal_resize_states
            .entry(group.anchor_tile_id)
            .or_insert_with(|| PortalResizeState::new(group.portal_id_hash));

        // Apply the hotkey resize to the whole-portal rect (O(1) on the hot path).
        let outcome = apply_hotkey_resize(
            true, // portal is focused (checked above)
            dir,
            old_rect,
            &bounds,
            resize_state,
        );

        let snapshot = match outcome {
            tze_hud_input::HotkeyResizeOutcome::Applied { snapshot } => snapshot,
            tze_hud_input::HotkeyResizeOutcome::NotFocused => return false,
        };

        // Local-first feedback: scale every constituent surface immediately in
        // the scene (same frame, no adapter roundtrip) per §6b.2 /
        // local-feedback-first, preserving relative layout around the top-left
        // anchor. `commit_portal_group_resize` bumps `scene.version` once when
        // the geometry actually changes, so the compositor re-primes the
        // truncation cache at the new geometry (hud-ghhxa — spec §6b.3) without
        // churning at a clamped boundary.
        let members = {
            let Some(state) = spin_acquire(&self.state.shared_state, INTERACTION_LOCK_BUDGET)
            else {
                return true; // hotkey consumed even if local update fails
            };
            let Some(mut scene) = spin_acquire(&state.scene, INTERACTION_LOCK_BUDGET) else {
                return true;
            };
            commit_portal_group_resize(&mut scene, &group, old_rect, snapshot)
        };

        // Broadcast a geometry snapshot per constituent surface to gRPC
        // subscribers via ElementRepositionedEvent (§6b.4: coalescible
        // state-stream delivery), and mirror each into the in-process projection
        // authority so the drain loop sees the updated bounds next cycle.
        for member in &members {
            dispatch_portal_geometry_event(
                &self.state.element_repositioned_tx,
                member.tile_id,
                &member.snapshot,
                display_w,
                display_h,
            );
            self.state
                .portal_projection_driver
                .push_geometry_snapshot_for_tile(member.tile_id, member.snapshot);
        }

        tracing::debug!(
            anchor_tile_id = ?group.anchor_tile_id,
            members = members.len(),
            new_width = snapshot.rect.width,
            new_height = snapshot.rect.height,
            sequence = snapshot.sequence,
            "portal resize: hotkey applied — whole-portal bounds updated locally"
        );

        true // hotkey consumed
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex as StdMutex};

    use tze_hud_input::{
        FocusManager, FocusRequest, InputProcessor, KeyboardProcessor, PointerEvent,
        PointerEventKind, PortalResizeState, PortalWindowTokens, RawKeyDownEvent, RawKeyUpEvent,
    };
    use tze_hud_telemetry::TelemetryCollector;

    use super::super::lifecycle::pointer_down_starts_guaranteed_feedback_gesture;
    use super::super::test_support::{
        make_shared_state, portal_scene_with_focus, scene_with_capture_tile,
        scene_with_drag_handle_tile,
    };
    use super::super::{WindowedConfig, WindowedRuntimeState, WinitApp};
    use super::*;
    use crate::channels::{INPUT_EVENT_CAPACITY, frame_ready_channel};
    use crate::pipeline::FramePipeline;
    use crate::runtime_context::RuntimeContext;
    use crate::threads::ShutdownToken;
    use crate::window::WindowMode;

    fn make_windowed_keyboard_test_app(
        scene: tze_hud_scene::graph::SceneGraph,
        focus_manager: FocusManager,
        input_processor: InputProcessor,
    ) -> (
        WinitApp,
        tokio::sync::broadcast::Receiver<(String, tze_hud_protocol::proto::EventBatch)>,
    ) {
        let cfg = WindowedConfig::default();
        let shared_state = make_shared_state();
        let (input_capture_tx, input_capture_rx) = tokio::sync::mpsc::unbounded_channel();
        let (_paste_inject_tx, paste_inject_rx) = tokio::sync::mpsc::unbounded_channel();
        let (frame_ready_tx, frame_ready_rx) = frame_ready_channel();
        let (input_event_tx, input_event_rx) = tokio::sync::broadcast::channel(8);
        let safe_mode_atomic = {
            let state = shared_state
                .try_lock()
                .expect("shared state must be uncontended in test setup");
            let mut scene_guard = state
                .scene
                .try_lock()
                .expect("scene must be uncontended in test setup");
            *scene_guard = scene;
            state.refresh_active_tab_mirror(&scene_guard);
            Arc::clone(&state.safe_mode_atomic)
        };
        let active_tab_mirror = {
            let state = shared_state
                .try_lock()
                .expect("shared state must be uncontended after scene setup");
            Arc::clone(&state.active_tab_mirror)
        };

        let state = WindowedRuntimeState {
            config: cfg,
            compositor_handle: None,
            network_rt: None,
            network_handles: Vec::new(),
            runtime_context: Arc::new(RuntimeContext::headless_default()),
            _runtime_widget_store: None,
            fallback_unrestricted: false,
            shared_state,
            safe_mode_atomic,
            active_tab_mirror,
            safe_mode_exit_tx: None,
            chrome_state: Arc::new(std::sync::RwLock::new(crate::shell::ChromeState::new())),
            input_ring: Arc::new(std::sync::Mutex::new(
                std::collections::VecDeque::with_capacity(INPUT_EVENT_CAPACITY),
            )),
            pending_input_latency: Arc::new(StdMutex::new(VecDeque::new())),
            frame_ready_rx,
            frame_ready_tx: Some(frame_ready_tx),
            compositor: None,
            window_surface: None,
            input_processor,
            input_capture_rx,
            pending_input_capture_commands: std::collections::VecDeque::new(),
            paste_inject_rx,
            focus_manager,
            keyboard_processor: KeyboardProcessor::new(),
            telemetry: TelemetryCollector::new(),
            pipeline: FramePipeline::new(),
            shutdown: ShutdownToken::new(),
            benchmark_failed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            cursor_x: 0.0,
            cursor_y: 0.0,
            left_button_down: false,
            cursor_tracker: tze_hud_input::CursorIconTracker::new(),
            window: None,
            effective_mode: WindowMode::Fullscreen,
            hit_regions: Vec::new(),
            static_hit_regions: Vec::new(),
            widget_hover_trackers: std::collections::HashMap::new(),
            pending_mode_switch: None,
            pending_widget_svgs: Vec::new(),
            modifiers: winit::keyboard::ModifiersState::empty(),
            current_monitor_index: 0,
            global_tokens: std::collections::HashMap::new(),
            element_repositioned_tx: None,
            input_event_tx: Some(input_event_tx),
            pending_blur_delivery_context: None,
            portal_resize_states: std::collections::HashMap::new(),
            consumed_portal_resize_keydowns: std::collections::HashSet::new(),
            local_composer_state: Arc::new(StdMutex::new(None)),
            portal_projection_driver: crate::portal_projection_driver::InProcessPortalDriver::new(),
            portal_op_rx: None,
            pending_keyboard_events: VecDeque::new(),
            resident_grpc_bridge: None,
        };

        drop(input_capture_tx);

        (WinitApp { state }, input_event_rx)
    }

    // ── Drag-to-move: long-press drag moves a text stream portal tile [hud-9yfce] ──

    /// A long-press drag on a tile's drag handle must move the tile's bounds and
    /// return a `DragReleasedData` payload when the pointer is released.
    ///
    /// Acceptance criteria for hud-9yfce:
    /// - `Moved` outcome during pointer-move updates `tile.bounds` immediately.
    /// - `Released` outcome on pointer-up produces persist data.
    /// - Click-focus is unaffected: a short tap (no long-press) produces no move.
    #[test]
    fn drag_to_move_long_press_moves_tile_bounds() {
        use std::thread;
        use std::time::Duration;
        use tze_hud_input::{InputProcessor, PointerEvent};

        let (mut scene, tile_id, element_id, _interaction_id) =
            scene_with_drag_handle_tile(400.0, 300.0, 600.0, 200.0);

        // The drag handle was placed at:
        //   x = 400 + 600/2 - 20 = 680, y = 300 - 10 = 290, w=40, h=20
        // So the handle spans x: 680..720, y: 290..310.
        let handle_cx = 700.0_f32; // centre of the handle
        let handle_cy = 300.0_f32;

        let mut processor = InputProcessor::new();

        // ── Step 1: PointerDown on the drag handle ────────────────────────────
        let down = PointerEvent {
            x: handle_cx,
            y: handle_cy,
            kind: PointerEventKind::Down,
            device_id: 0,
            timestamp: None,
        };
        // process() produces the HitResult for the drag handle.
        let result_down = processor.process(&down, &mut scene);
        assert!(
            result_down.hit.is_zone_interaction(),
            "pointer-down on drag handle must produce ZoneInteraction hit"
        );

        // Drive the drag state machine — should start accumulating.
        let released_on_down = super::apply_drag_handle_pointer_event(
            &mut processor,
            &down,
            &result_down.hit,
            &mut scene,
            1920.0,
            1080.0,
        );
        assert!(
            released_on_down.is_none(),
            "PointerDown must not trigger release"
        );
        assert!(
            processor.drag_states.contains_key(&0),
            "drag state must be created for device 0 after PointerDown on handle"
        );

        // ── Step 2: Wait for long-press threshold (250 ms) ───────────────────
        thread::sleep(Duration::from_millis(260));

        // ── Step 3: PointerMove — first move activates the drag, second moves tile ──
        //
        // The state machine on the first PointerMove after the threshold transitions
        // Accumulating → Activated (returns `Activated`, not `Moved` yet).  The
        // grab offset is recorded at activation.  Subsequent PointerMove events
        // return `Moved` and update the tile bounds.
        let move1 = PointerEvent {
            x: handle_cx + 5.0, // small nudge triggers Activated
            y: handle_cy,
            kind: PointerEventKind::Move,
            device_id: 0,
            timestamp: None,
        };
        let result_move1 = processor.process(&move1, &mut scene);
        let released_on_move1 = super::apply_drag_handle_pointer_event(
            &mut processor,
            &move1,
            &result_move1.hit,
            &mut scene,
            1920.0,
            1080.0,
        );
        assert!(
            released_on_move1.is_none(),
            "first PointerMove (Activated) must not trigger release"
        );

        // Second PointerMove — now in Activated phase, returns Moved.
        let move2 = PointerEvent {
            x: handle_cx + 100.0,
            y: handle_cy + 50.0,
            kind: PointerEventKind::Move,
            device_id: 0,
            timestamp: None,
        };
        let result_move2 = processor.process(&move2, &mut scene);
        let released_on_move2 = super::apply_drag_handle_pointer_event(
            &mut processor,
            &move2,
            &result_move2.hit,
            &mut scene,
            1920.0,
            1080.0,
        );
        assert!(
            released_on_move2.is_none(),
            "PointerMove must not trigger release"
        );

        // The tile must have moved from its original position.
        let tile_after_move = scene.tiles.get(&tile_id).expect("tile must exist");
        assert_ne!(
            tile_after_move.bounds.x, 400.0,
            "tile X must change after drag move"
        );
        assert_ne!(
            tile_after_move.bounds.y, 300.0,
            "tile Y must change after drag move"
        );

        // ── Step 4: PointerUp — should release and return persist data ────────
        // Release at the same position as the last move.
        let up = PointerEvent {
            x: handle_cx + 100.0,
            y: handle_cy + 50.0,
            kind: PointerEventKind::Up,
            device_id: 0,
            timestamp: None,
        };
        let result_up = processor.process(&up, &mut scene);
        let released_on_up = super::apply_drag_handle_pointer_event(
            &mut processor,
            &up,
            &result_up.hit,
            &mut scene,
            1920.0,
            1080.0,
        );

        let released =
            released_on_up.expect("PointerUp after activated drag must return released data");
        assert_eq!(
            released.element_id, element_id,
            "released element_id must match the dragged tile"
        );
        assert!(
            released.final_x >= 0.0 && released.final_x + released.width <= 1920.0,
            "final X must be within display bounds"
        );
        assert!(
            released.final_y >= 0.0 && released.final_y + released.height <= 1080.0,
            "final Y must be within display bounds"
        );

        // Drag state must be cleaned up after release.
        assert!(
            !processor.drag_states.contains_key(&0),
            "drag state must be removed after PointerUp"
        );
    }

    /// A quick tap (PointerDown immediately followed by PointerUp, no long-press)
    /// on a drag handle must NOT move the tile — the click-to-focus path must be
    /// unaffected.
    ///
    /// Hysteresis: the 250 ms threshold ensures taps are recognised as clicks,
    /// not drags.
    #[test]
    fn drag_to_move_quick_tap_does_not_move_tile() {
        use tze_hud_input::{InputProcessor, PointerEvent};

        let (mut scene, tile_id, _element_id, _interaction_id) =
            scene_with_drag_handle_tile(400.0, 300.0, 600.0, 200.0);

        // Same drag handle position as drag_to_move_long_press_moves_tile_bounds:
        //   x: 680..720, y: 290..310.
        let handle_cx = 700.0_f32; // centre of the handle
        let handle_cy = 300.0_f32;

        let mut processor = InputProcessor::new();

        // PointerDown.
        let down = PointerEvent {
            x: handle_cx,
            y: handle_cy,
            kind: PointerEventKind::Down,
            device_id: 0,
            timestamp: None,
        };
        let result_down = processor.process(&down, &mut scene);
        let _ = super::apply_drag_handle_pointer_event(
            &mut processor,
            &down,
            &result_down.hit,
            &mut scene,
            1920.0,
            1080.0,
        );

        // PointerUp immediately — no long-press threshold met.
        let up = PointerEvent {
            x: handle_cx,
            y: handle_cy,
            kind: PointerEventKind::Up,
            device_id: 0,
            timestamp: None,
        };
        let result_up = processor.process(&up, &mut scene);
        let released_on_up = super::apply_drag_handle_pointer_event(
            &mut processor,
            &up,
            &result_up.hit,
            &mut scene,
            1920.0,
            1080.0,
        );

        // Must NOT return release data — this was a tap, not a drag.
        assert!(
            released_on_up.is_none(),
            "quick tap must not trigger drag release [click-focus coexistence]"
        );

        // Tile bounds must be unchanged.
        let tile = scene.tiles.get(&tile_id).expect("tile must exist");
        assert_eq!(
            tile.bounds.x, 400.0,
            "tile X must not change after a tap on the drag handle"
        );
        assert_eq!(
            tile.bounds.y, 300.0,
            "tile Y must not change after a tap on the drag handle"
        );
    }

    #[test]
    fn pointer_down_on_drag_handle_requests_guaranteed_feedback_from_snapshot_gate() {
        let (scene, _tile_id, _element_id, _interaction_id) =
            scene_with_drag_handle_tile(400.0, 300.0, 600.0, 200.0);
        let snapshot = crate::pipeline::HitTestSnapshot::from_scene(&scene);

        assert!(
            pointer_down_starts_guaranteed_feedback_gesture(
                &snapshot,
                700.0,
                300.0,
                None,
                &FocusManager::new(),
                PortalWindowTokens::default(),
            ),
            "PointerDown on a drag handle must spin-acquire so the drag state can start under contention"
        );
    }

    #[test]
    fn pointer_down_on_resize_affordance_requests_guaranteed_feedback_from_snapshot_gate() {
        let (scene, tab_id, _tile_id, fm) = portal_scene_with_focus();
        let snapshot = crate::pipeline::HitTestSnapshot::from_scene(&scene);

        assert!(
            pointer_down_starts_guaranteed_feedback_gesture(
                &snapshot,
                496.0,
                250.0,
                Some(tab_id),
                &fm,
                PortalWindowTokens::default(),
            ),
            "PointerDown on a portal resize affordance must spin-acquire so the resize gesture can start under contention"
        );
    }

    #[test]
    fn ordinary_pointer_down_does_not_request_guaranteed_feedback_spin() {
        let (scene, _tile_id) = scene_with_capture_tile();
        let snapshot = crate::pipeline::HitTestSnapshot::from_scene(&scene);

        assert!(
            !pointer_down_starts_guaranteed_feedback_gesture(
                &snapshot,
                320.0,
                420.0,
                scene.active_tab,
                &FocusManager::new(),
                PortalWindowTokens::default(),
            ),
            "ordinary content PointerDown must stay on the single try_lock path to preserve click-to-focus latency"
        );
    }

    #[test]
    fn ctrl_resize_hotkey_resizes_focused_portal_while_composer_active() {
        use tze_hud_input::{FocusManager, InputProcessor, KeyboardModifiers};
        use tze_hud_scene::types::{HitRegionNode, TileScrollConfig};
        use tze_hud_scene::{Capability, Node, NodeData, Rect, SceneGraph, SceneId};

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "portal-agent",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let tile_id = scene
            .create_tile(
                tab_id,
                "portal-agent",
                lease_id,
                Rect::new(100.0, 100.0, 400.0, 300.0),
                1,
            )
            .unwrap();
        scene
            .register_tile_scroll_config(tile_id, TileScrollConfig::vertical())
            .unwrap();

        let composer_id = SceneId::new();
        scene
            .set_tile_root(
                tile_id,
                Node {
                    id: composer_id,
                    children: vec![],
                    data: NodeData::HitRegion(HitRegionNode {
                        bounds: Rect::new(0.0, 0.0, 400.0, 60.0),
                        interaction_id: "portal-composer".to_string(),
                        accepts_focus: true,
                        accepts_pointer: true,
                        accepts_composer_input: true,
                        ..Default::default()
                    }),
                },
            )
            .unwrap();

        let mut processor = InputProcessor::new();
        let mut focus_manager = FocusManager::new();
        focus_manager.add_tab(tab_id);
        let (_input_result, transition) = processor.process_with_focus(
            &tze_hud_input::PointerEvent {
                x: 110.0,
                y: 110.0,
                kind: tze_hud_input::PointerEventKind::Down,
                device_id: 1,
                timestamp: None,
            },
            &mut scene,
            &mut focus_manager,
            tab_id,
        );
        assert!(
            transition.and_then(|t| t.gained).is_some(),
            "pointer down must focus the composer node"
        );
        assert!(
            processor.is_composer_active(),
            "composer must be active after focusing the portal composer"
        );

        let (mut app, mut input_event_rx) =
            make_windowed_keyboard_test_app(scene, focus_manager, processor);

        let bounds = |app: &WinitApp| {
            let shared = app
                .state
                .shared_state
                .try_lock()
                .expect("shared state must be available during key dispatch test");
            let scene = shared
                .scene
                .try_lock()
                .expect("scene must be available during key dispatch test");
            scene.tiles.get(&tile_id).unwrap().bounds
        };

        let dispatch_ctrl_key =
            |app: &mut WinitApp, key_code: &str, key: &str, shift: bool, timestamp: u64| {
                app.dispatch_key_down_event_inner(
                    &RawKeyDownEvent {
                        key_code: key_code.to_string(),
                        key: key.to_string(),
                        modifiers: KeyboardModifiers {
                            ctrl: true,
                            shift,
                            ..KeyboardModifiers::NONE
                        },
                        repeat: false,
                        timestamp_mono_us: tze_hud_scene::MonoUs(timestamp),
                    },
                    Some(tab_id),
                );
            };
        let dispatch_ctrl_key_up =
            |app: &mut WinitApp, key_code: &str, key: &str, shift: bool, timestamp: u64| {
                app.dispatch_key_up_event_inner(
                    &RawKeyUpEvent {
                        key_code: key_code.to_string(),
                        key: key.to_string(),
                        modifiers: KeyboardModifiers {
                            ctrl: true,
                            shift,
                            ..KeyboardModifiers::NONE
                        },
                        timestamp_mono_us: tze_hud_scene::MonoUs(timestamp),
                    },
                    Some(tab_id),
                );
            };

        let before_equal = bounds(&app);
        dispatch_ctrl_key(&mut app, "Equal", "=", false, 1);
        let after_equal = bounds(&app);

        assert!(
            after_equal.width > before_equal.width,
            "Ctrl+= must grow the focused portal even when the composer is active"
        );
        assert!(
            after_equal.height > before_equal.height,
            "Ctrl+= must grow the focused portal vertically as well"
        );

        // The matching KeyUp for a KeyDown that already resized must be swallowed
        // by the dedup set, NOT applied as a second resize (hud-v4k1h).
        dispatch_ctrl_key_up(&mut app, "Equal", "=", false, 2);
        let after_equal_release = bounds(&app);
        assert_eq!(
            after_equal_release.width, after_equal.width,
            "matching Ctrl+= KeyUp must not apply a second horizontal resize after KeyDown already resized"
        );
        assert_eq!(
            after_equal_release.height, after_equal.height,
            "matching Ctrl+= KeyUp must not apply a second vertical resize after KeyDown already resized"
        );

        dispatch_ctrl_key(&mut app, "Equal", "+", true, 3);
        let after_plus = bounds(&app);
        assert!(
            after_plus.width > after_equal_release.width,
            "Ctrl++ must grow the focused portal even when the composer is active"
        );
        assert!(
            after_plus.height > after_equal_release.height,
            "Ctrl++ must grow the focused portal vertically as well"
        );

        dispatch_ctrl_key(&mut app, "Minus", "-", false, 4);
        let after_minus = bounds(&app);
        assert!(
            after_minus.width < after_plus.width,
            "Ctrl+- must shrink the focused portal even when the composer is active"
        );
        assert!(
            after_minus.height < after_plus.height,
            "Ctrl+- must shrink the focused portal vertically as well"
        );
        assert!(
            input_event_rx.try_recv().is_err(),
            "resize hotkey must be consumed locally, not forwarded as agent keyboard input"
        );
    }

    /// hud-02sp5: focus acquired via keyboard Tab traversal (no pointer) must
    /// enable the same keyboard operations as click-to-focus. A keyboard-only
    /// viewer (Mobile Presence Node / glasses) Tabs onto the portal composer and
    /// then the Ctrl+= resize chord must resolve and resize the focused portal —
    /// exactly as it does when focus was acquired by a click.
    #[test]
    fn ctrl_resize_hotkey_resizes_portal_focused_via_tab_without_pointer() {
        use tze_hud_input::{FocusManager, InputProcessor, KeyboardModifiers};
        use tze_hud_scene::types::{HitRegionNode, TileScrollConfig};
        use tze_hud_scene::{Capability, Node, NodeData, Rect, SceneGraph, SceneId};

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "portal-agent",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let tile_id = scene
            .create_tile(
                tab_id,
                "portal-agent",
                lease_id,
                Rect::new(100.0, 100.0, 400.0, 300.0),
                1,
            )
            .unwrap();
        scene
            .register_tile_scroll_config(tile_id, TileScrollConfig::vertical())
            .unwrap();

        // Focusable composer affordance — the only Tab stop in this portal.
        let composer_id = SceneId::new();
        scene
            .set_tile_root(
                tile_id,
                Node {
                    id: composer_id,
                    children: vec![],
                    data: NodeData::HitRegion(HitRegionNode {
                        bounds: Rect::new(0.0, 0.0, 400.0, 60.0),
                        interaction_id: "portal-composer".to_string(),
                        accepts_focus: true,
                        accepts_pointer: true,
                        accepts_composer_input: true,
                        ..Default::default()
                    }),
                },
            )
            .unwrap();

        // NB: no pointer event is ever synthesized. Focus is acquired purely
        // through the windowed Tab key path.
        let processor = InputProcessor::new();
        let mut focus_manager = FocusManager::new();
        focus_manager.add_tab(tab_id);

        let (mut app, _input_event_rx) =
            make_windowed_keyboard_test_app(scene, focus_manager, processor);

        let bounds = |app: &WinitApp| {
            let shared = app
                .state
                .shared_state
                .try_lock()
                .expect("shared state must be available during key dispatch test");
            let scene = shared
                .scene
                .try_lock()
                .expect("scene must be available during key dispatch test");
            scene.tiles.get(&tile_id).unwrap().bounds
        };

        // Bare Tab (no modifiers) advances focus onto the composer — the
        // no-pointer analogue of a click. This is the wiring proven by
        // hud-v0cal (PR #980); here we assert it unlocks the resize chord.
        app.dispatch_key_down_event_inner(
            &RawKeyDownEvent {
                key_code: "Tab".to_string(),
                key: "Tab".to_string(),
                modifiers: KeyboardModifiers::NONE,
                repeat: false,
                timestamp_mono_us: tze_hud_scene::MonoUs(1),
            },
            Some(tab_id),
        );
        assert!(
            app.state.input_processor.is_composer_active(),
            "bare Tab must focus the portal composer without any pointer event"
        );

        let dispatch_ctrl_key =
            |app: &mut WinitApp, key_code: &str, key: &str, shift: bool, timestamp: u64| {
                app.dispatch_key_down_event_inner(
                    &RawKeyDownEvent {
                        key_code: key_code.to_string(),
                        key: key.to_string(),
                        modifiers: KeyboardModifiers {
                            ctrl: true,
                            shift,
                            ..KeyboardModifiers::NONE
                        },
                        repeat: false,
                        timestamp_mono_us: tze_hud_scene::MonoUs(timestamp),
                    },
                    Some(tab_id),
                );
            };

        let before = bounds(&app);
        dispatch_ctrl_key(&mut app, "Equal", "=", false, 2);
        let after = bounds(&app);
        assert!(
            after.width > before.width && after.height > before.height,
            "Ctrl+= must grow the portal whose composer was focused via Tab \
             (Tab-acquired focus must enable the same keyboard ops as a click)"
        );
    }

    /// Regression for hud-v4k1h: on live Windows the OS (SendInput) can deliver
    /// ONLY the `KeyUp` for the Equal/Minus chord — the `KeyDown` never arrives
    /// while Ctrl is held. A key-down-only resize intercept therefore silently
    /// does nothing. The key-up fallback in `dispatch_key_up_event_inner` must
    /// resize on the release in that case.
    #[test]
    fn ctrl_resize_keyup_fallback_resizes_when_live_windows_omits_keydown() {
        use tze_hud_input::{FocusManager, InputProcessor, KeyboardModifiers};
        use tze_hud_scene::types::{HitRegionNode, TileScrollConfig};
        use tze_hud_scene::{Capability, Node, NodeData, Rect, SceneGraph, SceneId};

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "portal-agent",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let tile_id = scene
            .create_tile(
                tab_id,
                "portal-agent",
                lease_id,
                Rect::new(100.0, 100.0, 400.0, 300.0),
                1,
            )
            .unwrap();
        scene
            .register_tile_scroll_config(tile_id, TileScrollConfig::vertical())
            .unwrap();

        let composer_id = SceneId::new();
        scene
            .set_tile_root(
                tile_id,
                Node {
                    id: composer_id,
                    children: vec![],
                    data: NodeData::HitRegion(HitRegionNode {
                        bounds: Rect::new(0.0, 0.0, 400.0, 60.0),
                        interaction_id: "portal-composer".to_string(),
                        accepts_focus: true,
                        accepts_pointer: true,
                        accepts_composer_input: true,
                        ..Default::default()
                    }),
                },
            )
            .unwrap();

        let mut processor = InputProcessor::new();
        let mut focus_manager = FocusManager::new();
        focus_manager.add_tab(tab_id);
        let (_input_result, transition) = processor.process_with_focus(
            &tze_hud_input::PointerEvent {
                x: 110.0,
                y: 110.0,
                kind: tze_hud_input::PointerEventKind::Down,
                device_id: 1,
                timestamp: None,
            },
            &mut scene,
            &mut focus_manager,
            tab_id,
        );
        assert!(
            transition.and_then(|t| t.gained).is_some(),
            "pointer down must focus the composer node"
        );

        let (mut app, mut input_event_rx) =
            make_windowed_keyboard_test_app(scene, focus_manager, processor);

        let bounds = |app: &WinitApp| {
            let shared = app
                .state
                .shared_state
                .try_lock()
                .expect("shared state must be available during key dispatch test");
            let scene = shared
                .scene
                .try_lock()
                .expect("scene must be available during key dispatch test");
            scene.tiles.get(&tile_id).unwrap().bounds
        };

        let dispatch_ctrl_key_up =
            |app: &mut WinitApp, key_code: &str, key: &str, shift: bool, timestamp: u64| {
                app.dispatch_key_up_event_inner(
                    &RawKeyUpEvent {
                        key_code: key_code.to_string(),
                        key: key.to_string(),
                        modifiers: KeyboardModifiers {
                            ctrl: true,
                            shift,
                            ..KeyboardModifiers::NONE
                        },
                        timestamp_mono_us: tze_hud_scene::MonoUs(timestamp),
                    },
                    Some(tab_id),
                );
            };

        // No KeyDown was ever consumed, so each KeyUp must drive the resize.
        let before_equal = bounds(&app);
        dispatch_ctrl_key_up(&mut app, "Equal", "=", false, 1);
        let after_equal = bounds(&app);
        assert!(
            after_equal.width > before_equal.width,
            "Ctrl+= release fallback must grow the focused portal when the live OS stream omitted Equal KeyDown"
        );
        assert!(
            after_equal.height > before_equal.height,
            "Ctrl+= release fallback must grow the focused portal vertically as well"
        );

        dispatch_ctrl_key_up(&mut app, "Minus", "-", false, 2);
        let after_minus = bounds(&app);
        assert!(
            after_minus.width < after_equal.width,
            "Ctrl+- release fallback must shrink the focused portal when the live OS stream omitted Minus KeyDown"
        );
        assert!(
            after_minus.height < after_equal.height,
            "Ctrl+- release fallback must shrink the focused portal vertically as well"
        );
        assert!(
            input_event_rx.try_recv().is_err(),
            "release-fallback resize hotkey must be consumed locally, not forwarded as agent keyboard input"
        );
    }

    #[test]
    fn ctrl_resize_hotkey_ignores_unfocused_portal_on_windowed_dispatch_path() {
        use tze_hud_input::{FocusManager, FocusRequest, InputProcessor, KeyboardModifiers};
        use tze_hud_protocol::proto::input_envelope::Event as InputEvent;
        use tze_hud_scene::types::TileScrollConfig;
        use tze_hud_scene::{Capability, Rect, SceneGraph};

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "portal-agent",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let portal_tile_id = scene
            .create_tile(
                tab_id,
                "portal-agent",
                lease_id,
                Rect::new(100.0, 100.0, 400.0, 300.0),
                1,
            )
            .unwrap();
        scene
            .register_tile_scroll_config(portal_tile_id, TileScrollConfig::vertical())
            .unwrap();

        let plain_tile_id = scene
            .create_tile(
                tab_id,
                "portal-agent",
                lease_id,
                Rect::new(700.0, 100.0, 240.0, 160.0),
                2,
            )
            .unwrap();

        let mut focus_manager = FocusManager::new();
        let (focus_result, _transition) = focus_manager.request_focus(
            FocusRequest {
                tile_id: plain_tile_id,
                node_id: None,
                steal: true,
                requesting_namespace: "portal-agent".to_string(),
            },
            tab_id,
            &scene,
        );
        assert_eq!(
            focus_result,
            tze_hud_input::FocusResult::Granted,
            "test setup must focus the non-portal tile"
        );

        let (mut app, mut input_event_rx) =
            make_windowed_keyboard_test_app(scene, focus_manager, InputProcessor::new());

        let portal_bounds = |app: &WinitApp| {
            let shared = app
                .state
                .shared_state
                .try_lock()
                .expect("shared state must be available during key dispatch test");
            let scene = shared
                .scene
                .try_lock()
                .expect("scene must be available during key dispatch test");
            scene.tiles.get(&portal_tile_id).unwrap().bounds
        };

        let before = portal_bounds(&app);
        app.dispatch_key_down_event_inner(
            &RawKeyDownEvent {
                key_code: "Equal".to_string(),
                key: "=".to_string(),
                modifiers: KeyboardModifiers {
                    ctrl: true,
                    ..KeyboardModifiers::NONE
                },
                repeat: false,
                timestamp_mono_us: tze_hud_scene::MonoUs(1),
            },
            Some(tab_id),
        );
        let after = portal_bounds(&app);

        assert_eq!(
            after, before,
            "Ctrl+= must not resize a portal that does not hold focus"
        );
        assert!(
            app.state.portal_resize_states.is_empty(),
            "unfocused portal hotkey must not create resize state"
        );
        let (namespace, batch) = input_event_rx
            .try_recv()
            .expect("non-consumed hotkey should continue to normal keyboard routing");
        assert_eq!(namespace, "portal-agent");
        match batch
            .events
            .first()
            .and_then(|envelope| envelope.event.as_ref())
        {
            Some(InputEvent::KeyDown(ev)) => {
                assert_eq!(ev.key, "=");
                assert!(ev.ctrl, "forwarded key event must preserve Ctrl");
            }
            other => panic!("expected forwarded KeyDown event, got {other:?}"),
        }
    }

    // ── Portal control keyboard recovery / activation (hud-2v8br) ──────────
    //
    // Build a portal tile whose root composer (accepts_composer_input) has a
    // non-composer minimize control as a focusable child, land Tab focus on the
    // control, and assert the runtime never strands the keyboard user.

    /// Construct a portal scene: one tile rooted at a composer node with a
    /// focusable non-composer control child. Returns
    /// `(scene, tab_id, tile_id, composer_id, control_id)`.
    fn portal_scene_with_control() -> (
        tze_hud_scene::graph::SceneGraph,
        tze_hud_scene::SceneId,
        tze_hud_scene::SceneId,
        tze_hud_scene::SceneId,
        tze_hud_scene::SceneId,
    ) {
        use tze_hud_scene::types::{HitRegionNode, TileScrollConfig};
        use tze_hud_scene::{Capability, Node, NodeData, Rect, SceneGraph, SceneId};

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "portal-agent",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let tile_id = scene
            .create_tile(
                tab_id,
                "portal-agent",
                lease_id,
                Rect::new(100.0, 100.0, 400.0, 300.0),
                1,
            )
            .unwrap();
        scene
            .register_tile_scroll_config(tile_id, TileScrollConfig::vertical())
            .unwrap();

        let composer_id = SceneId::new();
        let control_id = SceneId::new();
        scene.nodes.insert(
            composer_id,
            Node {
                id: composer_id,
                children: vec![control_id],
                data: NodeData::HitRegion(HitRegionNode {
                    bounds: Rect::new(0.0, 0.0, 400.0, 60.0),
                    interaction_id: "portal-composer".to_string(),
                    accepts_focus: true,
                    accepts_pointer: true,
                    accepts_composer_input: true,
                    ..Default::default()
                }),
            },
        );
        scene.nodes.insert(
            control_id,
            Node {
                id: control_id,
                children: vec![],
                data: NodeData::HitRegion(HitRegionNode {
                    bounds: Rect::new(10.0, 80.0, 40.0, 40.0),
                    interaction_id: "portal-minimize".to_string(),
                    accepts_focus: true,
                    accepts_pointer: true,
                    accepts_composer_input: false,
                    ..Default::default()
                }),
            },
        );
        scene.tiles.get_mut(&tile_id).unwrap().root_node = Some(composer_id);
        (scene, tab_id, tile_id, composer_id, control_id)
    }

    /// Enter on a Tab-focused portal control activates it by broadcasting a
    /// synthetic PointerDown (+PointerUp) carrying the control's interaction_id,
    /// so the owning agent's click handler fires — the control is not a dead stop.
    #[test]
    fn enter_on_focused_portal_control_activates_via_synthetic_pointer() {
        use tze_hud_input::KeyboardModifiers;
        use tze_hud_protocol::proto::input_envelope::Event as InputEvent;

        let (mut scene, tab_id, _tile_id, composer_id, control_id) = portal_scene_with_control();
        let mut processor = InputProcessor::new();
        let mut focus_manager = FocusManager::new();
        focus_manager.add_tab(tab_id);
        // Tab onto the composer, then Tab again onto the control.
        processor.navigate_focus(&mut focus_manager, &mut scene, tab_id, false);
        processor.navigate_focus(&mut focus_manager, &mut scene, tab_id, false);
        assert_eq!(
            focus_manager.current_owner(tab_id).node_id(),
            Some(control_id),
            "test setup: focus must rest on the non-composer control"
        );
        assert!(
            !processor.is_composer_active(),
            "composer must be inactive while the control holds focus"
        );

        let (mut app, mut input_event_rx) =
            make_windowed_keyboard_test_app(scene, focus_manager, processor);

        app.dispatch_key_down_event_inner(
            &RawKeyDownEvent {
                key_code: "Enter".to_string(),
                key: "Enter".to_string(),
                modifiers: KeyboardModifiers::NONE,
                repeat: false,
                timestamp_mono_us: tze_hud_scene::MonoUs(1),
            },
            Some(tab_id),
        );

        // First broadcast must be a synthetic PointerDown on the control.
        let (namespace, batch) = input_event_rx
            .try_recv()
            .expect("Enter on a focused control must broadcast a synthetic pointer event");
        assert_eq!(namespace, "portal-agent");
        match batch.events.first().and_then(|e| e.event.as_ref()) {
            Some(InputEvent::PointerDown(ev)) => {
                assert_eq!(
                    ev.interaction_id, "portal-minimize",
                    "activation must target the focused control's interaction_id"
                );
            }
            other => panic!("expected synthetic PointerDown, got {other:?}"),
        }
        // A matching PointerUp completes the synthetic click.
        let (_ns, up_batch) = input_event_rx
            .try_recv()
            .expect("activation must also broadcast a PointerUp");
        assert!(
            matches!(
                up_batch.events.first().and_then(|e| e.event.as_ref()),
                Some(InputEvent::PointerUp(_))
            ),
            "second synthetic event must be a PointerUp"
        );
        // Focus stays on the control (activation is not a focus move).
        let _ = composer_id;
    }

    /// Typing a printable character while a portal control holds focus recovers
    /// to the composer and inserts the character — a keyboard user is never
    /// stranded with dead typing.
    #[test]
    fn typing_on_focused_portal_control_recovers_to_composer() {
        use tze_hud_input::RawCharacterEvent;

        let (mut scene, tab_id, _tile_id, composer_id, control_id) = portal_scene_with_control();
        let mut processor = InputProcessor::new();
        let mut focus_manager = FocusManager::new();
        focus_manager.add_tab(tab_id);
        processor.navigate_focus(&mut focus_manager, &mut scene, tab_id, false);
        processor.navigate_focus(&mut focus_manager, &mut scene, tab_id, false);
        assert_eq!(
            focus_manager.current_owner(tab_id).node_id(),
            Some(control_id),
            "test setup: focus must rest on the control"
        );

        let (mut app, _input_event_rx) =
            make_windowed_keyboard_test_app(scene, focus_manager, processor);

        app.dispatch_character_event_inner(
            &RawCharacterEvent {
                character: "h".to_string(),
                timestamp_mono_us: tze_hud_scene::MonoUs(1),
            },
            Some(tab_id),
        );

        assert!(
            app.state.input_processor.is_composer_active(),
            "typing on a control must recover focus to the composer (draft active)"
        );
        assert_eq!(
            app.state.focus_manager.current_owner(tab_id).node_id(),
            Some(composer_id),
            "recovery must move focus onto the composer node"
        );
        assert_eq!(
            app.state
                .input_processor
                .composer_draft_snapshot()
                .map(|s| s.0),
            Some("h".to_string()),
            "the typed character must land in the composer draft after recovery"
        );
    }

    /// Escape on a Tab-focused portal control recovers focus to the composer.
    #[test]
    fn escape_on_focused_portal_control_recovers_to_composer() {
        use tze_hud_input::KeyboardModifiers;

        let (mut scene, tab_id, _tile_id, composer_id, control_id) = portal_scene_with_control();
        let mut processor = InputProcessor::new();
        let mut focus_manager = FocusManager::new();
        focus_manager.add_tab(tab_id);
        processor.navigate_focus(&mut focus_manager, &mut scene, tab_id, false);
        processor.navigate_focus(&mut focus_manager, &mut scene, tab_id, false);
        assert_eq!(
            focus_manager.current_owner(tab_id).node_id(),
            Some(control_id)
        );

        let (mut app, _input_event_rx) =
            make_windowed_keyboard_test_app(scene, focus_manager, processor);

        app.dispatch_key_down_event_inner(
            &RawKeyDownEvent {
                key_code: "Escape".to_string(),
                key: "Escape".to_string(),
                modifiers: KeyboardModifiers::NONE,
                repeat: false,
                timestamp_mono_us: tze_hud_scene::MonoUs(1),
            },
            Some(tab_id),
        );

        assert_eq!(
            app.state.focus_manager.current_owner(tab_id).node_id(),
            Some(composer_id),
            "Escape on a control must recover focus to the composer"
        );
        assert!(
            app.state.input_processor.is_composer_active(),
            "composer draft must be active after Escape recovery"
        );
    }

    #[test]
    fn shell_reserved_ctrl_tab_does_not_resize_focused_portal() {
        use tze_hud_input::{InputProcessor, KeyboardModifiers};

        let (scene, tab_id, tile_id, focus_manager) = portal_scene_with_focus();
        let (mut app, _input_event_rx) =
            make_windowed_keyboard_test_app(scene, focus_manager, InputProcessor::new());

        let bounds = |app: &WinitApp| {
            let shared = app
                .state
                .shared_state
                .try_lock()
                .expect("shared state must be available during key dispatch test");
            let scene = shared
                .scene
                .try_lock()
                .expect("scene must be available during key dispatch test");
            scene.tiles.get(&tile_id).unwrap().bounds
        };

        let before = bounds(&app);
        app.dispatch_key_down_event_inner(
            &RawKeyDownEvent {
                key_code: "Tab".to_string(),
                key: "Tab".to_string(),
                modifiers: KeyboardModifiers {
                    ctrl: true,
                    ..KeyboardModifiers::NONE
                },
                repeat: false,
                timestamp_mono_us: tze_hud_scene::MonoUs(1),
            },
            Some(tab_id),
        );
        let after = bounds(&app);

        assert_eq!(
            after, before,
            "shell-reserved Ctrl+Tab must not be consumed as a portal resize hotkey"
        );
        assert!(
            app.state.portal_resize_states.is_empty(),
            "shell-reserved shortcut must not create portal resize state"
        );
    }

    #[test]
    fn ctrl_resize_hotkey_is_captured_by_safe_mode_before_resizing() {
        use std::sync::atomic::Ordering;

        use tze_hud_input::{FocusManager, InputProcessor, KeyboardModifiers};
        use tze_hud_scene::types::{HitRegionNode, TileScrollConfig};
        use tze_hud_scene::{Capability, Node, NodeData, Rect, SceneGraph, SceneId};

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "portal-agent",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let tile_id = scene
            .create_tile(
                tab_id,
                "portal-agent",
                lease_id,
                Rect::new(100.0, 100.0, 400.0, 300.0),
                1,
            )
            .unwrap();
        scene
            .register_tile_scroll_config(tile_id, TileScrollConfig::vertical())
            .unwrap();

        let composer_id = SceneId::new();
        scene
            .set_tile_root(
                tile_id,
                Node {
                    id: composer_id,
                    children: vec![],
                    data: NodeData::HitRegion(HitRegionNode {
                        bounds: Rect::new(0.0, 0.0, 400.0, 60.0),
                        interaction_id: "portal-composer".to_string(),
                        accepts_focus: true,
                        accepts_pointer: true,
                        accepts_composer_input: true,
                        ..Default::default()
                    }),
                },
            )
            .unwrap();

        let mut processor = InputProcessor::new();
        let mut focus_manager = FocusManager::new();
        let (_input_result, transition) = processor.process_with_focus(
            &tze_hud_input::PointerEvent {
                x: 110.0,
                y: 110.0,
                kind: tze_hud_input::PointerEventKind::Down,
                device_id: 1,
                timestamp: None,
            },
            &mut scene,
            &mut focus_manager,
            tab_id,
        );
        assert!(
            transition.and_then(|t| t.gained).is_some(),
            "pointer down must focus the composer node"
        );

        let (mut app, mut input_event_rx) =
            make_windowed_keyboard_test_app(scene, focus_manager, processor);
        app.state.safe_mode_atomic.store(true, Ordering::Release);

        let bounds = |app: &WinitApp| {
            let shared = app
                .state
                .shared_state
                .try_lock()
                .expect("shared state must be available during key dispatch test");
            let scene = shared
                .scene
                .try_lock()
                .expect("scene must be available during key dispatch test");
            scene.tiles.get(&tile_id).unwrap().bounds
        };

        let before = bounds(&app);
        app.dispatch_key_down_event(&RawKeyDownEvent {
            key_code: "Equal".to_string(),
            key: "=".to_string(),
            modifiers: KeyboardModifiers {
                ctrl: true,
                ..KeyboardModifiers::NONE
            },
            repeat: false,
            timestamp_mono_us: tze_hud_scene::MonoUs(1),
        });
        let after = bounds(&app);

        assert_eq!(
            after, before,
            "safe mode must capture Ctrl+= before portal resize can mutate bounds"
        );
        assert!(
            app.state.portal_resize_states.is_empty(),
            "safe-mode-captured resize hotkey must not create resize state"
        );
        assert!(
            input_event_rx.try_recv().is_err(),
            "safe-mode-captured resize hotkey must not be forwarded to the agent"
        );
    }

    // ── Pointer-affordance portal resize ─────────────────────────────────────

    /// A pointer-down on a resize affordance starts the gesture:
    ///   - `gesture_active()` becomes true.
    ///   - An outcome (snapshot) is returned.
    ///   - The snapshot's `gesture_active` flag is true.
    ///   - Tile bounds are unchanged (clamped initial = current).
    #[test]
    fn pointer_down_on_affordance_starts_gesture() {
        let (mut scene, tab_id, tile_id, fm) = portal_scene_with_focus();
        let mut resize_states = std::collections::HashMap::new();
        let display_w = 1920.0_f32;
        let display_h = 1080.0_f32;

        // Tile is at (100, 100, 400, 300). Affordance strip is 8px.
        // Pointer at (496, 250) is on the right edge (100+400-8 = 492 ≤ 496 ≤ 500).
        let event = PointerEvent {
            x: 496.0,
            y: 250.0,
            kind: PointerEventKind::Down,
            device_id: 1,
            timestamp: None,
        };

        let outcome = apply_portal_resize_pointer_event(
            &event,
            &mut resize_states,
            Some(tab_id),
            &fm,
            &mut scene,
            display_w,
            display_h,
            PortalWindowTokens::default(),
        );

        assert!(
            outcome.is_some(),
            "pointer-down on affordance must return an outcome"
        );
        let outcome = outcome.unwrap();
        assert_eq!(
            outcome.members[0].tile_id, tile_id,
            "outcome must reference the focused portal tile"
        );
        assert!(
            outcome.members[0].snapshot.gesture_active,
            "snapshot must have gesture_active=true on pointer-down"
        );

        // Resize state map must now have an entry with gesture_active=true.
        let resize_state = resize_states
            .get(&tile_id)
            .expect("resize state must be created on pointer-down");
        assert!(
            resize_state.gesture_active(),
            "gesture_active() must be true after pointer-down on affordance"
        );
    }

    /// A pointer-move during an active gesture applies local bounds immediately
    /// (local-first feedback), and the tile width grows as the pointer moves right.
    #[test]
    fn pointer_move_during_gesture_updates_tile_bounds_locally() {
        let (mut scene, tab_id, tile_id, fm) = portal_scene_with_focus();
        let mut resize_states = std::collections::HashMap::new();
        let display_w = 1920.0_f32;
        let display_h = 1080.0_f32;

        // Start gesture on the right edge.
        let down = PointerEvent {
            x: 496.0,
            y: 250.0,
            kind: PointerEventKind::Down,
            device_id: 1,
            timestamp: None,
        };
        apply_portal_resize_pointer_event(
            &down,
            &mut resize_states,
            Some(tab_id),
            &fm,
            &mut scene,
            display_w,
            display_h,
            PortalWindowTokens::default(),
        );

        let width_before = scene.tiles[&tile_id].bounds.width;

        // Move pointer 20px to the right → right edge should grow by 20px.
        let mv = PointerEvent {
            x: 516.0,
            y: 250.0,
            kind: PointerEventKind::Move,
            device_id: 1,
            timestamp: None,
        };
        let outcome = apply_portal_resize_pointer_event(
            &mv,
            &mut resize_states,
            Some(tab_id),
            &fm,
            &mut scene,
            display_w,
            display_h,
            PortalWindowTokens::default(),
        );

        assert!(
            outcome.is_some(),
            "pointer-move during gesture must return an outcome"
        );
        let new_width = scene.tiles[&tile_id].bounds.width;
        assert!(
            new_width > width_before,
            "tile width must grow when pointer moves right on right edge: before={width_before}, after={new_width}"
        );
    }

    /// A pointer-up ends the gesture and broadcasts the final geometry snapshot.
    ///   - After pointer-up, `gesture_active()` becomes false.
    ///   - The final snapshot's `gesture_active` is false.
    ///   - Tile bounds reflect the final clamped position.
    ///   - A geometry event outcome is returned (for broadcasting).
    #[test]
    fn pointer_up_ends_gesture_and_returns_geometry_event() {
        let (mut scene, tab_id, tile_id, fm) = portal_scene_with_focus();
        let mut resize_states = std::collections::HashMap::new();
        let display_w = 1920.0_f32;
        let display_h = 1080.0_f32;

        // Start gesture on the right edge.
        let down = PointerEvent {
            x: 496.0,
            y: 250.0,
            kind: PointerEventKind::Down,
            device_id: 1,
            timestamp: None,
        };
        apply_portal_resize_pointer_event(
            &down,
            &mut resize_states,
            Some(tab_id),
            &fm,
            &mut scene,
            display_w,
            display_h,
            PortalWindowTokens::default(),
        );

        // Move to establish a drag delta.
        let mv = PointerEvent {
            x: 530.0,
            y: 250.0,
            kind: PointerEventKind::Move,
            device_id: 1,
            timestamp: None,
        };
        apply_portal_resize_pointer_event(
            &mv,
            &mut resize_states,
            Some(tab_id),
            &fm,
            &mut scene,
            display_w,
            display_h,
            PortalWindowTokens::default(),
        );

        // Release pointer.
        let up = PointerEvent {
            x: 530.0,
            y: 250.0,
            kind: PointerEventKind::Up,
            device_id: 1,
            timestamp: None,
        };
        let outcome = apply_portal_resize_pointer_event(
            &up,
            &mut resize_states,
            Some(tab_id),
            &fm,
            &mut scene,
            display_w,
            display_h,
            PortalWindowTokens::default(),
        );

        assert!(
            outcome.is_some(),
            "pointer-up must return a geometry event outcome"
        );
        let outcome = outcome.unwrap();
        assert!(
            !outcome.members[0].snapshot.gesture_active,
            "snapshot gesture_active must be false after pointer-up"
        );
        assert_eq!(
            outcome.members[0].tile_id, tile_id,
            "outcome tile_id must match the resized portal"
        );

        // gesture_active() must be false after the last device lifts.
        let resize_state = resize_states
            .get(&tile_id)
            .expect("resize state must exist");
        assert!(
            !resize_state.gesture_active(),
            "gesture_active() must be false after pointer-up"
        );

        // Final tile width should reflect the drag delta (496→530 = +34px on right edge).
        let final_width = scene.tiles[&tile_id].bounds.width;
        assert!(
            final_width > 400.0,
            "tile width must be larger than initial 400px after rightward drag: {final_width}"
        );
    }

    /// Move/up routing must find the active resize gesture for the current
    /// `device_id`, not merely the first portal that has any active gesture.
    #[test]
    fn multi_device_resize_move_and_up_route_to_matching_device_gesture() {
        use tze_hud_scene::types::TileScrollConfig;
        use tze_hud_scene::{Capability, Rect};

        let (mut scene, tab_id, tile_a, mut fm) = portal_scene_with_focus();
        let lease_b = scene.grant_lease(
            "portal-agent-b",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let tile_b = scene
            .create_tile(
                tab_id,
                "portal-agent-b",
                lease_b,
                Rect::new(700.0, 100.0, 400.0, 300.0),
                2,
            )
            .unwrap();
        scene
            .register_tile_scroll_config(tile_b, TileScrollConfig::vertical())
            .unwrap();

        let mut resize_states = std::collections::HashMap::new();
        let display_w = 1920.0_f32;
        let display_h = 1080.0_f32;
        let tokens = PortalWindowTokens::default();

        let down_a = PointerEvent {
            x: 496.0,
            y: 250.0,
            kind: PointerEventKind::Down,
            device_id: 1,
            timestamp: None,
        };
        let outcome_a = apply_portal_resize_pointer_event(
            &down_a,
            &mut resize_states,
            Some(tab_id),
            &fm,
            &mut scene,
            display_w,
            display_h,
            tokens,
        )
        .expect("device 1 down must start a resize on tile A");
        assert_eq!(outcome_a.members[0].tile_id, tile_a);

        let (focus_result, _) = fm.request_focus(
            FocusRequest {
                tile_id: tile_b,
                node_id: None,
                steal: true,
                requesting_namespace: "portal-agent-b".to_string(),
            },
            tab_id,
            &scene,
        );
        assert_eq!(
            focus_result,
            tze_hud_input::FocusResult::Granted,
            "test setup must focus the second portal tile"
        );

        let down_b = PointerEvent {
            x: 1096.0,
            y: 250.0,
            kind: PointerEventKind::Down,
            device_id: 2,
            timestamp: None,
        };
        let outcome_b = apply_portal_resize_pointer_event(
            &down_b,
            &mut resize_states,
            Some(tab_id),
            &fm,
            &mut scene,
            display_w,
            display_h,
            tokens,
        )
        .expect("device 2 down must start a resize on tile B");
        assert_eq!(outcome_b.members[0].tile_id, tile_b);

        let active_order = resize_states
            .iter()
            .filter(|(_, state)| state.gesture_active())
            .map(|(&tile_id, _)| tile_id)
            .collect::<Vec<_>>();
        assert_eq!(
            active_order.len(),
            2,
            "test setup must have two concurrently active portal resize gestures"
        );

        let first_iterated_tile = active_order[0];
        let target_tile = active_order[1];
        let (target_device_id, target_left_x, other_tile) = if target_tile == tile_a {
            (1, 100.0, tile_b)
        } else {
            (2, 700.0, tile_a)
        };

        let target_width_before = scene.tiles[&target_tile].bounds.width;
        let other_width_before = scene.tiles[&other_tile].bounds.width;
        let mv = PointerEvent {
            x: target_left_x + 430.0,
            y: 250.0,
            kind: PointerEventKind::Move,
            device_id: target_device_id,
            timestamp: None,
        };
        let move_outcome = apply_portal_resize_pointer_event(
            &mv,
            &mut resize_states,
            Some(tab_id),
            &fm,
            &mut scene,
            display_w,
            display_h,
            tokens,
        )
        .expect("move must find the portal whose gesture belongs to this device");

        assert_eq!(
            move_outcome.members[0].tile_id, target_tile,
            "move must update the active portal for the current device"
        );
        assert!(
            scene.tiles[&target_tile].bounds.width > target_width_before,
            "target portal width must grow after moving its right-edge gesture"
        );
        assert_eq!(
            scene.tiles[&other_tile].bounds.width, other_width_before,
            "move for one device must not mutate the other active portal"
        );

        let up = PointerEvent {
            x: target_left_x + 430.0,
            y: 250.0,
            kind: PointerEventKind::Up,
            device_id: target_device_id,
            timestamp: None,
        };
        let up_outcome = apply_portal_resize_pointer_event(
            &up,
            &mut resize_states,
            Some(tab_id),
            &fm,
            &mut scene,
            display_w,
            display_h,
            tokens,
        )
        .expect("up must end the portal gesture for the current device");

        assert_eq!(
            up_outcome.members[0].tile_id, target_tile,
            "up must end the active portal for the current device"
        );
        assert!(
            !resize_states[&target_tile].gesture_active(),
            "target portal gesture must end after pointer-up"
        );
        assert!(
            resize_states[&first_iterated_tile].gesture_active(),
            "the other portal gesture must remain active"
        );
    }

    /// gesture_active() is true during the drag, making adapter publishes
    /// rejectable — the primary reason this code path must exist in production
    /// (hud-o0st9 acceptance criterion: gesture_active becomes reachable).
    #[test]
    fn gesture_active_is_true_during_pointer_drag() {
        let (mut scene, tab_id, tile_id, fm) = portal_scene_with_focus();
        let mut resize_states = std::collections::HashMap::new();
        let display_w = 1920.0_f32;
        let display_h = 1080.0_f32;

        // Before any gesture, no entry exists — gesture_active is implicitly false.
        assert!(
            resize_states
                .get(&tile_id)
                .is_none_or(|s: &PortalResizeState| !s.gesture_active()),
            "gesture must not be active before pointer-down"
        );

        let down = PointerEvent {
            x: 496.0,
            y: 250.0,
            kind: PointerEventKind::Down,
            device_id: 1,
            timestamp: None,
        };
        apply_portal_resize_pointer_event(
            &down,
            &mut resize_states,
            Some(tab_id),
            &fm,
            &mut scene,
            display_w,
            display_h,
            PortalWindowTokens::default(),
        );

        assert!(
            resize_states[&tile_id].gesture_active(),
            "gesture_active() must be true between pointer-down and pointer-up"
        );

        let up = PointerEvent {
            x: 510.0,
            y: 250.0,
            kind: PointerEventKind::Up,
            device_id: 1,
            timestamp: None,
        };
        apply_portal_resize_pointer_event(
            &up,
            &mut resize_states,
            Some(tab_id),
            &fm,
            &mut scene,
            display_w,
            display_h,
            PortalWindowTokens::default(),
        );

        assert!(
            !resize_states[&tile_id].gesture_active(),
            "gesture_active() must be false after pointer-up"
        );
    }

    /// A pointer-down outside any affordance (in the content area) must NOT
    /// start a gesture.
    #[test]
    fn pointer_down_in_content_area_does_not_start_gesture() {
        let (mut scene, tab_id, tile_id, fm) = portal_scene_with_focus();
        let mut resize_states = std::collections::HashMap::new();
        let display_w = 1920.0_f32;
        let display_h = 1080.0_f32;

        // Tile is at (100, 100, 400, 300). Content area center is at (300, 250).
        let event = PointerEvent {
            x: 300.0,
            y: 250.0,
            kind: PointerEventKind::Down,
            device_id: 1,
            timestamp: None,
        };

        let outcome = apply_portal_resize_pointer_event(
            &event,
            &mut resize_states,
            Some(tab_id),
            &fm,
            &mut scene,
            display_w,
            display_h,
            PortalWindowTokens::default(),
        );

        assert!(
            outcome.is_none(),
            "pointer-down in content area must not start a gesture"
        );
        assert!(
            resize_states
                .get(&tile_id)
                .is_none_or(|s| !s.gesture_active()),
            "no gesture must be active after content-area pointer-down"
        );
    }

    // ── Lease-bound maxima (hud-kgu8u / hud-zleu2) ───────────────────────────

    /// `compute_portal_max_dims` with no lease constraint (0.0) uses the full
    /// display dimensions, not `display_w - tile_x`.
    ///
    /// A portal at (100, 100) on a 1920×1080 display must be allowed to grow
    /// up to 1920×1080 — `PortalRect::clamped` shifts the origin as needed to
    /// keep the portal on-screen.
    #[test]
    fn compute_portal_max_dims_unconstrained_uses_display_boundary() {
        // 0.0 = unconstrained (no lease spatial budget)
        let (max_w, max_h) = compute_portal_max_dims(0.0, 0.0, 1920.0, 1080.0, 50.0, 50.0);

        // Display boundary: full display width/height, not display - tile.origin.
        assert_eq!(
            max_w, 1920.0,
            "unconstrained width must equal display_w (not display_w - tile_x)"
        );
        assert_eq!(
            max_h, 1080.0,
            "unconstrained height must equal display_h (not display_h - tile_y)"
        );
    }

    /// A portal at x=500 on a 1000px-wide display must be allowed to grow to
    /// 1000px wide.  `PortalRect::clamped` shifts the origin to x=0 so the
    /// right edge lands at the screen edge.  The pre-fix code produced 500px
    /// (display_w - tile.x), capping the portal at half the screen width.
    #[test]
    fn compute_portal_max_dims_uses_display_w_not_display_w_minus_tile_x() {
        // Scenario from PR #691 review: 1000px-wide display; the function must
        // return the full display width, not display_w - tile.x.
        let (max_w, max_h) = compute_portal_max_dims(0.0, 0.0, 1000.0, 800.0, 50.0, 50.0);

        assert_eq!(
            max_w, 1000.0,
            "portal at x=500 on 1000px display must be allowed to reach full display width (1000), \
             not be capped at display_w - tile.x = 500"
        );
        assert_eq!(
            max_h, 800.0,
            "portal at y=200 on 800px display must be allowed to reach full display height (800), \
             not be capped at display_h - tile.y = 600"
        );
    }

    /// When the lease budget is tighter than the display boundary, the lease
    /// budget wins (most-restrictive policy).
    #[test]
    fn compute_portal_max_dims_lease_budget_is_more_restrictive() {
        let (max_w, max_h) = compute_portal_max_dims(500.0, 400.0, 1920.0, 1080.0, 50.0, 50.0);

        assert_eq!(
            max_w, 500.0,
            "lease budget (500) must win over display boundary (1920)"
        );
        assert_eq!(
            max_h, 400.0,
            "lease budget (400) must win over display boundary (1080)"
        );
    }

    /// When the display boundary is tighter than the lease budget, the display
    /// boundary wins.
    #[test]
    fn compute_portal_max_dims_display_boundary_is_more_restrictive() {
        // Lease budget larger than the display; display boundary must be the
        // binding constraint.
        let (max_w, max_h) = compute_portal_max_dims(5000.0, 5000.0, 1920.0, 1080.0, 50.0, 50.0);

        // Display boundary is the full display, not display - tile.origin.
        assert_eq!(
            max_w, 1920.0,
            "display boundary (1920) must win over lease budget (5000)"
        );
        assert_eq!(
            max_h, 1080.0,
            "display boundary (1080) must win over lease budget (5000)"
        );
    }

    /// A lease budget smaller than the token minimum is floored to the minimum
    /// so a portal is always growable to at least the token minimum.
    #[test]
    fn compute_portal_max_dims_lease_budget_floored_to_token_minimum() {
        let (max_w, max_h) = compute_portal_max_dims(10.0, 10.0, 1920.0, 1080.0, 50.0, 50.0);

        assert_eq!(
            max_w, 50.0,
            "budget smaller than min_width must be floored to min_width"
        );
        assert_eq!(
            max_h, 50.0,
            "budget smaller than min_height must be floored to min_height"
        );
    }

    /// Portal resize gesture respects the lease-bound maximum:
    /// dragging the right edge beyond the lease limit is clamped to the limit.
    #[test]
    fn pointer_drag_respects_lease_bound_maximum() {
        use tze_hud_input::FocusManager;
        use tze_hud_scene::{Capability, Rect, SceneGraph};

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "portal-agent",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        // Tile at (100, 100, 400, 300) — starts at 400×300.
        let tile_id = scene
            .create_tile(
                tab_id,
                "portal-agent",
                lease_id,
                Rect::new(100.0, 100.0, 400.0, 300.0),
                1,
            )
            .unwrap();
        // Set a tight lease budget on the authoritative lease entry:
        // max 450 wide, 350 tall.
        if let Some(lease) = scene.leases.get_mut(&lease_id) {
            lease.spatial_budget.max_tile_width_px = 450.0;
            lease.spatial_budget.max_tile_height_px = 350.0;
        }
        scene
            .register_tile_scroll_config(
                tile_id,
                tze_hud_scene::types::TileScrollConfig::vertical(),
            )
            .unwrap();

        let mut fm = FocusManager::new();
        fm.request_focus(
            tze_hud_input::FocusRequest {
                tile_id,
                node_id: None,
                steal: true,
                requesting_namespace: "portal-agent".to_string(),
            },
            tab_id,
            &scene,
        );

        let mut resize_states = std::collections::HashMap::new();
        let display_w = 1920.0_f32;
        let display_h = 1080.0_f32;

        // Start gesture on right edge (100+400-8=492 ≤ 496 ≤ 500).
        let down = PointerEvent {
            x: 496.0,
            y: 250.0,
            kind: PointerEventKind::Down,
            device_id: 1,
            timestamp: None,
        };
        apply_portal_resize_pointer_event(
            &down,
            &mut resize_states,
            Some(tab_id),
            &fm,
            &mut scene,
            display_w,
            display_h,
            PortalWindowTokens::default(),
        );

        // Drag far to the right — would grow the tile to >> 450px without a lease clamp.
        let mv = PointerEvent {
            x: 800.0,
            y: 250.0,
            kind: PointerEventKind::Move,
            device_id: 1,
            timestamp: None,
        };
        apply_portal_resize_pointer_event(
            &mv,
            &mut resize_states,
            Some(tab_id),
            &fm,
            &mut scene,
            display_w,
            display_h,
            PortalWindowTokens::default(),
        );

        let width_after_drag = scene.tiles[&tile_id].bounds.width;
        assert!(
            width_after_drag <= 450.0,
            "tile width must be clamped to lease budget 450.0, got {width_after_drag}"
        );
    }

    // ── portal_resize_states map pruning (hud-kgu8u) ─────────────────────────

    /// `prune_portal_resize_states` removes the entry for a tile that was
    /// deleted from the scene. Verifies: entry present before deletion,
    /// entry absent after prune.
    ///
    /// This test drives `prune_portal_resize_states` directly via a thin
    /// harness because the method requires a `WinitApp` (holds scene + map).
    /// We use the existing scene helpers and a manually constructed state to
    /// avoid spinning up a full winit event loop.
    #[test]
    fn portal_resize_state_pruned_after_tile_removal() {
        use tze_hud_scene::{Capability, Rect, SceneGraph};

        // Build a minimal scene with one portal tile.
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "portal-agent",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let tile_id = scene
            .create_tile(
                tab_id,
                "portal-agent",
                lease_id,
                Rect::new(100.0, 100.0, 400.0, 300.0),
                1,
            )
            .unwrap();

        // Simulate a resize-state entry that accumulated for this tile.
        let mut portal_resize_states: std::collections::HashMap<
            tze_hud_scene::SceneId,
            PortalResizeState,
        > = std::collections::HashMap::new();
        portal_resize_states.insert(
            tile_id,
            PortalResizeState::new(tile_id.as_uuid().as_u128() as u64),
        );
        assert!(
            portal_resize_states.contains_key(&tile_id),
            "entry must be present before tile removal"
        );

        // Remove the tile from the scene (simulates DeleteTile mutation).
        scene.tiles.remove(&tile_id);
        assert!(
            !scene.tiles.contains_key(&tile_id),
            "tile must be absent from scene after removal"
        );

        // Prune: entries for absent tiles must be removed.
        let before = portal_resize_states.len();
        portal_resize_states.retain(|id, _| scene.tiles.contains_key(id));
        let removed = before - portal_resize_states.len();

        assert_eq!(removed, 1, "exactly one stale entry must be pruned");
        assert!(
            !portal_resize_states.contains_key(&tile_id),
            "entry for removed tile must be gone after pruning"
        );
    }

    /// `prune_portal_resize_states` preserves entries for tiles that still
    /// exist in the scene.
    #[test]
    fn portal_resize_state_preserved_for_live_tiles() {
        use tze_hud_scene::{Capability, Rect, SceneGraph};

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "portal-agent",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let tile_a = scene
            .create_tile(
                tab_id,
                "portal-agent",
                lease_id,
                Rect::new(0.0, 0.0, 300.0, 200.0),
                1,
            )
            .unwrap();
        let tile_b = scene
            .create_tile(
                tab_id,
                "portal-agent",
                lease_id,
                Rect::new(400.0, 0.0, 300.0, 200.0),
                2,
            )
            .unwrap();

        let mut portal_resize_states: std::collections::HashMap<
            tze_hud_scene::SceneId,
            PortalResizeState,
        > = std::collections::HashMap::new();
        portal_resize_states.insert(
            tile_a,
            PortalResizeState::new(tile_a.as_uuid().as_u128() as u64),
        );
        portal_resize_states.insert(
            tile_b,
            PortalResizeState::new(tile_b.as_uuid().as_u128() as u64),
        );

        // Remove only tile_b from the scene.
        scene.tiles.remove(&tile_b);

        // Prune.
        portal_resize_states.retain(|id, _| scene.tiles.contains_key(id));

        assert!(
            portal_resize_states.contains_key(&tile_a),
            "entry for still-live tile_a must be retained"
        );
        assert!(
            !portal_resize_states.contains_key(&tile_b),
            "entry for removed tile_b must be pruned"
        );
        assert_eq!(
            portal_resize_states.len(),
            1,
            "exactly one entry (tile_a) must remain"
        );
    }

    /// Verifies the eager drain-based `portal_resize_states` cleanup path
    /// introduced by hud-4tuw5.
    ///
    /// `SceneGraph::drain_removed_tile_ids` yields the IDs of tiles removed via
    /// `remove_tile_and_nodes`; `prune_portal_resize_states` then removes each
    /// returned ID from the map.  This test drives that contract directly
    /// without the windowed event loop, matching the style of the existing
    /// sweep-based prune tests above.
    ///
    /// The `remove_tile_and_nodes` → drain queue half of the contract is
    /// exercised by `portal_resize_drain_queue_populated_by_remove_tile` in
    /// `tze_hud_scene` (where the function is visible).
    #[test]
    fn portal_resize_state_pruned_via_drain_queue_on_tile_removal() {
        use tze_hud_scene::{Capability, Rect, SceneGraph};

        // Build a minimal scene with one portal tile.
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "portal-agent",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let tile_id = scene
            .create_tile(
                tab_id,
                "portal-agent",
                lease_id,
                Rect::new(100.0, 100.0, 400.0, 300.0),
                1,
            )
            .unwrap();

        // Simulate a resize-state entry accumulated for this portal tile.
        let mut portal_resize_states: std::collections::HashMap<
            tze_hud_scene::SceneId,
            PortalResizeState,
        > = std::collections::HashMap::new();
        portal_resize_states.insert(
            tile_id,
            PortalResizeState::new(tile_id.as_uuid().as_u128() as u64),
        );
        assert!(
            portal_resize_states.contains_key(&tile_id),
            "entry must be present before tile removal"
        );

        // Sanity: drain queue is empty before any removal.
        assert!(
            scene.drain_removed_tile_ids().is_empty(),
            "drain queue must be empty before any tile removal"
        );

        // Simulate what remove_tile_and_nodes does: remove from tiles map and
        // enqueue in recently_removed_tile_ids.  (remove_tile_and_nodes is
        // pub(crate) in tze_hud_scene; we cannot call it directly here.)
        scene.tiles.remove(&tile_id);
        scene.overlay.recently_removed_tile_ids.push(tile_id);

        // Drain the removal queue — this mirrors what prune_portal_resize_states does.
        let removed_ids = scene.drain_removed_tile_ids();
        assert_eq!(
            removed_ids,
            vec![tile_id],
            "drain queue must yield exactly the removed tile ID"
        );

        // Apply the drain to the portal_resize_states map.
        for id in &removed_ids {
            portal_resize_states.remove(id);
        }

        assert!(
            !portal_resize_states.contains_key(&tile_id),
            "portal_resize_states entry must be gone after drain-based pruning (hud-4tuw5)"
        );
        assert!(
            portal_resize_states.is_empty(),
            "no stale entries must remain after drain"
        );

        // Confirm the drain queue is now empty (idempotent).
        assert!(
            scene.drain_removed_tile_ids().is_empty(),
            "drain queue must be empty after drain"
        );
    }

    // ── scene.version increment on pointer-drag resize (hud-g1hij) ───────────

    /// A pointer-drag resize (on_pointer_move → GestureUpdate) increments
    /// `scene.version` exactly once when the tile bounds change, matching the
    /// hotkey resize path (hud-ghhxa / hud-g1hij).
    ///
    /// Regression test — the 20Hz re-prime cadence gate in
    /// `prime_truncation_cache` is keyed on `scene.version`; if the drag path
    /// did NOT bump it, mid-drag re-truncation would never fire.
    ///
    /// Also verifies that a pointer-move at the identical position (no size
    /// delta) does NOT bump the version a second time, preventing spurious
    /// cache invalidations when the gesture is clamped at a boundary.
    #[test]
    fn drag_resize_pointer_move_bumps_scene_version_on_size_change() {
        let (mut scene, tab_id, _tile_id, fm) = portal_scene_with_focus();
        let mut resize_states = std::collections::HashMap::new();
        let display_w = 1920.0_f32;
        let display_h = 1080.0_f32;

        // Pointer-down on the right-edge affordance (x=496 hits the 8px strip
        // of a tile whose right edge is at 100+400=500).  No version bump expected
        // on gesture start (clamped initial rect is identical to current rect).
        let version_before_down = scene.version;
        let down = PointerEvent {
            x: 496.0,
            y: 250.0,
            kind: PointerEventKind::Down,
            device_id: 1,
            timestamp: None,
        };
        apply_portal_resize_pointer_event(
            &down,
            &mut resize_states,
            Some(tab_id),
            &fm,
            &mut scene,
            display_w,
            display_h,
            PortalWindowTokens::default(),
        );
        assert_eq!(
            scene.version, version_before_down,
            "scene.version must NOT change on gesture start (bounds unchanged at down)"
        );

        // Pointer-move 20px right → right edge grows, size changes.
        // scene.version must increment exactly once.
        let version_before_move = scene.version;
        let mv_grow = PointerEvent {
            x: 516.0,
            y: 250.0,
            kind: PointerEventKind::Move,
            device_id: 1,
            timestamp: None,
        };
        apply_portal_resize_pointer_event(
            &mv_grow,
            &mut resize_states,
            Some(tab_id),
            &fm,
            &mut scene,
            display_w,
            display_h,
            PortalWindowTokens::default(),
        );
        assert_eq!(
            scene.version,
            version_before_move + 1,
            "scene.version must increment exactly once when drag resize changes tile size"
        );

        // Pointer-move at the exact same position (no delta) — the tile size
        // is already at the value from the previous move, so `size_changed` is
        // false and the version must NOT advance again.
        let version_before_noop = scene.version;
        let mv_noop = PointerEvent {
            x: 516.0,
            y: 250.0,
            kind: PointerEventKind::Move,
            device_id: 1,
            timestamp: None,
        };
        apply_portal_resize_pointer_event(
            &mv_noop,
            &mut resize_states,
            Some(tab_id),
            &fm,
            &mut scene,
            display_w,
            display_h,
            PortalWindowTokens::default(),
        );
        assert_eq!(
            scene.version, version_before_noop,
            "scene.version must NOT change when pointer-move produces no size delta"
        );
    }

    // ── Whole-portal resize (hud-fb3en) ──────────────────────────────────────

    /// Build a multi-surface portal (frame + transcript pane + composer pane +
    /// far-corner drag shield) sharing one lease, with keyboard focus on the
    /// composer/input pane — the exact live configuration that made Ctrl resize
    /// scale only the focused pane. Returns (scene, tab, frame, transcript,
    /// composer, drag_shield, focus_manager).
    #[allow(clippy::type_complexity)]
    fn multi_surface_portal_scene() -> (
        tze_hud_scene::graph::SceneGraph,
        tze_hud_scene::SceneId,
        tze_hud_scene::SceneId,
        tze_hud_scene::SceneId,
        tze_hud_scene::SceneId,
        tze_hud_scene::SceneId,
        FocusManager,
    ) {
        use tze_hud_scene::types::TileScrollConfig;
        use tze_hud_scene::{Capability, Rect, SceneGraph};

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "portal-agent",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        // Frame is the portal-sized anchor (largest area, not scrollable).
        let frame_id = scene
            .create_tile(
                tab_id,
                "portal-agent",
                lease_id,
                Rect::new(100.0, 100.0, 400.0, 300.0),
                1,
            )
            .unwrap();
        // Transcript (output) pane — a scrollable surface inside the frame.
        let transcript_id = scene
            .create_tile(
                tab_id,
                "portal-agent",
                lease_id,
                Rect::new(110.0, 110.0, 180.0, 280.0),
                3,
            )
            .unwrap();
        // Composer (input) pane — a scrollable surface inside the frame.
        let composer_id = scene
            .create_tile(
                tab_id,
                "portal-agent",
                lease_id,
                Rect::new(300.0, 110.0, 190.0, 280.0),
                3,
            )
            .unwrap();
        // Drag shield — parked in the far display corner, NOT scrollable, NOT
        // spatially part of the portal frame.
        let shield_id = scene
            .create_tile(
                tab_id,
                "portal-agent",
                lease_id,
                Rect::new(1900.0, 1070.0, 1.0, 1.0),
                20,
            )
            .unwrap();
        scene
            .register_tile_scroll_config(transcript_id, TileScrollConfig::vertical())
            .unwrap();
        scene
            .register_tile_scroll_config(composer_id, TileScrollConfig::vertical())
            .unwrap();

        // Focus the composer/input pane — the live trigger for the bug.
        let mut fm = FocusManager::new();
        fm.request_focus(
            FocusRequest {
                tile_id: composer_id,
                node_id: None,
                steal: true,
                requesting_namespace: "portal-agent".to_string(),
            },
            tab_id,
            &scene,
        );

        (
            scene,
            tab_id,
            frame_id,
            transcript_id,
            composer_id,
            shield_id,
            fm,
        )
    }

    /// Relative geometry of a surface `b` expressed as fractions of the frame
    /// `f`: (x offset, y offset, width, height) all divided by the frame rect.
    /// Two surfaces share a portal layout iff these tuples are (approx) equal
    /// before and after a resize.
    fn rel_to_frame(b: tze_hud_scene::Rect, f: tze_hud_scene::Rect) -> (f32, f32, f32, f32) {
        (
            (b.x - f.x) / f.width,
            (b.y - f.y) / f.height,
            b.width / f.width,
            b.height / f.height,
        )
    }

    fn approx_tuple(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-3
            && (a.1 - b.1).abs() < 1e-3
            && (a.2 - b.2).abs() < 1e-3
            && (a.3 - b.3).abs() < 1e-3
    }

    /// Core hud-fb3en regression: focusing the composer and pressing Ctrl+= must
    /// grow the WHOLE portal — every constituent surface scales together,
    /// preserving relative layout, anchored top-left — not just the focused
    /// pane. The far-corner drag shield must NOT move.
    #[test]
    fn ctrl_resize_hotkey_scales_whole_portal_not_just_focused_surface() {
        use tze_hud_input::{HotkeyResizeDir, InputProcessor};

        let (scene, tab_id, frame_id, transcript_id, composer_id, shield_id, fm) =
            multi_surface_portal_scene();
        let (mut app, _rx) = make_windowed_keyboard_test_app(scene, fm, InputProcessor::new());

        let read = |app: &WinitApp, id: tze_hud_scene::SceneId| {
            let shared = app.state.shared_state.try_lock().unwrap();
            let scene = shared.scene.try_lock().unwrap();
            scene.tiles.get(&id).unwrap().bounds
        };

        let frame_before = read(&app, frame_id);
        let composer_before = read(&app, composer_id);
        let transcript_before = read(&app, transcript_id);
        let shield_before = read(&app, shield_id);

        let consumed = app.apply_portal_resize_hotkey(tab_id, HotkeyResizeDir::Grow);
        assert!(
            consumed,
            "Ctrl resize hotkey must be consumed for a focused portal surface"
        );

        let frame_after = read(&app, frame_id);
        let composer_after = read(&app, composer_id);
        let transcript_after = read(&app, transcript_id);
        let shield_after = read(&app, shield_id);

        // Frame (anchor) grew, top-left anchored (origin fixed).
        assert!(
            frame_after.width > frame_before.width && frame_after.height > frame_before.height,
            "the portal frame must grow"
        );
        assert_eq!(
            (frame_after.x, frame_after.y),
            (frame_before.x, frame_before.y),
            "grow must be anchored at the frame's top-left corner"
        );

        // Both panes scaled with the portal — not left in place, not resized in
        // isolation.
        assert!(
            composer_after.width > composer_before.width
                && composer_after.height > composer_before.height,
            "the focused composer pane must scale WITH the whole portal"
        );
        assert!(
            transcript_after.width > transcript_before.width
                && transcript_after.height > transcript_before.height,
            "the transcript pane must scale WITH the whole portal"
        );

        // Relative layout preserved for every constituent surface.
        assert!(
            approx_tuple(
                rel_to_frame(composer_before, frame_before),
                rel_to_frame(composer_after, frame_after)
            ),
            "composer must keep its relative position/size within the portal"
        );
        assert!(
            approx_tuple(
                rel_to_frame(transcript_before, frame_before),
                rel_to_frame(transcript_after, frame_after)
            ),
            "transcript must keep its relative position/size within the portal"
        );

        // The spatially-detached drag shield is not a spatial member of the
        // portal frame and must not scale or move.
        assert_eq!(
            shield_after, shield_before,
            "the far-corner drag shield must not scale/move with a portal resize"
        );
    }

    /// A non-portal (non-scrollable) focused surface must not consume the resize
    /// hotkey nor change any geometry — the whole-portal path only engages for a
    /// focused portal surface.
    #[test]
    fn ctrl_resize_hotkey_ignored_when_focused_surface_is_not_a_portal() {
        use tze_hud_input::{HotkeyResizeDir, InputProcessor};

        let (scene, tab_id, frame_id, _transcript_id, _composer_id, _shield_id, _fm) =
            multi_surface_portal_scene();

        // Focus the frame, which is NOT a scrollable portal surface.
        let mut fm = FocusManager::new();
        fm.request_focus(
            FocusRequest {
                tile_id: frame_id,
                node_id: None,
                steal: true,
                requesting_namespace: "portal-agent".to_string(),
            },
            tab_id,
            &scene,
        );

        let (mut app, _rx) = make_windowed_keyboard_test_app(scene, fm, InputProcessor::new());

        let read = |app: &WinitApp, id: tze_hud_scene::SceneId| {
            let shared = app.state.shared_state.try_lock().unwrap();
            let scene = shared.scene.try_lock().unwrap();
            scene.tiles.get(&id).unwrap().bounds
        };
        let frame_before = read(&app, frame_id);

        let consumed = app.apply_portal_resize_hotkey(tab_id, HotkeyResizeDir::Grow);
        assert!(
            !consumed,
            "resize hotkey must not be consumed when the focused surface is not a portal"
        );
        assert_eq!(
            read(&app, frame_id),
            frame_before,
            "geometry must not change when the focused surface is not a portal"
        );
    }

    /// Pointer-affordance resize (dragging the frame's bottom-right corner) must
    /// also scale the WHOLE portal: the returned member set covers the frame and
    /// both panes but excludes the far-corner drag shield, and every member
    /// scales together while the shield stays put.
    #[test]
    fn pointer_affordance_resize_scales_whole_portal_from_frame() {
        use tze_hud_input::{
            PointerEvent, PointerEventKind, PortalResizeState, PortalWindowTokens,
        };

        let (mut scene, tab_id, frame_id, transcript_id, composer_id, shield_id, fm) =
            multi_surface_portal_scene();
        let mut states: std::collections::HashMap<tze_hud_scene::SceneId, PortalResizeState> =
            std::collections::HashMap::new();
        let tokens = PortalWindowTokens::default();
        let (display_w, display_h) = (1920.0_f32, 1080.0_f32);

        let read = |scene: &tze_hud_scene::graph::SceneGraph, id: tze_hud_scene::SceneId| {
            scene.tiles.get(&id).unwrap().bounds
        };
        let frame_before = read(&scene, frame_id);
        let composer_before = read(&scene, composer_id);
        let transcript_before = read(&scene, transcript_id);
        let shield_before = read(&scene, shield_id);

        // PointerDown on the frame's bottom-right corner affordance (500, 400).
        let down = PointerEvent {
            x: 498.0,
            y: 398.0,
            kind: PointerEventKind::Down,
            device_id: 0,
            timestamp: None,
        };
        let out_down = apply_portal_resize_pointer_event(
            &down,
            &mut states,
            Some(tab_id),
            &fm,
            &mut scene,
            display_w,
            display_h,
            tokens,
        );
        assert!(
            out_down.is_some(),
            "pointer-down on the frame resize affordance must start a whole-portal gesture even with the composer focused"
        );

        // PointerMove outward to grow the portal.
        let mv = PointerEvent {
            x: 570.0,
            y: 470.0,
            kind: PointerEventKind::Move,
            device_id: 0,
            timestamp: None,
        };
        let out_move = apply_portal_resize_pointer_event(
            &mv,
            &mut states,
            Some(tab_id),
            &fm,
            &mut scene,
            display_w,
            display_h,
            tokens,
        )
        .expect("pointer-move must update the whole portal");

        let member_ids: Vec<_> = out_move.members.iter().map(|m| m.tile_id).collect();
        assert!(
            member_ids.contains(&frame_id)
                && member_ids.contains(&composer_id)
                && member_ids.contains(&transcript_id),
            "whole-portal resize members must include the frame and both panes"
        );
        assert!(
            !member_ids.contains(&shield_id),
            "the far-corner drag shield must be excluded from resize members"
        );

        let frame_after = read(&scene, frame_id);
        let composer_after = read(&scene, composer_id);
        let transcript_after = read(&scene, transcript_id);
        let shield_after = read(&scene, shield_id);

        assert!(
            frame_after.width > frame_before.width && frame_after.height > frame_before.height,
            "pointer resize must grow the frame"
        );
        assert!(
            composer_after.width > composer_before.width,
            "pointer resize must scale the composer pane with the portal"
        );
        assert!(
            transcript_after.width > transcript_before.width,
            "pointer resize must scale the transcript pane with the portal"
        );
        assert!(
            approx_tuple(
                rel_to_frame(composer_before, frame_before),
                rel_to_frame(composer_after, frame_after)
            ),
            "pointer resize must preserve the composer's relative layout"
        );
        assert_eq!(
            shield_after, shield_before,
            "pointer resize must not move the far-corner drag shield"
        );

        // End the gesture cleanly.
        let up = PointerEvent {
            x: 570.0,
            y: 470.0,
            kind: PointerEventKind::Up,
            device_id: 0,
            timestamp: None,
        };
        apply_portal_resize_pointer_event(
            &up,
            &mut states,
            Some(tab_id),
            &fm,
            &mut scene,
            display_w,
            display_h,
            tokens,
        );
    }

    /// Grow the whole portal group that owns `frame_id` to `new_rect`, mirroring
    /// what a viewer resize gesture commits. Returns after the members have been
    /// scaled and viewer-geometry-locked.
    fn resize_group_to(
        scene: &mut tze_hud_scene::graph::SceneGraph,
        frame_id: tze_hud_scene::SceneId,
        new_rect: tze_hud_input::PortalRect,
    ) {
        let group = resolve_portal_group(scene, frame_id).expect("group must resolve");
        let old_rect = group.portal_rect;
        let snapshot = tze_hud_input::GeometrySnapshot {
            portal_id_hash: group.portal_id_hash,
            rect: new_rect,
            gesture_active: false,
            sequence: 1,
        };
        commit_portal_group_resize(scene, &group, old_rect, snapshot);
    }

    /// hud-rpmwt core: after a whole-portal resize the transcript/composer text
    /// must re-resolve to the NEW pane geometry, not stay wrapped at the
    /// attach-time width. The compositor wraps `TextMarkdownNode` text to the
    /// node's own `bounds.width` (tile-local, see
    /// `TextItem::from_text_markdown_node` / `from_text_markdown_cached`), so a
    /// resize that scales only `tile.bounds` — leaving the node tree stale —
    /// leaves the text wrapped at the old column: "resize works but the text
    /// isn't being resized". Assert both the node bounds AND the resulting
    /// `TextItem` layout width track the resized pane, at the draw-item seam.
    #[test]
    fn whole_portal_resize_reflows_transcript_text_to_new_pane_width() {
        use tze_hud_compositor::TextItem;
        use tze_hud_scene::SceneId;
        use tze_hud_scene::types::{
            FontFamily, Node, NodeData, Rect, Rgba, TextAlign, TextMarkdownNode, TextOverflow,
        };

        let (mut scene, _tab_id, frame_id, transcript_id, _composer_id, _shield_id, _fm) =
            multi_surface_portal_scene();

        // Attach a wrapping TextMarkdown node to the transcript pane. Node bounds
        // are tile-local: fill the transcript tile (180 x 280 from the fixture).
        let text_id = SceneId::new();
        scene.nodes.insert(
            text_id,
            Node {
                id: text_id,
                children: vec![],
                data: NodeData::TextMarkdown(TextMarkdownNode {
                    content: "the quick brown fox jumps over the lazy dog again and again"
                        .to_owned(),
                    bounds: Rect::new(0.0, 0.0, 180.0, 280.0),
                    font_size_px: 14.0,
                    font_family: FontFamily::SystemSansSerif,
                    color: Rgba::new(1.0, 1.0, 1.0, 1.0),
                    background: None,
                    alignment: TextAlign::Start,
                    overflow: TextOverflow::Clip,
                    color_runs: Box::default(),
                }),
            },
        );
        scene.tiles.get_mut(&transcript_id).unwrap().root_node = Some(text_id);

        let node_width = |scene: &tze_hud_scene::graph::SceneGraph| match &scene
            .nodes
            .get(&text_id)
            .unwrap()
            .data
        {
            NodeData::TextMarkdown(tm) => tm.bounds.width,
            other => panic!("expected TextMarkdown, got {other:?}"),
        };
        // Wrap column the compositor would use, before the resize.
        let layout_width_before = {
            let NodeData::TextMarkdown(tm) = &scene.nodes.get(&text_id).unwrap().data else {
                unreachable!()
            };
            TextItem::from_text_markdown_node(tm, 0.0, 0.0).bounds_width
        };
        let node_width_before = node_width(&scene);
        let transcript_before = scene.tiles.get(&transcript_id).unwrap().bounds;

        // Grow the whole portal ~1.5x in width, ~1.3x in height.
        resize_group_to(
            &mut scene,
            frame_id,
            tze_hud_input::PortalRect {
                x: 100.0,
                y: 100.0,
                width: 600.0,
                height: 390.0,
            },
        );

        let transcript_after = scene.tiles.get(&transcript_id).unwrap().bounds;
        assert!(
            transcript_after.width > transcript_before.width,
            "precondition: the transcript tile must have grown"
        );

        // 1) The node tree re-resolved: node width scaled with its tile.
        let node_width_after = node_width(&scene);
        let tile_ratio = transcript_after.width / transcript_before.width;
        let expected_node_width = node_width_before * tile_ratio;
        assert!(
            (node_width_after - expected_node_width).abs() < 1e-2,
            "transcript text node width must scale with the pane: expected \
             ~{expected_node_width}, got {node_width_after}"
        );

        // 2) The draw-item wrap column tracks the new pane — the seam the
        //    compositor shapes/wraps against. Before the fix this stayed pinned
        //    to the attach-time width.
        let layout_width_after = {
            let NodeData::TextMarkdown(tm) = &scene.nodes.get(&text_id).unwrap().data else {
                unreachable!()
            };
            TextItem::from_text_markdown_node(tm, 0.0, 0.0).bounds_width
        };
        assert!(
            layout_width_after > layout_width_before + 1.0,
            "TextItem layout/wrap width must grow with the resized pane: \
             before={layout_width_before}, after={layout_width_after}"
        );

        // 3) The wrap column stays within the resized pane — no layout to a
        //    width wider than the tile (overflow contract: no partially clipped
        //    glyphs from a stale-wide layout).
        assert!(
            layout_width_after <= transcript_after.width + 1e-3,
            "wrap width {layout_width_after} must not exceed the resized pane \
             width {}",
            transcript_after.width
        );
    }

    /// hud-lyqun core: dragging one constituent surface of a text-stream portal
    /// must translate the WHOLE portal by the same delta — every member moves
    /// together preserving relative layout, the far-corner drag shield stays
    /// parked, and every moved member takes viewer geometry authority.
    #[test]
    fn drag_move_translates_whole_portal_group_coherently() {
        let (mut scene, _tab_id, frame_id, transcript_id, composer_id, shield_id, _fm) =
            multi_surface_portal_scene();

        let read =
            |scene: &tze_hud_scene::graph::SceneGraph, id| scene.tiles.get(&id).unwrap().bounds;
        let frame_before = read(&scene, frame_id);
        let transcript_before = read(&scene, transcript_id);
        let composer_before = read(&scene, composer_id);
        let shield_before = read(&scene, shield_id);

        let (dx, dy) = (140.0_f32, -35.0_f32);
        let translated = translate_portal_group_on_drag(&mut scene, frame_id, dx, dy);
        assert!(
            translated,
            "dragging a portal surface must engage the whole-portal translate path"
        );

        let frame_after = read(&scene, frame_id);
        let transcript_after = read(&scene, transcript_id);
        let composer_after = read(&scene, composer_id);
        let shield_after = read(&scene, shield_id);

        // Every constituent surface moved by exactly the drag delta.
        for (before, after, name) in [
            (frame_before, frame_after, "frame"),
            (transcript_before, transcript_after, "transcript"),
            (composer_before, composer_after, "composer"),
        ] {
            assert!(
                (after.x - (before.x + dx)).abs() < 1e-3
                    && (after.y - (before.y + dy)).abs() < 1e-3,
                "{name} must translate by the drag delta"
            );
            assert!(
                (after.width - before.width).abs() < 1e-3
                    && (after.height - before.height).abs() < 1e-3,
                "{name} size must not change on a move"
            );
        }

        // Relative layout preserved for every member.
        assert!(approx_tuple(
            rel_to_frame(transcript_before, frame_before),
            rel_to_frame(transcript_after, frame_after)
        ));
        assert!(approx_tuple(
            rel_to_frame(composer_before, frame_before),
            rel_to_frame(composer_after, frame_after)
        ));

        // The far-corner drag shield is not a spatial member and stays put.
        assert_eq!(
            shield_after, shield_before,
            "the far-corner drag shield must not move with a portal drag"
        );

        // Every moved member now holds viewer geometry authority.
        for id in [frame_id, transcript_id, composer_id] {
            assert!(
                scene.is_viewer_geometry_locked(id),
                "each dragged portal member must take viewer geometry authority"
            );
        }
        assert!(
            !scene.is_viewer_geometry_locked(shield_id),
            "the untouched drag shield must not be locked"
        );
    }

    /// hud-uyhpn core: a multi-frame drag-move is a POSITION-ONLY mutation. It
    /// must NOT bump `scene.version` (the sentinel the compositor's markdown /
    /// truncation caches gate on) — otherwise every pointer delta invalidates the
    /// content-shaped caches and forces a full re-hash / re-shape per frame, the
    /// low-fps / flickery drag observed live. Instead each frame advances
    /// `scene.geometry_epoch` so the present-gate still repaints the moved portal.
    ///
    /// Baseline (pre-fix) over a K-frame drag: `scene.version` advanced K times →
    /// K content-cache re-primes. Fixed: `scene.version` advances 0 times → ZERO
    /// re-primes, while position updates every frame (`geometry_epoch` +K, bounds
    /// move each frame).
    #[test]
    fn drag_move_is_position_only_no_version_bump_repaints_each_frame() {
        let (mut scene, _tab_id, frame_id, transcript_id, composer_id, _shield_id, _fm) =
            multi_surface_portal_scene();

        let read =
            |scene: &tze_hud_scene::graph::SceneGraph, id| scene.tiles.get(&id).unwrap().bounds;

        // Settle to a known baseline as if content had just been committed and the
        // compositor primed its caches at this version.
        let version_at_drag_start = scene.version;
        let epoch_at_drag_start = scene.geometry_epoch;

        // Simulate a K-frame pointer drag: many small deltas, one per frame.
        const FRAMES: usize = 30;
        let (dx, dy) = (3.0_f32, -2.0_f32);
        let members = [frame_id, transcript_id, composer_id];
        let mut prev: Vec<_> = members.iter().map(|&id| read(&scene, id)).collect();

        for frame in 0..FRAMES {
            let translated = translate_portal_group_on_drag(&mut scene, frame_id, dx, dy);
            assert!(translated, "frame {frame}: portal translate must engage");

            // Content-cache sentinel must NOT move: version is frozen across the
            // whole drag, so the version-gated markdown/truncation caches skip
            // every frame — ZERO re-primes / re-shapes.
            assert_eq!(
                scene.version, version_at_drag_start,
                "frame {frame}: a position-only drag must NOT bump scene.version \
                 (would re-prime content caches every frame — hud-uyhpn)"
            );

            // The present-gate must still see the frame as dirty: geometry_epoch
            // advances exactly once per applied translate.
            assert_eq!(
                scene.geometry_epoch,
                epoch_at_drag_start + (frame as u64 + 1),
                "frame {frame}: geometry_epoch must advance once per drag frame so \
                 the moved portal repaints"
            );

            // Every member actually moved by the delta this frame.
            for (i, &id) in members.iter().enumerate() {
                let now = read(&scene, id);
                assert!(
                    (now.x - (prev[i].x + dx)).abs() < 1e-3
                        && (now.y - (prev[i].y + dy)).abs() < 1e-3,
                    "frame {frame}: member {i} must translate by the per-frame delta"
                );
                // Size never changes on a move — so truncation keys (size-based)
                // stay identical and the cache stays valid the whole drag.
                assert!(
                    (now.width - prev[i].width).abs() < 1e-3
                        && (now.height - prev[i].height).abs() < 1e-3,
                    "frame {frame}: member {i} size must not change on a move"
                );
                prev[i] = now;
            }
        }

        // Over the whole drag: version never moved (0 re-primes), geometry_epoch
        // moved once per frame (FRAMES repaints).
        assert_eq!(
            scene.version, version_at_drag_start,
            "scene.version must be unchanged across the entire drag"
        );
        assert_eq!(
            scene.geometry_epoch,
            epoch_at_drag_start + FRAMES as u64,
            "geometry_epoch must have advanced once per drag frame"
        );
    }

    /// hud-uyhpn: the single-tile (non-portal) move fallback is also position-only
    /// — it must advance `geometry_epoch`, not `scene.version`.
    #[test]
    fn single_tile_move_fallback_is_position_only() {
        use tze_hud_scene::{Capability, Rect, SceneGraph};
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "agent",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(10.0, 10.0, 100.0, 80.0),
                1,
            )
            .unwrap();

        let version_before = scene.version;
        let epoch_before = scene.geometry_epoch;

        // A plain tile resolves to the single-tile fallback (translate returns
        // false); the caller then applies bounds + geometry epoch. Reproduce that
        // position-only contract exactly.
        assert!(
            !translate_portal_group_on_drag(&mut scene, tile_id, 20.0, 15.0),
            "a plain tile must fall through to the single-tile move path"
        );
        if scene.tiles.contains_key(&tile_id) {
            if let Some(tile) = scene.tiles.get_mut(&tile_id) {
                tile.bounds.x += 20.0;
                tile.bounds.y += 15.0;
            }
            scene.bump_geometry_epoch();
        }

        assert_eq!(
            scene.version, version_before,
            "single-tile move must not bump scene.version (position-only)"
        );
        assert_eq!(
            scene.geometry_epoch,
            epoch_before + 1,
            "single-tile move must advance geometry_epoch so it repaints"
        );
    }

    /// hud-643dv: the runtime header-band drag handle targets the portal FRAME
    /// (the largest-area lease member), and a drag originating from that anchor —
    /// exactly what the band produces — moves the whole group position-only
    /// (leverages the #991 geometry-epoch path: no `scene.version` bump).
    #[test]
    fn header_band_anchor_is_the_frame_and_band_drag_is_position_only() {
        let (mut scene, _tab, frame_id, transcript_id, composer_id, _shield, _fm) =
            multi_surface_portal_scene();

        // The band handle the compositor emits targets the frame/anchor tile.
        let anchors = scene.portal_header_band_anchors(52.0);
        assert!(
            anchors.iter().any(|(a, _)| *a == frame_id),
            "the header-band drag handle must target the portal frame anchor"
        );
        assert!(
            !anchors
                .iter()
                .any(|(a, _)| *a == transcript_id || *a == composer_id),
            "panes must NOT get their own header band"
        );

        // Dragging from the anchor (as a band drag does) moves the whole group
        // position-only: no scene.version bump, geometry_epoch advances once.
        let read = |s: &tze_hud_scene::graph::SceneGraph, id| s.tiles.get(&id).unwrap().bounds;
        let (fb, tb, cb) = (
            read(&scene, frame_id),
            read(&scene, transcript_id),
            read(&scene, composer_id),
        );
        let version_before = scene.version;
        let epoch_before = scene.geometry_epoch;
        let (dx, dy) = (40.0_f32, -25.0_f32);
        assert!(translate_portal_group_on_drag(&mut scene, frame_id, dx, dy));

        assert_eq!(
            scene.version, version_before,
            "a band drag must be position-only — no content-cache re-prime (hud-uyhpn)"
        );
        assert_eq!(scene.geometry_epoch, epoch_before + 1);
        for (before, id) in [(fb, frame_id), (tb, transcript_id), (cb, composer_id)] {
            let after = read(&scene, id);
            assert!(
                (after.x - (before.x + dx)).abs() < 1e-3
                    && (after.y - (before.y + dy)).abs() < 1e-3,
                "every member must translate by the band-drag delta"
            );
        }
    }

    /// A single non-portal tile drag must NOT engage the whole-portal translate
    /// path (no scrollable constituent), so behavior is unchanged for plain tiles.
    #[test]
    fn drag_move_single_non_portal_tile_is_not_group_translated() {
        use tze_hud_scene::{Capability, Rect, SceneGraph};
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "agent",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(10.0, 10.0, 100.0, 80.0),
                1,
            )
            .unwrap();

        let translated = translate_portal_group_on_drag(&mut scene, tile_id, 20.0, 20.0);
        assert!(
            !translated,
            "a plain non-portal tile must be left to the single-tile move path"
        );
        assert!(
            !scene.is_viewer_geometry_locked(tile_id),
            "a non-portal single-tile drag must not take geometry authority"
        );
    }

    /// hud-lyqun regression: resize the whole portal, THEN drag it — the group
    /// must stay coherent (relative layout preserved) rather than fracturing.
    #[test]
    fn resize_then_drag_keeps_portal_group_coherent() {
        let (mut scene, _tab_id, frame_id, transcript_id, composer_id, _shield_id, _fm) =
            multi_surface_portal_scene();

        // Grow the portal (top-left anchored) to a larger rect.
        resize_group_to(
            &mut scene,
            frame_id,
            tze_hud_input::PortalRect {
                x: 100.0,
                y: 100.0,
                width: 560.0,
                height: 420.0,
            },
        );

        let read =
            |scene: &tze_hud_scene::graph::SceneGraph, id| scene.tiles.get(&id).unwrap().bounds;
        let frame_mid = read(&scene, frame_id);
        let transcript_mid = read(&scene, transcript_id);
        let composer_mid = read(&scene, composer_id);

        // Now drag the resized portal.
        let (dx, dy) = (-60.0_f32, 90.0_f32);
        assert!(translate_portal_group_on_drag(&mut scene, frame_id, dx, dy));

        let frame_after = read(&scene, frame_id);
        let transcript_after = read(&scene, transcript_id);
        let composer_after = read(&scene, composer_id);

        // Relative layout is preserved through resize AND the subsequent drag.
        assert!(
            approx_tuple(
                rel_to_frame(transcript_mid, frame_mid),
                rel_to_frame(transcript_after, frame_after)
            ),
            "transcript must keep relative layout after resize+drag"
        );
        assert!(
            approx_tuple(
                rel_to_frame(composer_mid, frame_mid),
                rel_to_frame(composer_after, frame_after)
            ),
            "composer must keep relative layout after resize+drag"
        );
        // Frame translated by the drag delta.
        assert!(
            (frame_after.x - (frame_mid.x + dx)).abs() < 1e-3
                && (frame_after.y - (frame_mid.y + dy)).abs() < 1e-3,
            "frame must translate by the drag delta after a resize"
        );
    }

    /// hud-lyqun proof: after a whole-portal resize, an adapter republishing its
    /// stale client-side member layout (via `update_tile_bounds`) CANNOT move any
    /// member — the group cannot be fractured.
    #[test]
    fn adapter_republish_cannot_fracture_resized_portal_group() {
        let (mut scene, _tab_id, frame_id, transcript_id, composer_id, _shield_id, _fm) =
            multi_surface_portal_scene();

        resize_group_to(
            &mut scene,
            frame_id,
            tze_hud_input::PortalRect {
                x: 100.0,
                y: 100.0,
                width: 560.0,
                height: 420.0,
            },
        );

        let read =
            |scene: &tze_hud_scene::graph::SceneGraph, id| scene.tiles.get(&id).unwrap().bounds;
        let frame_scaled = read(&scene, frame_id);
        let transcript_scaled = read(&scene, transcript_id);
        let composer_scaled = read(&scene, composer_id);

        // The adapter re-emits its OLD pre-resize client-side layout for a subset
        // of members (exactly the live-observed fracture: some members stomped to
        // stale bounds while others keep runtime-scaled bounds).
        let _ = scene.update_tile_bounds(
            transcript_id,
            tze_hud_scene::Rect::new(110.0, 110.0, 180.0, 280.0),
            "portal-agent",
        );
        let _ = scene.update_tile_bounds(
            composer_id,
            tze_hud_scene::Rect::new(300.0, 110.0, 190.0, 280.0),
            "portal-agent",
        );

        // Nothing moved: the runtime-owned scaled geometry held for every member.
        assert_eq!(
            read(&scene, transcript_id),
            transcript_scaled,
            "adapter republish must not stomp the transcript pane after a resize"
        );
        assert_eq!(
            read(&scene, composer_id),
            composer_scaled,
            "adapter republish must not stomp the composer pane after a resize"
        );
        assert_eq!(
            read(&scene, frame_id),
            frame_scaled,
            "the frame must keep its resized geometry"
        );

        // The group is still internally coherent (relative layout intact).
        assert!(approx_tuple(
            rel_to_frame(transcript_scaled, frame_scaled),
            rel_to_frame(read(&scene, transcript_id), read(&scene, frame_id))
        ));
    }
}
