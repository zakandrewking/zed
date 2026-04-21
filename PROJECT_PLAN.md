# Project Plan: Local Zeta2 in Zed via the Native Internal Edit-Prediction Path

## Project Summary

Build a fork of Zed that runs Zeta2 locally while preserving the richer native Zeta edit-prediction path instead of degrading to the generic OpenAI-compatible completions API.

This project is not primarily about "running an 8B model locally." It is about reproducing the full native behavior of Zed's edit prediction stack:

- rich editor/context extraction
- native Zeta2 prompt formulation
- correct rewrite-region semantics
- correct output interpretation
- acceptable local latency for debounced inline predictions

The project should proceed in a way that separates:

1. editor integration correctness
2. prompt / contract reconstruction
3. local inference performance
4. product quality evaluation

That ordering matters. Do not confuse model quality, quantization loss, prompt mismatch, and bad output-application logic.

---

## Goals

### Primary Goal

Enable Zed to use a local Zeta2 backend through the native internal edit-prediction API, preserving the richer Zeta2-specific capabilities.

### Secondary Goals

- Match hosted/native Zeta2 behavior as closely as practical.
- Reach a latency/quality point that is comfortable for debounced interactive use.
- Produce a maintainable fork with clean extension points.
- Build a replay/evaluation harness so improvements are measurable, not anecdotal.

### Non-Goals

- Building a generic model provider for all edit-prediction models.
- Matching hosted Zeta2 exactly in all cases.
- Shipping a polished OSS product from day one.
- Supporting every quantization backend immediately.

---

## Product Definition

### What "Success" Means

A user can edit code in the forked Zed build and receive local inline predictions that:

- are generated from the native Zeta path, not OpenAI-compatible FIM mode
- use rich editor context
- apply as correct local rewrites around the cursor
- feel responsive enough with debounce
- are good enough to be used daily on real code

### User Experience Constraints

- Some debounce is acceptable.
- The user does not need ghost text on every keystroke.
- Slightly worse latency than hosted Zeta2 is acceptable.
- Bad rewrites, giant deletions, and raw sentinel leakage are unacceptable.

---

## Background Assumptions

This plan assumes the following are true enough to design against:

- Zeta2 is an open-weight 8B model intended for edit prediction, not just generic FIM completion.
- The native Zeta2 path uses richer context than the generic self-hosted completions path, including recent edits and editor-aware context.
- Recent self-hosted reports suggest that pushing Zeta2 through generic paths can produce broken rewrite behavior, which implies the native request/response contract matters.

---

## Strategy

### Core Strategy

Treat this as a systems reverse-engineering and replication project, not a prompt-tweaking project.

The fastest path is:

1. Restore a native backend hook in Zed.
2. Instrument the native path heavily.
3. Capture a corpus of real editing examples.
4. Replay those examples through hosted/native behavior.
5. Fit a local prompt compiler and output interpreter to that behavior.
6. Only then optimize local inference.

This avoids the biggest trap: spending weeks "testing prompts" when the real problem is rewrite-window semantics or output interpretation.

### Strategic Principles

#### 1. Keep Zed as the Oracle

Zed already knows how to collect the right editor state. Use it.

#### 2. Separate Transport from Semantics

First reproduce the native request/response shape. Then refine prompt construction.

#### 3. Optimize for Replayed Behavior, Not Elegance

The best prompt is the one that reproduces good edits, not the one that looks nicest.

#### 4. De-risk in Layers

Hosted-through-local-shim first, true local inference second.

#### 5. Build Measurement Early

You need datasets, logs, and acceptance proxies before making architectural bets.

---

## Scope

### In Scope

- Forking Zed
- Restoring or adding a configurable native predict-edits backend URL
- Logging and instrumentation of the Zeta2 path
- Local shim/server implementation
- Prompt compiler for native Zeta2 prompt formulation
- Output parser / rewrite applier correctness checks
- Replay and evaluation harness
- Local inference integration
- Quantization experiments
- Debounce tuning
- Quality and latency benchmarking

### Out of Scope

- Replacing all of Zed's AI infrastructure
- Building a cloud service
- Cross-platform packaging polish
- Fine-tuning Zeta2
- Training a replacement model

---

## Architecture

### High-Level Architecture

```text
Forked Zed client
  -> native Zeta2 request builder
  -> configurable local predict-edits URL
  -> local zeta server/shim
      -> request logger
      -> prompt compiler
      -> inference backend adapter
      -> output interpreter
      -> response normalizer
  -> local model runtime
      -> MLX / llama.cpp / vLLM / Ollama-compatible backend
```

