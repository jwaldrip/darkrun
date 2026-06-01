# darkrun annotations — the human↔agent feedback channel

**view = review.** There is one artifact surface, not two. You open an artifact to
look at it; annotation is a *mode you toggle on*, never a separate screen. Tick a
tool, mark the thing, type what you mean, submit. The same act that lets a human
express themselves has to hand the agent a precise, re-locatable place in the
*source* and a clear intent — otherwise the loop is broken.

So every annotation is three things, always:

- **anchor** — a durable pointer into the artifact (the *where*)
- **expression** — how the human marked it: pin, box, arrow, highlight, freehand, text-selection (the *how*)
- **intent** — the comment plus an optional structured ask (the *why / what-to-do*)

The hard requirement: an annotation must **round-trip**. The human marks pixels or
a text span; we store structured, version-pinned data; the agent receives a
**source location** + the crop/quote + the intent, so it can deterministically
re-find the work and change it. Pixels in, `file:line` out.

---

## The universal envelope

Every annotation, regardless of artifact type, is the same record. Only the
`anchor` block is typed per artifact.

```jsonc
{
  "id": "anno_01J...",
  "created_at": "2026-05-31T18:40:00Z",
  "author": "human",                      // human | agent (agent can annotate too)
  "work_item": { "kind": "output", "id": "dashboard", "station": "build" },
  "artifact": {
    "id": "dashboard.png",
    "path": ".darkrun/checkout/build/outputs/dashboard.png",
    "type": "image",                      // text | image | html | pdf | svg | video
    "version_sha": "9f3c…"                // sha256 of the artifact bytes when marked
  },
  "anchor": { /* typed per artifact type — see below */ },
  "expression": { "tool": "box", "color": "#5fd7ff" },
  "comment": "total is misaligned with the rows",
  "ask": { "kind": "change", "severity": "should" },  // change|question|nit|praise · must|should|nit
  "status": "open"                        // open | addressed | dismissed
}
```

`version_sha` is load-bearing: an annotation is pinned to the *exact version of the
artifact it was made against*. When the agent produces a new version, every
annotation re-anchors against it (below) and is flagged if it may have shifted.

Stored at `.darkrun/<run>/annotations/<id>.json`, indexed by `work_item`. Retained
on exit for debuggability (same policy as the run dir) — `status` carries the
lifecycle, nothing is deleted.

---

## Anchoring by artifact type

### Text — markdown, code, spec, plain

A text annotation targets a **span**, and we store it three ways so it survives the
file changing under it:

```jsonc
"anchor": {
  "range": { "start_line": 42, "start_col": 3, "end_line": 44, "end_col": 18 },
  "quote": "fn charge(card: Card) -> Result<Receipt> {\n    gateway.submit(card)",
  "prefix": "// the unhappy path is the point\n",   // ~40 chars before
  "suffix": "\n        .map_err(Error::Gateway)?;"     // ~40 chars after
}
```

- If the artifact's current `version_sha` matches, the **line range is exact** — the
  agent jumps straight there.
- If it's changed, we **re-anchor by `quote`** (search the file for the span), using
  `prefix`/`suffix` to disambiguate duplicates. Found → exact span, silently
  re-based. Not found → the annotation is flagged **"may have shifted"** and shown
  near its old line with the quote, so a human (or the agent) can re-place it. This
  is the same idea that lets a code-review comment go "outdated" instead of lying.

**Expression for text:** select-to-comment (the span highlights, a comment
popover anchors to it), multi-line select, **strike** (suggest deletion),
**suggestion** (inline replacement — stored as a unified diff on the span).

**What the agent gets:**
```
build/outputs/payment.rs:42-44   "may have shifted: false"
> fn charge(card: Card) -> Result<Receipt> {
>     gateway.submit(card)
comment: handle the declined-card path here, not just network errors
suggestion (optional):
  - .map_err(Error::Gateway)?
  + .map_err(classify_decline)?
```
The agent opens the file, lands on the span (by line, or by quote if shifted),
reads the comment, and — if a suggestion diff is present — has a concrete edit to
apply or argue with.

### Image — png/jpg, screenshots, renders

Image annotations live in **normalized coordinates (0..1)** so they're
resolution-independent, and carry a shape:

```jsonc
"anchor": {
  "shape": "rect",                 // pin (point) | rect | arrow | path (freehand) | highlight
  "rect": { "x": 0.55, "y": 0.46, "w": 0.18, "h": 0.07 },
  // pin:  { "x":0.55,"y":0.46 }   arrow: { "from":{},"to":{} }   path: [{x,y}…]
  "render_w": 1440, "render_h": 900   // the pixel space it was drawn over
}
```

On submit, the engine **crops the annotated region** from the version-pinned image
and stores it beside the annotation (`anno_…__crop.png`). So the agent doesn't get
"a point on an image" — it gets *the actual pixels the human circled*, plus the
coordinates, plus the comment.

