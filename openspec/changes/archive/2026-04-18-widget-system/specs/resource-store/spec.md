# Resource Store Specification (Widget System Delta)

Source: RFC 0011 (Resource Store), widget-system proposal
Domain: RUNTIME

---

## MODIFIED Requirements

### Requirement: V1 Resource Type Enumeration
V1 SHALL support exactly six resource types: five raster/font types (IMAGE_RGBA8, IMAGE_PNG, IMAGE_JPEG, FONT_TTF, FONT_OTF) and one vector type (IMAGE_SVG). The post-v1 type VideoSurfaceRef is deferred. Zone publish content MUST match one of the zone type's accepted_media_types. IMAGE_SVG resources SHALL be used exclusively by the widget asset bundle system; they SHALL NOT be publishable directly to zones. Upload validation for IMAGE_SVG MUST verify that the content parses as valid SVG (well-formed XML with an `<svg>` root element). IMAGE_SVG decode validation SHALL NOT rasterize the SVG — parsing to a retained SVG tree is sufficient.
Source: RFC 0011 §2.1, §2.2
Scope: v1-mandatory

#### Scenario: IMAGE_SVG upload accepted with valid SVG
- **WHEN** an agent uploads a resource with type IMAGE_SVG containing well-formed XML with an `<svg>` root element
- **THEN** the upload MUST be accepted, the content parsed to a retained SVG tree, and a ResourceStored confirmation returned

#### Scenario: IMAGE_SVG upload rejected with invalid XML
- **WHEN** an agent uploads a resource with type IMAGE_SVG containing content that is not well-formed XML
- **THEN** the upload MUST be rejected with RESOURCE_DECODE_ERROR

#### Scenario: IMAGE_SVG upload rejected with non-SVG XML root
- **WHEN** an agent uploads a resource with type IMAGE_SVG containing well-formed XML but with a root element other than `<svg>` (e.g., `<html>` or `<div>`)
- **THEN** the upload MUST be rejected with RESOURCE_DECODE_ERROR

#### Scenario: IMAGE_SVG not accepted by zone publish
- **WHEN** an agent attempts to publish an IMAGE_SVG resource directly to a zone via zone publish
- **THEN** the publish MUST be rejected; IMAGE_SVG is reserved for the widget asset bundle system and is not a valid zone media type

---

## ADDED Requirements

### Requirement: SVG Resource Budget Accounting
IMAGE_SVG resources MUST be accounted against an agent's texture budget using an estimated rasterized size, not the raw SVG byte size. The estimated size MUST be computed as: `width_px * height_px * 4` (RGBA8) where width_px and height_px are taken from the SVG's `viewBox` or `width`/`height` attributes, clamped to a maximum of 2048x2048. If the SVG has no explicit dimensions, the runtime MUST use a default of 512x512 for budget estimation. This estimation occurs at upload time and is stored alongside the resource.
Source: widget-system proposal
Scope: v1-mandatory

#### Scenario: SVG with viewBox budget estimated
- **WHEN** an agent uploads an IMAGE_SVG with `viewBox="0 0 800 600"`
- **THEN** the budget charge MUST be `800 * 600 * 4 = 1,920,000` bytes (approximately 1.83 MiB) and this estimate is stored alongside the resource

#### Scenario: SVG without dimensions uses 512x512 default
- **WHEN** an agent uploads an IMAGE_SVG that has no `viewBox`, `width`, or `height` attributes on the `<svg>` root element
- **THEN** the budget charge MUST be `512 * 512 * 4 = 1,048,576` bytes (1 MiB)

#### Scenario: SVG exceeding 2048 clamped
- **WHEN** an agent uploads an IMAGE_SVG with `width="4096" height="4096"`
- **THEN** the dimensions MUST be clamped to 2048x2048 and the budget charge MUST be `2048 * 2048 * 4 = 16,777,216` bytes (16 MiB)

#### Scenario: Budget charged to referencing agent
- **WHEN** Agent A uploads an IMAGE_SVG with estimated budget of 1 MiB and the runtime loads an IMAGE_SVG from a widget asset bundle and creates a widget instance that references that SVG
- **THEN** The widget instance's SVG texture budget SHALL be accounted as runtime overhead, not charged against any individual agent's per-agent texture budget. Widget SVG resources are runtime-owned infrastructure (loaded from asset bundles at startup), not agent-uploaded resources.