### Major Components

#### 1. Zed Fork

Responsibilities:

- preserve native Zeta2 path
- expose configurable local backend endpoint
- emit detailed logs/telemetry for evaluation
- optionally capture full request/response artifacts

#### 2. Local Zeta Server / Shim

Responsibilities:

- accept native Zed predict-edits requests
- compile request data into native Zeta2 prompt form
- call local inference backend
- normalize model output into editor-safe rewrite responses
- log latency and quality metadata

#### 3. Prompt Compiler

Responsibilities:

- convert editor state into Zeta2-native serialized prompt
- control truncation and context ordering
- control rewrite window
- include edit history and related-file context
- support ablations/config search

#### 4. Output Interpreter

Responsibilities:

- parse model output safely
- handle no-edit / keep-previous semantics
- bound rewrite region
- prevent catastrophic deletions from malformed outputs
- produce a normalized result that Zed can apply

#### 5. Replay Harness

Responsibilities:

- evaluate candidate prompt compiler settings
- compare hosted/native outputs to local outputs
- score results on correctness, overlap, latency, and acceptance proxies

---

## Workstreams

### Workstream A: Zed Fork and Native Hook Restoration

#### Objective

Patch current Zed so the native Zeta2 path can target a user-controlled local backend.

#### Deliverables

- forked Zed repo
- configurable local endpoint
- debug build with rich logging
- feature flag to switch between hosted and local backend

#### Tasks

1. Identify current Zeta2 request path in the codebase.
2. Restore or add a `local_zeta_url` setting or env var.
3. Add debug logging at:
   - request construction
   - serialized context generation
   - response receipt
   - rewrite application
4. Add a safe fallback to hosted mode.
5. Add toggles for experimental logging intensity.

#### Acceptance Criteria

- Zed can route native Zeta2 requests to a local server.
- Same editor session can be switched between hosted and local backend.
- Logs capture enough detail to replay requests.

### Workstream B: Native Request Introspection

#### Objective

Understand exactly what data the native path produces before trying to emulate it.

#### Deliverables

- request schema documentation
- example payload archive
- mapping from editor state to request fields

#### Tasks

1. Capture native hosted requests for diverse edit scenarios.
2. Document all request fields.
3. Identify:
   - cursor position representation
   - related file inclusion
   - edit history serialization
   - LSP-derived context presence
   - rewrite region metadata
4. Compare multiple requests from similar edit scenarios.
5. Identify what is deterministic vs heuristic.

#### Acceptance Criteria

- You can explain every field in a native request at a high level.
- You have representative samples for major edit types.

### Workstream C: Golden Dataset Collection

#### Objective

Build a corpus of real editing examples to guide reverse engineering and evaluation.

#### Deliverables

- labeled corpus of edit scenarios
- stored request/response artifacts
- replayable test fixtures

#### Dataset Design

Collect examples across:

- insert in middle of line
- replace identifier
- complete obvious syntax
- add function parameter and update body
- wrap expression / refactor local block
- add import and use symbol
- type-driven edits
- cross-file symbol usage
- multiline block insertion
- deletion / replacement scenarios

For each example, store:

- file before
- cursor location
- nearby project files if present
- recent edit history
- native request payload
- hosted response
- accepted/rejected outcome if known
- ground-truth resulting edit if available

#### Acceptance Criteria

- At least 200 high-signal examples
- Coverage across 3-5 languages or file types you care about
- Replayable fixtures usable in automated evaluation

### Workstream D: Prompt Compiler

#### Objective

Reconstruct a Zeta2-native prompt compiler that matches hosted behavior closely enough.

#### Deliverables

- serialized prompt generator
- tunable policy config
- prompt snapshots for each replay fixture

#### Prompt Compiler Inputs

- target file content
- cursor position
- editable region / rewrite window
- recent edits
- related-file snippets
- LSP symbol/type context
- token budget policy

#### Tunable Policy Parameters

- rewrite window size before cursor
- rewrite window size after cursor
- number of recent edits
- edit-history format
- number of related files
- snippet extraction policy
- ordering of context blocks
- truncation policy
- stop sequence policy

#### Implementation Approach

Build a structured intermediate representation first:

`EditorState -> PromptIR -> SerializedPrompt`

Where `PromptIR` makes the context selection explicit.

#### Acceptance Criteria

