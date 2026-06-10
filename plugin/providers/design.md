---
provider_kind: design
category: source
always_on: false
splices_into:
  - spec
description: Design source provider — pull existing designs, components, and tokens to align units to the design system.
---

# Design Provider — Behavior Contract

A design provider is configured (Figma, Pencil, Penpot, Excalidraw, Canva, OpenPencil) when `providers.design.*` is set in `.darkrun/settings.yml`. This is a **source** provider for darkrun's purposes — read designs that already exist, align station outputs to them. Don't push designs back unless the user explicitly asks (and the relevant MCP tool with write access is available).

## What you, the agent, must do

### At Spec (Shape station)
- Pull the existing design file(s) referenced in the Run. Record each via `darkrun_external_ref` (`design`).
- Extract design tokens (colors, spacing, typography) and stage them as discovery inputs. Downstream units cite the token names.
- Surface the component inventory: what already exists that the units can reuse vs what's new.

### At Spec (Build / non-design stations)
- Re-fetch the design refs the Shape station produced. Verify they haven't drifted since Shape's checkpoint locked. Surface drift to the operator — a changed upstream design is input drift.

### At decompose
- Every UI unit cites the relevant design ref in `inputs`. The reference is enough — don't inline the design content into the unit body.
- Map component names from the design system to implementation modules. If a unit's scope includes a named component, the unit's title and body should match the component name.

## When to push back

Push to the design tool only when:
1. The user explicitly asks ("update the Figma component"), AND
2. You have the write tool available for the configured design provider.

Default behavior is read-only. The design tool is the source of truth; darkrun units are the implementation.

## Per-tool storage convention

When recording a design reference, use the per-tool URI convention:

| Tool | URI |
|---|---|
| Figma | `figma://<file_key>#node=<node_id>` |
| Penpot | `penpot://<host>/<project_id>/<file_id>#component=<id>` |
| Pencil / OpenPencil | `pencil://<document_id>#node=<node_id>` |
| Canva | `canva://<design_id>#page=<n>` |
| Excalidraw | `excalidraw://<drawing_id>` or `excalidraw://local/<file_path>` |

## What NOT to do

- Don't fabricate design URIs. If you can't reach the design tool, leave the field empty and surface the gap.
- Don't push wireframes or mockups darkrun produces back to the design tool by default — they're station artifacts, not design-tool-of-truth content.
- Don't load the entire design file into context if a single frame or component is enough.
