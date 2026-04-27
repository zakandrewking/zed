# Execution Plan: Local Zeta2 in Zed

## Purpose

This document translates [PROJECT_PLAN.md](/Users/zak/repos/zed/PROJECT_PLAN.md) into an execution sequence that is resilient to setbacks.

It exists to answer four operational questions:

1. What is the next concrete thing to do?
2. How do we know it worked?
3. What do we do if the current hypothesis fails?
4. What work is blocked versus parallelizable?

---

## Working Rules

- Keep the fork diff small until native local routing works.
- Prefer instrumentation over speculation.
- Do not optimize inference before the replay/evaluation loop exists.
- Treat "no suggestion" as safer than malformed suggestion.
- Every phase must end with a concrete artifact, not just understanding.
- If a phase stalls, reduce scope rather than broadening it.
- Every phase must define how it is tested before it is called done.
- Prefer building reusable tooling once over repeating manual debugging steps.

---

## Current Checkpoint

Last updated: 2026-04-27

Current branch:
- `local-zeta2-native-routing` on `git@github.com:zakandrewking/zed.git`

Verified artifacts now in place:
- native local routing hook and capture flow
- local native stub server with hosted passthrough and local no-op modes
- imported native capture fixtures under `crates/edit_prediction_cli/evals-generated/native-captures/20260421-111437`
- capture replay reports for default and all Zeta formats
- output safety replay command and report
- raw model-output normalization mode in the local stub
- dynamic raw model-output command mode in the local stub, with prompt/request JSON stdin options and timeout handling

Latest verified commands:
- `cargo test -p edit_prediction_cli model_command -- --nocapture`
- `cargo test -p edit_prediction_cli model_output_response -- --nocapture`
- `cargo test -p edit_prediction_cli capture_ -- --nocapture`
- `./script/clippy -p edit_prediction_cli`
- `target/debug/ep replay-output-safety --directory crates/edit_prediction_cli/evals-generated/native-captures/20260421-111437 -o crates/edit_prediction_cli/evals-generated/native-captures/20260421-111437/output-safety-report.md`
- live smoke: `ep serve-stub --model-command /bin/sh --model-command-arg=-c --model-command-arg 'sleep 2' --model-command-timeout-ms 10 --once` rejected a captured request with `/bin/sh timed out after 10 ms`

Immediate next useful milestone:
- add an offline replay runner that invokes the same model-command adapter against captured fixtures and reports latency plus output safety.

Fallback if local backend integration stalls:
- add file-watching or stdin-driven raw output mode first, then connect a real model runtime behind that stable boundary.

---

## Verification Strategy

### Verification Principles

- Verify the smallest layer that can fail before testing the full stack.
- Prefer deterministic fixture-based tests over purely interactive checks.
- Keep one manual smoke test per phase, but do not rely on manual testing alone.
- When adding instrumentation, also define what output means success vs failure.
- If a behavior cannot yet be asserted automatically, capture the missing assertion as a tool/task.

### Verification Layers

#### Layer 1: Unit and Parsing Tests

Use for:

- output parser behavior
- editable-range enforcement
- prompt serialization invariants
- request/response schema handling

Proof of success:

- deterministic tests pass for known good and known bad cases

#### Layer 2: Fixture Replay Tests

Use for:

- prompt compiler changes
- output interpretation changes
- compatibility between native request shapes and local responses

Proof of success:

- the same captured fixture can be replayed repeatedly with stable results

#### Layer 3: Local Integration Tests

Use for:

- local endpoint routing
- end-to-end request/response handling
- hosted fallback behavior

Proof of success:

- Zed client code can target a local stub/shim and receive a valid native response

#### Layer 4: Manual Editor Smoke Tests

Use for:

- debounce feel
- UI behavior
- subjective quality checks

Proof of success:

- editor behavior matches the intended mode without visible corruption or obvious regressions

### What Must Be Verified Per Phase

#### Phase 1: Native Path Mapping

- verify file/function ownership by tracing the actual request path in code
- record the exact intended interception point in this document or code notes

#### Phase 2: Native Local Routing

- automated: local integration test or stub-backed test for native routing
- manual: toggle local vs hosted and confirm both paths still work

#### Phase 3: Instrumentation and Capture

- automated: logs/artifacts are emitted with expected fields
- manual: inspect one captured request/response pair for completeness

#### Phase 4: Local Shim

- automated: passthrough request produces equivalent hosted behavior on a sample fixture
- manual: inspect shim logs and response shape

#### Phase 5: Replay Harness