- Prompt compiler can recreate prompts for the replay corpus.
- Policy knobs can be varied automatically.
- Prompt snapshots are versioned and comparable.

### Workstream E: Output Interpretation and Safety Layer

#### Objective

Convert raw local Zeta2 output into safe, correct editor rewrites.

#### Deliverables

- robust parser for model outputs
- normalization layer
- safety guards against catastrophic edits

#### Key Responsibilities

- detect no-op / no-edit responses
- strip/control sentinel artifacts
- stop at correct terminator
- apply rewrite only within allowed region
- reject malformed outputs
- fail gracefully to "no suggestion"

#### Safety Rules

1. Never allow rewrite beyond the declared editable region unless explicitly intended.
2. Reject outputs missing required structural markers.
3. Reject suspicious giant deletions unless confidence is high and scenario justifies it.
4. Prefer no suggestion over bad suggestion.

#### Acceptance Criteria

- Known bad-output cases do not corrupt files.
- Output interpreter passes unit tests for all known failure modes.

### Workstream F: Local Inference Backend

#### Objective

Run Zeta2 locally with usable latency and stable output behavior.

#### Candidate Backends

- MLX
- llama.cpp
- vLLM
- Ollama-compatible serving only if it can preserve native contract cleanly

#### Initial Recommendation

Start with one backend optimized for:

- lowest engineering complexity
- good Apple Silicon performance
- deterministic serving behavior
- easy control over stop sequences and decoding

#### Inference Config Variables

- quantization level
- max output tokens
- temperature
- top-p
- repeat penalty
- stop sequences
- batching behavior
- streaming vs non-streaming

#### Phased Model Targets

1. Higher-quality reference local config
2. Faster daily-driver config

#### Acceptance Criteria

- Local backend runs stably on target machine
- Native local request -> response roundtrip works end to end
- Latency is measurable and tunable

### Workstream G: Replay Evaluation Harness

#### Objective

Make behavior comparable across hosted, local, prompt variants, and quantizations.

#### Deliverables

- automated replay runner
- scoring outputs
- summary reports

#### Metrics

- exact match on generated region
- token overlap / edit overlap
- structural validity
- rewrite safety
- estimated acceptance proxy
- latency
- first-token delay
- tail latency

#### Acceptance Criteria

- New prompt/compiler changes can be evaluated automatically
- Regression reports are available per experiment

### Workstream H: Performance and Product Tuning

#### Objective

Reach a latency/quality point suitable for daily use with debounce.

#### Areas to Tune

- debounce interval
- prompt size
- rewrite window size
- quantization level
- max predicted length
- local caching
- speculative early cutoff for poor generations

#### Product Success Criteria

- prediction feels fast enough not to interrupt flow
- bad suggestions are rare enough to keep feature enabled
- longer pauses are acceptable because debounce is intentional

---

## Project Phases

### Phase 0: Setup and Baseline

#### Objectives

- prepare repos
- baseline hosted behavior
- choose target machine/runtime

#### Tasks

- fork Zed
- set up dev environment
- identify Zeta2 path in code
- confirm current hosted edit prediction works
- establish logging conventions

#### Exit Criteria

- working local dev build of Zed
- baseline hosted behavior captured

### Phase 1: Native Hook Restoration

#### Objectives

- redirect native Zeta requests to local server
- keep hosted fallback

#### Tasks

- add configurable local endpoint
- implement local passthrough server
- verify end-to-end with hosted forwarding

#### Exit Criteria

- Zed native path can target local shim without changing user-visible behavior

### Phase 2: Corpus and Instrumentation

#### Objectives

- collect enough examples to stop guessing

#### Tasks

- instrument request/response logging
- capture diverse edit scenarios
- store fixtures in canonical format

#### Exit Criteria

- replay corpus exists and is usable

### Phase 3: Prompt Compiler v1

#### Objectives

- create first native prompt compiler
- support configurable policies

#### Tasks

- implement `PromptIR`
- implement serializer
- implement baseline policies
- generate prompts for fixtures

#### Exit Criteria

- local server can construct native-style prompts from corpus examples

### Phase 4: Output Interpreter v1

#### Objectives

- make returned edits safe and structurally correct

#### Tasks

- parser
- sentinel handling
- bounded rewrite application
- malformed output rejection

#### Exit Criteria

- known failure cases are contained safely

### Phase 5: Local Inference Integration

#### Objectives

- replace forwarded hosted inference with local inference

#### Tasks

