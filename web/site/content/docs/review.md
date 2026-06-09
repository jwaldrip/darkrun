# Review and feedback

darkrun keeps a human at the checkpoints, not in the weeds. When a station
produces a **Unit** you can open it, read its output, and respond — without
stopping the rest of the line.

## The review session

Open a Run's **Review** screen and you see the current station, its phase, and
every Unit it has produced. Each Unit shows its type, its status, and which
**Pass** it is on. The desktop app and the website render the same session
payload over the local engine's WebSocket feed.

## Leaving feedback

Feedback is anchored. You can pin a comment to a Unit as a whole, or inline to a
specific span of its output. Each comment carries a **severity** and a
**status** — open, resolved, or closed — so a station knows what is still
outstanding before it can lock.

## Decisions

A review ends in a decision: **approve** advances the checkpoint, **request
changes** sends the Unit back for another Pass through the fix-workers. The
manager records the decision and the iteration result so the next pass starts
with full context.

## HTML source resolution

When you box a region on a rendered HTML artifact, darkrun captures three things
about the element under your mark: a CSS **selector**, its **outerHTML**, and —
*if the project opted in* — a `file:line` **source** read from a
`data-darkrun-src` attribute. That last one is the strong case. With it, the
agent re-references your mark straight to the line that produced it. Pixels in,
`file:line` out.

darkrun does **not** ship a universal transformer that injects that attribute
for you. There is no way to do it generically — it depends entirely on your
framework, bundler, and render path. So source resolution is **opt-in**: your
project emits the map, darkrun reads it.

- **Opted in** — the marked element carries `data-darkrun-src="path:line"`. The
  agent payload resolves to that `file:line` and the mark resolves cleanly.
- **Not opted in** (or a blank/garbage attribute value) — darkrun degrades to
  the **selector + outerHTML + the cropped region**, and flags the item
  `unresolved (no source map)`. The agent still has the element, the pixels, and
  your comment; it just has to locate the source itself.

That degraded path is the default and it is fine — you lose the one-hop jump to
the line, nothing else.

### Opting in: the source-map injector

The injector is a tiny, project-side hook that stamps `data-darkrun-src` onto
the elements it renders, in **dev only**. The shape darkrun expects is exactly:

```
data-darkrun-src="<repo-relative-file>:<1-based-line>"
```

A blank value, a path with no line, or a line of `0` is treated as *not opted
in* — darkrun only trusts a real `file:line`.

How you produce it depends on your stack. The two patterns that cover most
projects:

1. **Build-time** — a Babel/SWC/Vite JSX plugin that already knows each
   element's source location (this is how React DevTools' "jump to source"
   works) writes `data-darkrun-src` next to the JSX. This is the most accurate:
   it maps to the authoring line, not the runtime DOM.
2. **Runtime** — a small dev middleware or client snippet that tags elements as
   they mount, from whatever source hint your framework exposes. Less precise,
   zero build wiring.

A minimal, framework-agnostic runtime example lives next to the engine:
[`crates/darkrun-mcp/examples/darkrun-src-injector.js`](https://github.com/darkrun-ai/darkrun/blob/main/crates/darkrun-mcp/examples/darkrun-src-injector.js).
Copy it, point it at your own source hints, and gate it behind your dev flag.
Once your elements carry the attribute, marks on them resolve to `file:line`
with no further darkrun configuration.