- automated: fixture replay command succeeds on a seeded corpus
- manual: inspect one generated comparison report

#### Phase 6: Output Safety Layer

- automated: malformed outputs are rejected in tests
- manual: verify bad outputs degrade to "no suggestion" rather than broken rewrites

#### Phase 7: Prompt Compiler

- automated: prompt snapshots are reproducible for the same fixture
- manual: inspect prompt diffs for a few representative fixtures

#### Phase 8: Local Inference

- automated: local backend can serve at least one end-to-end fixture successfully
- manual: verify one real editor flow produces a usable suggestion

#### Phase 9: Differential Tuning

- automated: experiment results are recorded and comparable
- manual: confirm chosen settings still feel acceptable in live editing

---

## Tooling Plan

Build small tools early to reduce repeated manual work.

### Tool 1: Native Request Capture Toggle

Purpose:
- capture native request/response artifacts from the Zed client path

Minimum shape:
- env-var or debug-setting gated
- writes JSON artifacts with timestamp, buffer metadata, request, response, and editable range

Why early:
- this unblocks reverse engineering, replay, and debugging simultaneously

### Tool 2: Local Native Stub Server

Purpose:
- accept native `PredictEditsV3Request` payloads and return controlled `PredictEditsV3Response` fixtures

Minimum shape:
- simple local HTTP server
- canned response mode
- echo/log mode

Why early:
- lets us verify native routing before building the real shim or local inference backend

### Tool 3: Fixture Recorder

Purpose:
- normalize captured request/response artifacts into replay fixtures

Minimum shape:
- stable file format
- one command to ingest captured logs into fixtures

Why early:
- avoids ad hoc artifact cleanup later

### Tool 4: Replay Runner

Purpose:
- replay fixtures against prompt/compiler/output variants

Minimum shape:
- runs a batch of fixtures
- stores outputs and metadata
- returns pass/fail summary

Why early:
- prevents prompt and parser work from turning into anecdotal tuning

### Tool 5: Prompt Snapshot Diff

Purpose:
- compare prompt output between compiler versions or policy settings

Minimum shape:
- fixture in, prompt out
- diff-friendly deterministic output

Why early:
- makes prompt changes inspectable instead of opaque

### Tool 6: Output Safety Test Corpus

Purpose:
- maintain a regression suite of malformed or dangerous outputs

Minimum shape:
- examples for giant deletions, sentinel leakage, malformed markers, and out-of-bounds rewrites

Why early:
- safety failures are cheap to reintroduce unless they are fixture-tested

### Tool 7: Experiment Runner

Purpose:
- run prompt/output/inference sweeps without changing code by hand each time

Minimum shape:
- config-driven run matrix
- report output directory keyed by timestamp or commit SHA

Why later:
- useful after routing, capture, and replay exist

---

## Phase Order

### Phase 1: Native Path Mapping

Goal:
Understand the exact native Zeta request flow and choose the cleanest interception point for local routing.

Primary artifact:
- request path notes with file/function references

Exit criteria:
- we can point to the current native request builder
- we can point to the current hosted request send path
- we know where a local native URL can be injected with minimal churn

Current findings:
- native Zeta request assembly lives in [crates/edit_prediction/src/zeta.rs](/Users/zak/repos/zed/crates/edit_prediction/src/zeta.rs)
- provider/model selection lives in [crates/edit_prediction/src/edit_prediction.rs](/Users/zak/repos/zed/crates/edit_prediction/src/edit_prediction.rs)
- generic self-hosted providers use [crates/edit_prediction/src/open_ai_compatible.rs](/Users/zak/repos/zed/crates/edit_prediction/src/open_ai_compatible.rs)
- the hosted native request/response schema is defined in [crates/cloud_llm_client/src/predict_edits_v3.rs](/Users/zak/repos/zed/crates/cloud_llm_client/src/predict_edits_v3.rs)

Fallback if blocked:
- if a settings-based hook is invasive, add an environment-variable override first
- if the send path is coupled to hosted auth, add a local unauthenticated branch before designing full settings UI

### Phase 2: Native Local Routing

Goal:
Make Zed send native predict-edits requests to a user-controlled local endpoint.

Primary artifact:
- working local routing patch

Exit criteria:
- native Zeta requests can target a local server
- hosted fallback still works
- routing can be toggled without large code changes

Preferred implementation order:
1. environment variable override
2. internal config field
3. user-facing settings/UI

Fallback if blocked:
- if a true Zeta-specific setting touches too many surfaces, keep the override internal and unblock instrumentation first

