# Glossary

Quick reference for darkrun's vocabulary.

- **Factory** — a methodology: an ordered set of stations that take work from
  intent to shipped. The top of the hierarchy.
- **Station** — one stage of the factory line. Runs the six-phase machine and
  locks a single durable artifact.
- **Unit** — a discrete piece of work a station produces and you review.
- **Pass** — one Make → Challenge → Resolve cycle a worker runs inside a Unit.
- **Make / Challenge / Resolve** — the three beats of a Pass: produce a
  candidate, attack it for its weakest seam, then fix what the attack surfaced.
- **Decompose** — split a station's work into Units with testable completion
  criteria and a dependency DAG before any output is made.
- **Worker** — an agent that runs a beat of a Pass.
- **Run** — the top-level execution of a factory against a real task.
- **Explorer** — an agent that gathers context in the spec phase.
- **Reviewer** — an agent that verifies a station's output in the audit phase.
- **Checkpoint** — the gate that ends a station: auto, ask, external, or await.
- **fix-workers** — targeted workers that repair drift and feedback without
  restarting the station.
- **the manager** — the loop that runs the line: advancing stations, dispatching
  workers, and stopping at checkpoints.
- **Phase** — one of spec, review, manufacture, audit, tests, checkpoint.