- integrate backend runtime
- choose first quantized model config
- wire decoder settings
- validate output structure

#### Exit Criteria

- end-to-end fully local edit prediction works

### Phase 6: Differential Fitting and Search

#### Objectives

- fit prompt policy closer to hosted behavior

#### Tasks

- replay corpus through hosted/local
- compare outputs
- tune rewrite window, ordering, edit history, LSP inclusion
- iterate

#### Exit Criteria

- local behavior reaches target quality threshold on replay

### Phase 7: Latency Optimization

#### Objectives

- reach daily-driver responsiveness

#### Tasks

- experiment with 4-bit / 5-bit / 6-bit
- tune debounce
- tune max output tokens
- tune prompt trimming
- reduce slow-path cases

#### Exit Criteria

- local setup feels good enough in real editing sessions

### Phase 8: Stabilization and Documentation

#### Objectives

- make setup repeatable
- reduce fragility

#### Tasks

- document architecture
- document tuning knobs
- document known failure modes
- add regression tests

#### Exit Criteria

- a second machine/user could reproduce the setup

---

## Milestones

### Milestone 1: Native Local Routing

Zed can send native Zeta requests to a local server.

### Milestone 2: Corpus Complete

You have a strong real-world replay dataset.

### Milestone 3: Prompt Compiler Works

Local server can produce native-style prompts from editor state.

### Milestone 4: Safe Output Application

Malformed outputs no longer threaten file integrity.

### Milestone 5: First True Local Prediction

A local model produces usable in-editor predictions.

### Milestone 6: Replay Parity Threshold

Local system achieves acceptable overlap with hosted behavior.

### Milestone 7: Daily Driver Threshold

Latency + quality are good enough to use as primary edit prediction.

---

## Team / Roles

For a solo project, map roles mentally:

### Tech Lead / Architect

- owns architecture and sequencing
- decides when to move phases

### Editor Integration Engineer

- patches Zed
- owns client-side instrumentation

### Inference Engineer

- owns local serving and runtime integration

### Evaluation Engineer

- owns corpus, replay, metrics, and experiment tracking

These can all be the same person, but they are distinct work modes.

---

## Deliverables

### Code Deliverables

- forked Zed with native local Zeta routing
- local Zeta server/shim
- prompt compiler
- output interpreter
- replay harness
- benchmark scripts

### Documentation Deliverables

- architecture doc
- request schema notes
- prompt compiler design
- evaluation methodology
- latency/quality benchmark report
- ops/setup guide

### Decision Deliverables

- chosen runtime/backend
- chosen quantization level
- chosen debounce defaults
- go/no-go on daily-driver viability

---

## Technical Design Details

### Canonical Data Model

#### EditorState

Should minimally include:

- file path
- file content
- cursor position
- selection range
- recent edits
- related files
- LSP definitions/snippets
- metadata about language/project

#### PromptIR

Should include:

- suffix context
- prefix context
- editable region
- target file metadata
- edit history blocks
- related file blocks
- symbol/type blocks
- truncation markers

#### PredictionResult

Should include:

- raw model output
- parsed output
- normalized rewrite
- confidence heuristics
- timing metadata
- validation flags

---

## Experiment Plan

### Hypotheses to Test

#### H1

Rewrite window selection matters more than related-file count.

#### H2

Correct output parsing matters more than quantization level for early quality.

#### H3

4-bit will be acceptable once prompt and output semantics are correct.

#### H4

Small debounce is enough to make local Zeta2 feel viable.

#### H5

Including too much edit history hurts more than helps after a threshold.

---

### Experiment Order

#### Experiment Group 1: Plumbing

- hosted passthrough via local shim
- verify zero/near-zero quality regression

#### Experiment Group 2: Output Safety

- malformed output injection
- deletion-heavy cases
- sentinel cases

#### Experiment Group 3: Prompt Policy Search

- rewrite window sweep
- context ordering sweep
- edit history count sweep
- related-file inclusion sweep

#### Experiment Group 4: Inference Search

- Q4
- Q5
- Q6
- optional higher precision reference

#### Experiment Group 5: UX Tuning

- debounce sweep
- max output length sweep
- long-pause behavior

---

## Metrics

### Quality Metrics

- suggestion shown rate
- structurally valid response rate
- accepted suggestion proxy
- exact edit overlap
- partial overlap
- catastrophic failure rate
- no-suggestion rate

### Latency Metrics

