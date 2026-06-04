{% include "_shared/announcement.md" %}

# Malformed decomposition — `{{ station }}`

The manager refuses to manufacture against a broken decomposition. Station `{{ station }}` has Units that don't hold together, and the line stops here until they do — a bad DAG caught now is cheap; caught after a wave of parallel workers built on it, it isn't.

**Problem:** `{{ problem }}`

{% if units %}
**Offending Units:**
{% for u in units %}
- `{{ u }}`
{% endfor %}
{% endif %}

## What to do

{% if problem == "invalid_naming" %}
A Unit slug is empty or not kebab-case (lowercase, hyphen-separated, no spaces). Rename the offending Units to valid slugs — `darkrun_unit_update` the name, or recreate them cleanly. Slugs are identity; downstream deps reference them, so get them right before anything depends on them.
{% elif problem == "unresolved_deps" %}
A Unit declares a `depends_on` that names no Unit in this run — a dangling edge. Either **create the missing Unit** (the dependency is real and was never decomposed) or **drop the stale edge** from the offending Unit's `depends_on`. Don't manufacture against a dependency that doesn't exist.
{% elif problem == "dependency_cycle" %}
The Units above form a **dependency cycle** — they depend on each other in a loop, so no wave can ever be ready. Break the cycle: find the edge that doesn't truly need to exist and remove it, or merge the mutually-dependent Units into one. A DAG has no cycles by definition; restore that.
{% elif problem == "missing_output" %}
Each Unit above locked but **never produced a declared output** — the artifact it promised in `outputs:` is not on disk. A station cannot advance to Audit on a promise. For each Unit, either **produce the missing artifact** at its declared path, or **correct the `outputs:` declaration** if the path was wrong. Don't audit work that didn't ship its output.
{% elif problem == "input_not_a_path" %}
Each Unit above declares an **`input` that names another Unit instead of a file path**. Inputs are *premises* — artifact paths the Unit was built against, which the engine witnesses for drift. A bare Unit slug can't be witnessed. If the Unit must run *after* another, that's a `depends_on` edge, not an `input`. Move the slug to `depends_on`, and declare the real upstream artifact path (e.g. `frame/frame.md`) as the `input`.
{% else %}
The decomposition is structurally invalid. Inspect the offending Units, correct their naming / dependencies, and re-validate.
{% endif %}

## Done when

Every Unit in `{{ station }}` has a valid slug, every `depends_on` resolves to a real Unit, and the dependency graph is acyclic. Then call `darkrun_tick` — the manager re-validates and, once clean, releases the first wave.