### Phase 3: Instrumentation and Capture

Goal:
Capture enough native requests and responses to stop guessing.

Primary artifact:
- request/response logs and replay fixtures

Exit criteria:
- replayable fixtures exist for representative edit classes
- prompt input and editable range are captured
- rewrite application can be correlated with response payloads

Fallback if blocked:
- if full artifact capture is too noisy, capture sampled sessions with explicit debug toggles

### Phase 4: Local Shim

Goal:
Insert a local server that accepts the native request shape and forwards to hosted first.

Primary artifact:
- passthrough shim with logging

Exit criteria:
- local shim forwards native requests with no material behavior regression
- local logs are sufficient for offline inspection

Fallback if blocked:
- if standalone server work slows progress, use a minimal local endpoint that only logs and echoes fixture-based responses

### Phase 5: Replay Harness

Goal:
Make native/local behavior comparable in a repeatable way.

Primary artifact:
- replay runner and fixture format

Exit criteria:
- fixtures can be replayed automatically
- prompt/output variants can be compared on the same examples

Fallback if blocked:
- start with file-based fixture replay before building full metrics/reporting

### Phase 6: Output Safety Layer

Goal:
Guarantee malformed model output does not corrupt buffers.

Primary artifact:
- parser/normalizer tests and safety guards

Exit criteria:
- known bad outputs are rejected safely
- rewrite bounds are enforced

Fallback if blocked:
- temporarily bias hard toward rejection and "no suggestion"

### Phase 7: Prompt Compiler

Goal:
Reconstruct enough of the native contract to get useful local behavior.

Primary artifact:
- PromptIR and serializer

Exit criteria:
- prompts can be generated deterministically from fixtures
- policy knobs are configurable and testable

Fallback if blocked:
- keep the first version narrow: single-file context, bounded history, limited related files

### Phase 8: Local Inference

Goal:
Replace hosted forwarding with true local inference.

Primary artifact:
- one working local backend integration

Exit criteria:
- end-to-end local prediction works
- latency is measurable

Fallback if blocked:
- keep hosted-through-shim as the reference path while iterating on backend/runtime

### Phase 9: Differential Tuning

Goal:
Fit local behavior toward hosted behavior.

Primary artifact:
- experiment results and chosen defaults

Exit criteria:
- acceptable replay quality threshold
- acceptable daily-driver threshold

Fallback if blocked:
- separate correctness from latency and tune them independently

---

## Decision Gates

### Gate A: Routing Patch Complete

Required before moving on:
- native request path identified
- local URL injection implemented
- hosted fallback preserved

### Gate B: Fixture Corpus Exists

Required before serious prompt work:
- representative examples captured
- fixture format stable enough to replay

### Gate C: Safety Layer Exists

Required before local inference becomes default:
- malformed output rejection implemented
- rewrite bounding verified

### Gate D: Replay Loop Exists

Required before optimization work:
- prompt/compiler changes can be compared automatically

---

## Immediate Next Actions

1. Finish mapping the native request send path and the minimal routing hook.
2. Implement a Zeta-native local endpoint override, preferably as an env var first.
3. Build a minimal local native stub server for routing verification.
4. Verify the override can hit the stub without regressing hosted fallback behavior.
5. Add targeted logging around request construction, response receipt, and editable range application.

---

## Current Focus

Current task:
- patch native Zeta routing, not generic OpenAI-compatible routing

Current hypothesis:
- the cleanest first step is to add a native local URL branch near the V3 request send path in `crates/edit_prediction/src/zeta.rs` or `crates/edit_prediction/src/edit_prediction.rs`

What would disprove this:
- routing is too coupled to hosted auth/usage plumbing
- editable range handling assumes hosted-only response semantics in a way that local routing cannot satisfy directly

If disproved:
- add a compatibility adapter that accepts the native `PredictEditsV3Request`/`PredictEditsV3Response` shape locally and leaves the rest of the client unchanged

Current verification target:
- prove native requests can be redirected to a local stub that speaks the V3 schema before changing prompt/compiler/inference behavior

---

## Tracking Format

Use this document as the source of truth for execution state:

- `Pending`: not started
- `Active`: current focus
- `Blocked`: waiting on an answered question or missing artifact
- `Done`: exit criteria met

Current state:
- Phase 1: `Active`
- Phase 2: `Pending`
- Phase 3: `Pending`
- Phase 4: `Pending`
- Phase 5: `Pending`
- Phase 6: `Pending`
- Phase 7: `Pending`
- Phase 8: `Pending`
- Phase 9: `Pending`