- debounce duration
- request serialization time
- network/IPC overhead
- TTFT
- completion finish time
- p50 / p95 / p99 latency

### Safety Metrics

- giant deletion rate
- malformed sentinel leakage rate
- out-of-bounds rewrite attempt rate

---

## Success Criteria

### Minimum Success

- local native Zeta2 works end to end
- suggestions are mostly correct in simple cases
- feature is safe enough not to risk file corruption

### Strong Success

- local setup is pleasant enough to use daily with debounce
- catastrophic failures are rare
- replay metrics show meaningful parity with hosted behavior

### Stretch Success

- local system becomes preferred workflow for private/offline use
- latency feels near-hosted in practice on target hardware

---

## Risks

### Technical Risks

#### 1. Hidden Server-Side Behavior in Hosted Zeta2

There may be non-obvious processing not visible from the client.

Mitigation: use a local forwarding shim first; log aggressively; isolate what must be reproduced.

#### 2. Prompt Mismatch Mistaken for Quantization Loss

You may blame 4-bit when the real problem is context formulation.

Mitigation: test higher-precision local reference before judging quantization.

#### 3. Output Contract Mismatch

Model may produce structurally plausible but editor-dangerous outputs.

Mitigation: strict safety layer and bounded rewrite rules.

#### 4. Native Path Churn in Upstream Zed

Internal APIs may change.

Mitigation: keep fork diff small and well-isolated.

#### 5. Latency Too Slow for Comfortable Use

Even debounced local inference may disappoint.

Mitigation: tune prompt size, output length, debounce, and quantization.

### Product Risks

#### 1. Good Technical Parity but Bad Subjective Feel

Metrics may look okay while UX still feels laggy or brittle.

Mitigation: do real editing trials, not just replay tests.

#### 2. Feature Becomes Too Experimental to Trust

Users may disable it after a few bad edits.

Mitigation: bias toward "no suggestion" over risky suggestion.

---

## Resourcing / Time Estimate

For one strong engineer working part-time to focused-half-time, rough estimate:

- Phase 0-1: 3-6 days
- Phase 2: 4-8 days
- Phase 3-4: 1-2 weeks
- Phase 5: 3-7 days
- Phase 6: 1-3 weeks
- Phase 7-8: 1-2 weeks

Rough total:

- best case prototype: 2-3 weeks
- credible daily-driver candidate: 4-8 weeks

That assumes strong familiarity with Rust, inference tooling, and editor internals.

---

## Dependencies

- Zed source fork
- local inference backend/runtime
- target Mac hardware
- logging + artifact storage
- corpus examples from real editing sessions

---

## Rollout Plan

### Stage 1: Internal-Only Debug Use

Use only for instrumentation and evaluation.

### Stage 2: Personal Daily-Driver Shadow Mode

Show local suggestions, but compare silently against hosted where possible.

### Stage 3: Primary Local Mode

Use local as default, hosted as fallback.

### Stage 4: Optional OSS Cleanup

Only after architecture stabilizes.

---

## Operational Practices

- Version all prompt compiler configs.
- Store replay artifacts by commit SHA.
- Never mix latency experiments with prompt-policy experiments in the same run.
- Maintain a small "nasty cases" regression suite:
  - giant deletion case
  - raw sentinel case
  - multiline refactor case
  - cross-file symbol case

---

## Recommended First Implementation Path

1. Fork Zed.
2. Reintroduce configurable native predict-edits backend routing.
3. Build a passthrough local shim that forwards to hosted Zeta2 and logs everything.
4. Collect a replay corpus.
5. Implement output parser/safety layer first.
6. Implement prompt compiler second.
7. Swap in local inference only after the replay/evaluation loop exists.
8. Tune quantization and debounce last.

That is the highest-leverage order.

---

## Open Questions

- How much hidden hosted-side normalization exists?
- Which local runtime gives the best combination of Apple Silicon speed and decoding control?
- What rewrite-window policy matters most?
- How much LSP context is truly load-bearing?
- At what debounce interval does local become subjectively "good enough"?

These should be answered experimentally, not by design debate.

---

## Final Recommendation

Proceed.

This is a hard but very plausible project, and the right architecture is to treat Zed's native Zeta2 path as something to preserve and instrument, not replace.

The best plan is:

- fork minimally
- instrument deeply
- collect a corpus
- fit the prompt/compiler against hosted behavior
- then optimize local inference

That gives you the best chance of ending up with a real daily-driver instead of an impressive but brittle demo.
