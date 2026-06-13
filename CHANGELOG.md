# Changelog

All notable changes to darkrun are recorded here. Versions follow semver.

## [0.5.0](https://github.com/darkrun-ai/darkrun/compare/v0.4.0...v0.5.0) (2026-06-13)


### Features

* **api:** add PickerKind::Size ([a5c9e9c](https://github.com/darkrun-ai/darkrun/commit/a5c9e9c8d0f1d16291bf123ccfd521ded8efacd6))
* **desktop:** readable question prompts — markdown, text-only cards, real mockups ([d71321a](https://github.com/darkrun-ai/darkrun/commit/d71321afb997617ca66f1797d40a5b8cbf751b9d))
* **engine+desktop:** questions surface on the run + persist across restarts ([598d98e](https://github.com/darkrun-ai/darkrun/commit/598d98ef384116869b23af9653be6c15df01ad6b))
* **engine+desktop:** sessions materialize on demand; chrome is not selectable ([c3c2395](https://github.com/darkrun-ai/darkrun/commit/c3c2395d87f5820142f6cf706e33e59a7800031a))
* **engine:** engine-driven run-setup elicitation (factory/mode/size pickers) ([905e8ca](https://github.com/darkrun-ai/darkrun/commit/905e8cadb65f1b41d1ca1f751711c350a5171b4b))
* **engine:** mode-gate questions + scope interactive sessions per station ([e990bd4](https://github.com/darkrun-ai/darkrun/commit/e990bd4b8652d384ec6585bea146e603fc594f03))
* **engine:** pull fable from the model selector (Anthropic removed support) ([38a4c80](https://github.com/darkrun-ai/darkrun/commit/38a4c805fe034cce808e1adb455e67b1878cb701))
* **engine:** the desktop surfaces with the work, not at the first gate ([299011a](https://github.com/darkrun-ai/darkrun/commit/299011a3e69fbb6486f53435a95d8eb2df86eeea))
* **site:** social card (Open Graph / Twitter) — the factory-line hero ([a3a2781](https://github.com/darkrun-ai/darkrun/commit/a3a2781765bd6cec1d8e614e6eda034bcd7ccb63))
* **statusline:** explorer chips at Spec + dev launcher freshness ([170ea16](https://github.com/darkrun-ai/darkrun/commit/170ea16fa08ddf17ae2b9b13c55dfbcbe31f724f))


### Bug Fixes

* **desktop:** a stale dev launch bundle execs the fresher build ([042f83b](https://github.com/darkrun-ai/darkrun/commit/042f83be6a509fed0bdd46383f0b18084f0b812d))
* **desktop:** key sidebar run lists by project slug, not display name ([14ec652](https://github.com/darkrun-ai/darkrun/commit/14ec652f6231531a49ce03134547cbdeeec64f11))
* **engine:** answering an interactive session dismisses it + surfaces the next ([5828727](https://github.com/darkrun-ai/darkrun/commit/5828727a91a98889a383ec8d207f4f5e806cb434))
* **engine:** raising a question/direction/picker gate launches the desktop ([e5586ef](https://github.com/darkrun-ai/darkrun/commit/e5586efaf3adb0dd01d9c96b6e7c179fae6259e4))
* **git:** normalize the common dir before deriving the project root ([028d2e3](https://github.com/darkrun-ai/darkrun/commit/028d2e38e5574127c22863854da49166df46f4d9))
* **http:** the Mine predicate checks the run's STABLE branch ([ab8eb8f](https://github.com/darkrun-ai/darkrun/commit/ab8eb8ff2dd1b7043183b82d095b327ffea98922))
* picker UX (chrome, stale selection, auto-close) + same-commit checkout ([a085a7d](https://github.com/darkrun-ai/darkrun/commit/a085a7d6165b3015963ec23a5ec9e0f460c445b3))
* **site:** preview question sample sets run_slug ([68f3edf](https://github.com/darkrun-ai/darkrun/commit/68f3edf99823643f0a259822c2e60548d59e4e0f))

## [0.4.0](https://github.com/darkrun-ai/darkrun/compare/v0.3.0...v0.4.0) (2026-06-11)


### Features

* **site:** Claude Code's boxed session-start banner on the statusline demo ([5963c7c](https://github.com/darkrun-ai/darkrun/commit/5963c7c89fd95d6b2d7c3a7dc24805c1dddce74c))
* **site:** left/right stepper on the statusline demo ([d9c1c07](https://github.com/darkrun-ai/darkrun/commit/d9c1c074ddb331a7c8cf9fbe874e615ab763af40))
* **site:** left/right stepper on the statusline demo ([#50](https://github.com/darkrun-ai/darkrun/issues/50)) ([27dcedf](https://github.com/darkrun-ai/darkrun/commit/27dcedf7314f1228418af0734b75222ea4319f04))
* **site:** render the statusline demo in situ, under Claude Code's prompt box ([6c42eab](https://github.com/darkrun-ai/darkrun/commit/6c42eab09a1f759b7ed58c46915b69e58107a3d0))
* **site:** the terminal panels follow the site theme ([4752490](https://github.com/darkrun-ai/darkrun/commit/47524906b4debf190be78ce582589ab82683ed37))


### Bug Fixes

* **site:** clay banner box, sized to the panel ([50fb85d](https://github.com/darkrun-ai/darkrun/commit/50fb85d60ed5400e1956dd7b6b5e188f52f6ade7))
* **site:** statusline stepper dots use the shared accent pill; drop the redundant slideshow slide ([eea29bd](https://github.com/darkrun-ai/darkrun/commit/eea29bd6b26f5f4d64a98f511872a9036acd35dc))
* **statusline:** read on light terminals — bold default-fg for slug and passed pips ([73be50f](https://github.com/darkrun-ai/darkrun/commit/73be50f3d28598ea2b1e742aee4e7e4e4aa03dd9))
* **ui:** saturate the tab count pill at 99+ ([8f5588a](https://github.com/darkrun-ai/darkrun/commit/8f5588a6b0310defe1f3421765ed476e5d89d19b))

## [0.3.0](https://github.com/darkrun-ai/darkrun/compare/v0.2.1...v0.3.0) (2026-06-11)


### Features

* **desktop:** live per-tick session mirror ([a9f565d](https://github.com/darkrun-ai/darkrun/commit/a9f565d50819a71fb1d0546bab096ff24fbb7ccf))
* **desktop:** live per-tick session mirror ([#46](https://github.com/darkrun-ai/darkrun/issues/46)) ([ac76fe0](https://github.com/darkrun-ai/darkrun/commit/ac76fe085336010beaca96fc4258f94fe2741e6e))
* **engine:** composite runs — multi-factory topology with sync points ([adcfef6](https://github.com/darkrun-ai/darkrun/commit/adcfef68e6c9bf7aeef5dc6022835ef59573f50d))
* **engine:** reject-escalation up the model ladder ([4b7ce33](https://github.com/darkrun-ai/darkrun/commit/4b7ce33cbf4ee9cc80d4b09adc1a1b4bf7a6b31a))
* **engine:** reject-escalation up the model ladder ([#49](https://github.com/darkrun-ai/darkrun/issues/49)) ([b63792b](https://github.com/darkrun-ai/darkrun/commit/b63792bda5c2c6fa477d9c5787b0e4a56c2ee60c))
* **engine:** save_wip clean-tree gate + unit-scope enforcement at completion ([603c106](https://github.com/darkrun-ai/darkrun/commit/603c1069e2e47241049ac71a184c0a4478a185c2))
* **engine:** session-event stream + OTLP telemetry export ([17873e5](https://github.com/darkrun-ai/darkrun/commit/17873e5c66eab59f5b00c89b4eb409919473e661))
* **engine:** station drop — the keep-or-drop offer at arrival ([e55f7fb](https://github.com/darkrun-ai/darkrun/commit/e55f7fb008c73455506eb85685894e0cee264876))
* **factory:** runtime-verifier run reviewer (the predecessor's strongest gate) ([08d8e45](https://github.com/darkrun-ai/darkrun/commit/08d8e45b0775e60ce7aae045bab3cfe73d4e3602))
* **hosting:** run-level draft PR with ready-at-seal flip + compare-URL fallback ([08d8e45](https://github.com/darkrun-ai/darkrun/commit/08d8e45b0775e60ce7aae045bab3cfe73d4e3602))
* **providers:** behavior contracts spliced into prompts + schema-validated settings ([99f2687](https://github.com/darkrun-ai/darkrun/commit/99f26873b330f51073d6ac25c35c5989fdf87da6))
* **site+desktop:** refreshed desktop screenshots + the harness that makes them reproducible ([7df1a90](https://github.com/darkrun-ai/darkrun/commit/7df1a90642ed6604aafd612242bfc311dde9b5a8))
* **site:** docs search + JSON-LD structured data ([3e0ceaa](https://github.com/darkrun-ai/darkrun/commit/3e0ceaa93691d2bda46a6bbd0b44df8a81f5c1ea))
* **statusline+site:** phase-track pips + the status line on the website + the fable tier ([5a7ae9a](https://github.com/darkrun-ai/darkrun/commit/5a7ae9ac7af4421911b2c84a97f80bcb0da48286))


### Bug Fixes

* **api:** make openapi.json a fixed point of release-please's rewrite ([f3bbf95](https://github.com/darkrun-ai/darkrun/commit/f3bbf957a4ae51d0746dfed79006678250a596e0))
* **api:** make openapi.json a fixed point of release-please's rewrite ([#42](https://github.com/darkrun-ai/darkrun/issues/42)) ([c964465](https://github.com/darkrun-ai/darkrun/commit/c96446584c9e1c5f84953ec92df57a909e4999a1))
* **desktop:** project identity self-heal, --worktree launch, choosable clone path ([c0efbbf](https://github.com/darkrun-ai/darkrun/commit/c0efbbf82d718677187fde831bf26e79a791bcb6))
* **site:** make the feed-date suggestions compile + emit valid formats ([88f4061](https://github.com/darkrun-ai/darkrun/commit/88f40619b36a4df2f03515140385072dba8dc2b6))

## [0.2.1](https://github.com/darkrun-ai/darkrun/compare/v0.2.0...v0.2.1) (2026-06-08)


### Bug Fixes

* propagate the 0.2.0 release bump (unblock all open PRs) ([#20](https://github.com/darkrun-ai/darkrun/issues/20)) ([ae45020](https://github.com/darkrun-ai/darkrun/commit/ae4502037ebb67513017416143c6830a2f77489b))

## [0.2.0](https://github.com/darkrun-ai/darkrun/compare/v0.1.0...v0.2.0) (2026-06-08)


### Features

* darkrun — factory-orchestration engine, design system, website, and plugin ([f6365d8](https://github.com/darkrun-ai/darkrun/commit/f6365d812cf4bd730c9af79147954fd3bf9356cd))
* **git:** complete Phase 1 gix reads — ls_tree, unresolved_paths, list_worktrees ([b33e131](https://github.com/darkrun-ai/darkrun/commit/b33e1318ec367af7571dff1536c4d81cd4b8a434))
* **git:** gix add_paths + checkout_paths — Phase 2 complete ([47ff4b3](https://github.com/darkrun-ai/darkrun/commit/47ff4b364f32ecd0344704c1861f5f17f0c9bcf2))
* **git:** gix create_branch (Phase 2 start) — idempotent ref write ([b5d7267](https://github.com/darkrun-ai/darkrun/commit/b5d7267bfb6fca912b6e6230e2b639c113661cc6))
* **git:** gix engine-protected three-way merge (Phase 4 — the core safety net) ([5346434](https://github.com/darkrun-ai/darkrun/commit/5346434f2807bd7d6ed4c189914e0868a17b380e))
* **git:** gix linked-worktree create/remove (Phase 3 — first gitoxide gap) ([233a16b](https://github.com/darkrun-ai/darkrun/commit/233a16bbae8deb584dc49f30138d7a3fbec6ec64))
* **git:** gix native fetch (Phase 5) — pure-Rust transport, C-free ([6955549](https://github.com/darkrun-ai/darkrun/commit/69555491cb7cec5f78d6db39809cbdbc0dca62c2))
* **git:** gix reads — is_ancestor, refs_have_identical_trees, merge_in_progress ([c074ed0](https://github.com/darkrun-ai/darkrun/commit/c074ed05688954c32044741d1da3f832e717c4d7))
* **git:** hand-rolled write-tree + gix commit (fork-A internals) ([a7e01b6](https://github.com/darkrun-ai/darkrun/commit/a7e01b6dd23af36a49a9ff22f791803a8981c435))
* **git:** scaffold pure-Rust gitoxide backend (Phase 1 foundation) ([885462d](https://github.com/darkrun-ai/darkrun/commit/885462dd8be770255219ee5193bef7720c27a206))
* phase redesign + engine-driven prompts + hooks; Apache-2.0; dark-factory positioning ([5ccf3e9](https://github.com/darkrun-ai/darkrun/commit/5ccf3e9fb6bde91532201c6e33304725c61d8eb2))
* **verify:** objective surface-routed verification + view/visual-review + proof ([60062d9](https://github.com/darkrun-ai/darkrun/commit/60062d96dd94aca99f7cdf8a8d47bfc76b35a5b8))
* **visual:** visual-question + design-direction sessions, screens, and a visual-design step ([db0500e](https://github.com/darkrun-ai/darkrun/commit/db0500e3bc079ca7639471c0939b9a7b2ec3bd3d))


### Bug Fixes

* 0-byte outputs don't satisfy the gate; verify gate/drift loop immunity (predecessor BUGs 1, 3, drift A/B) ([9220710](https://github.com/darkrun-ai/darkrun/commit/92207108953e3bf732d31118726026b03efe607e))
* darkrun show deep-links to the run; stations render in factory order ([655d7a0](https://github.com/darkrun-ai/darkrun/commit/655d7a087c8e7c144e885cd444e4df4312a43195))
* derived_station_phase test needs a unit with a Pass signal (was asserting None-case) ([770ed56](https://github.com/darkrun-ai/darkrun/commit/770ed564a31248490c7c9ac0b5947e14a5792471))
* **plugin:** implement the darkrun hook subcommand so hooks never block tools ([5c3eb12](https://github.com/darkrun-ai/darkrun/commit/5c3eb125cc5105887be856d4b5b831e5e783289e))
* **site:** embed factory corpus in wasm builds + dx 0.7 config ([f8a05f3](https://github.com/darkrun-ai/darkrun/commit/f8a05f3957c4fd09594c3572d0bfff8b04fa7e3e))
* **ui:** stack the run walkthrough — station walker over a centered phase machine ([f4d040b](https://github.com/darkrun-ai/darkrun/commit/f4d040b66b53f3aa54ac84016d75d464c8877a48))

## 0.1.0 — unreleased

The first darkrun: a native Rust engine that drives Runs through a factory's
stations (Frame → Specify → Shape → Build → Prove → Harden for the software
factory), with a desktop review app and a Claude Code plugin.

- **Manager** — a pure-read cursor over `.darkrun/` state, walking the
  six-phase station machine (spec → review → manufacture → audit → reflect →
  checkpoint) across a three-track priority (drift → feedback → run).
- **Full action set** — validation (units-invalid, escalate, safe-repair),
  repair/rollback, external review, and the seal tail.
- **Objective verification** — surface-routed proof (`darkrun verify web`,
  `darkrun bench`) instead of eyeballed review.
- **Reflection** — durable run-level retrospectives.
- **Auto-tune** — run-start right-sizing (quick / bugfix / refactor / full).
- **Drift sweep** — detects mutated locked artifacts and self-heals on revert.
- **Multi-harness** — Claude Code, Cursor, Windsurf, Gemini CLI, OpenCode,
  Kiro, and Codex, each adapted from one capability registry.