The missing piece is pixels → source. That comes from **provenance**: the output
record knows how `dashboard.png` was produced (which component / route / unit
rendered it). So the agent gets the crop + coords + comment **and** the source the
image came from. Where the producer can do better (HTML, below) we resolve all the
way to `file:line`.

**What the agent gets:**
```
output dashboard.png  ·  region (0.55,0.46 0.18×0.07)
[crop image attached]
provenance: rendered from web/cart/Summary.tsx (route /checkout)
comment: total is misaligned with the rows
```

### HTML / live visual — the strong case

HTML is the one type where we can anchor *both* pixels and source, so we capture
both:

```jsonc
"anchor": {
  "pixel": { "shape":"rect", "rect": {…}, "render_w":1440, "render_h":900 },
  "dom": {
    "selector": "main > section.summary > .total-row",
    "src": "web/cart/Summary.tsx:118",     // from a build-time source map
    "outer_html": "<div class=\"total-row\">…</div>"
  }
}
```

The trick: when the factory renders HTML for review, it injects
`data-darkrun-src="file:line"` on elements (a source map). The user annotates
*pixels* (natural, expressive); we read the element under the annotation and resolve
it to the **exact source line** via that attribute. The human never thinks about the
DOM; the agent gets `Summary.tsx:118`.

**What the agent gets:** `file:line` + the element's `outer_html` + the pixel crop +
the comment. It edits the component directly. This is the gold standard, and it's
why HTML/web outputs are the best-supported review target.

### Everything else

| type  | anchor                                   | source resolution |
|-------|------------------------------------------|-------------------|
| pdf   | `{ page, rect(normalized) }`             | per-page crop + provenance doc |
| svg   | `{ element_id \| xpath, bbox }`          | direct (svg is source) |
| video | `{ t_start, t_end, rect? }`              | frame crop + provenance |
| 3d    | `{ camera, hit_point, node_id }`         | node → source mesh/def |

The rule is uniform: a **typed, durable locator**, the **version** it was made
against, a **human-readable snapshot** (quote or crop), and enough to resolve to
**source**.

---

## Rich tools (human expression) → same envelope

The toolbar adapts to the artifact, but every tool emits the same record:

- **Text:** select-to-comment · multi-line select · highlight · strike (delete) ·
  suggestion (inline replacement, stored as a diff)
- **Visual:** pin · box/region · arrow (from→to) · freehand pen · highlighter ·
  text callout · redact/blur · measure
- **On every annotation:** a comment + an optional structured **ask**
  (`change | question | nit | praise`, severity `must | should | nit`)

Expression is for the human; the `ask` + anchor are for the agent. A freehand scrawl
and a precise box both resolve to the same `{anchor, comment, ask}` the agent acts
on — the richness is in how freely the human can point, not in how the agent parses
art.

---

## Severity drives the checkpoint

The checkpoint's two buttons are not static — the open annotations on the station
steer them, via each annotation's `ask.severity`:

- **Clean** — no annotations, or only `nit` — **Approve** is primary; Request
  changes is the quiet secondary.
- **Any open `should` (high) or `must` (blocker)** — **Approve darkens** (you can't
  cleanly approve over open blockers) and **Request changes becomes primary**.

`nit` never blocks; `should` and `must` flip the primary. The bar shows the open
count by severity (`2 blocker · 1 high · 3 nit`) so the steering is legible.

**Request changes** carries one **global station note** — a single station-level
comment (`work_item.kind: "station"`, no artifact anchor) — and submitting it ships
that note *plus every per-artifact annotation* as the station's feedback, kicking
the run into the rework loop. So the human leaves precise per-artifact marks
*and* one overall "here's the gist," and the agent gets both on the next tick.

## Versioning & the re-anchor pass

When the agent emits a new version of an artifact, the engine runs a re-anchor pass
over that artifact's open annotations:

1. New `version_sha` computed.
2. **Text:** exact if lines still hash-match; else quote-search; else flag shifted.
3. **Image/pdf/video:** coords are version-relative — re-crop against the new bytes;
   if the layout moved materially (diffable), flag "scene changed, re-check."
4. **HTML:** re-resolve the `selector`; if the element's `data-darkrun-src` still
   exists, the source anchor survives a restyle for free.

An annotation whose target is gone isn't deleted — it goes `status: shifted` and
surfaces for re-placement. Addressed ones flip to `addressed` and show as resolved
threads.

---

## End-to-end, one image annotation

1. Human opens `dashboard.png`, ticks **box**, drags a rectangle over the total,
   types "misaligned with the rows," tags it `change/should`.
2. Stored: envelope + `anchor.rect` (normalized) + `version_sha`; engine writes
   `anno…__crop.png`.
3. Next agent tick: the run hands the agent the open annotation bundle —
   crop + coords + comment + provenance (`Summary.tsx`).
4. Agent edits `Summary.tsx`, emits `dashboard.png` v2.
5. Re-anchor pass re-crops; layout fixed → human marks it `addressed`, or the agent
   proposes it resolved with the new crop as evidence.

The human pointed at a pixel. The agent changed a line. That's the whole game.
