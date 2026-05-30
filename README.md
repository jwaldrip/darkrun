# darkrun

A dark factory harness for your business — an agentic assembly line for any
structured work, built around an assembly-line metaphor and shipped as a single
Rust binary. The software factory is the first factory we ship, not the whole
product.

## The factory metaphor

darkrun models structured work as a **Factory** built from ordered **Stations**.
A top-level execution is a **Run**. Inside every Station the engine walks a
universal slot:

```
Explore -> Decompose -> Pass-loop(Make -> Challenge -> Resolve) -> Review -> Checkpoint -> Lock
```

Hierarchy: **Factory > Station > Unit > Pass**.

| concept            | meaning                                                       |
| ------------------ | ------------------------------------------------------------ |
| Factory            | a methodology (the software factory is the first one)        |
| Station            | one risk-eliminating stage in the factory                    |
| Unit               | a decomposed piece of work with completion criteria          |
| Pass               | one Make -> Challenge -> Resolve iteration over a Unit        |
| Worker             | an agent that performs a beat of a Pass                       |
| Explorer           | gathers the context a Station needs                          |
| Reviewer           | verifies output against criteria, independent of the Workers  |
| Checkpoint         | the gate (auto / ask / external / await) that ends a Station |
| Run                | a top-level execution through the factory                    |

The software factory's stations, in cost-of-late-discovery order:
**Frame -> Specify -> Shape -> Build -> Prove -> Harden**.

## Workspace layout

```
crates/
  darkrun-core/   domain types + filesystem state engine (locks, DAG, frontmatter)
  darkrun-api/    shared wire contract (serde + schemars session payloads)
desktop/          Dioxus cross-platform review app (later phase)
```

State lives entirely on the filesystem under `.darkrun/<run>/`:

```
.darkrun/<run>/
  run.md          frontmatter + body for the Run
  units/*.md      one markdown doc per Unit
  state.json      derived station/run state snapshot
  feedback/*.md   feedback items
  session.json    active review/question/etc. session
```

## Building

```sh
cargo build -p darkrun-core -p darkrun-api
cargo test  -p darkrun-core
```

Toolchain is pinned to stable via `rust-toolchain.toml` (edition 2021).
