# Pure Rust, no C

darkrun is Rust end to end. The engine, the CLI, the desktop app, the website you're reading. No Python sidecar, no Node runtime, no shelling out to a system tool and parsing its stdout.

:::callout
One binary, no runtime dependencies.
:::

The part people don't expect is git.

## Git with no git

darkrun does a lot of git. It cuts a worktree per station so work stays isolated, fetches, merges three ways when a rebase conflicts, branches per unit. The obvious way to build that is to shell out to the `git` CLI and scrape the output. Almost everything does this.

darkrun doesn't run `git` once. Git is 100% pure Rust through [gitoxide/gix](https://github.com/GitoxideLabs/gitoxide). Worktree creation, fetch, three-way merge, ref updates — all of it happens in-process, against the object database, with no subprocess and no C library linked in.

| Operation | Common approach | darkrun |
|---|---|---|
| Create worktree | `git worktree add` subprocess | `gix` in-process |
| Fetch | `git fetch` subprocess | native `gix` fetch |
| Merge | `git merge`, parse exit code | three-way merge in Rust |
| Read status | `git status --porcelain`, parse | direct object-db read |

The payoff is that there's nothing to scrape and nothing to mis-parse. A subprocess hands you a string and an exit code, and you reverse-engineer what happened from text meant for a human. In-process, the merge gives you a structured result. A conflict is a value, not a line you grep for. No `git` on the PATH, no version skew between the git someone has installed and the behavior darkrun expects.

## The factory corpus ships inside the binary

A factory is its prose: the station definitions, the worker and reviewer prompts, the lifecycle. darkrun embeds all of it at compile time with [`rust-embed`](https://github.com/pyrossh/rust-embed). The corpus lives *inside* the executable.

So there's no install step that copies templates to `~/.config`, no "did the files get there," no version drift between the binary and the prose it runs. The factory the binary runs is the factory the binary was built with. Byte for byte.

## One language all the way down

```
engine        Rust
CLI           Rust
desktop app   Rust (Dioxus)
website        Rust (Dioxus SSG)
git           Rust (gix)
factory corpus  embedded (rust-embed)
```

The desktop app is [Dioxus](https://dioxuslabs.com/), so the review UI is the same language as the engine it drives. The website is Dioxus too, rendered as a static site at build time, which is why these pages have no client framework loading underneath them. The page is HTML the build produced from the same content the app embeds.

## Why bother

Three things fall out of "no C, no subprocess, one binary."

:::keypoints title="What no C, no subprocess, one binary buys"
- **It's one artifact.** You ship a single static binary. No "make sure git 2.40+ is installed," no Python venv, no node_modules. It runs where you put it.
- **It's reproducible.** Subprocesses make behavior depend on the host. The git on your laptop and the git in CI can differ, and then so does darkrun. With gix linked in, the git behavior is pinned to the binary. Same input, same output, everywhere.
- **It fails honestly.** A merge conflict comes back as a typed value the engine matches on, not a string it parses and hopes it understood. Fewer places for "the tool said something we didn't anticipate" to turn into a wedged run.
:::

:::callout
Rust everywhere buys one thing: darkrun is a single artifact you can drop on a machine and trust, instead of a script only as reliable as the six tools it happens to find installed. The purity is the means, not the flex.
:::
