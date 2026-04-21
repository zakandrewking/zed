# Agent Guidelines

This file defines how coding agents should work in this repository.

It complements, but does not replace:

- `PROJECT_PLAN.md`
- `EXECUTION_PLAN.md`
- `.rules`
- `docs/AGENTS.md` for documentation-only work

If a more specific `AGENTS.md` exists in a subdirectory, treat that file as the narrower-scope override for work in that area.

## Scope

Use this file for:

- Rust code changes
- tooling and automation
- agent workflow
- planning and execution
- verification strategy

Do not use this file to override crate-specific technical guidance already captured in `.rules` or deeper-scoped agent files.

## Core Operating Model

Agents working on large projects should optimize for legibility, verification, and incremental progress.

That means:

- work depth-first on the smallest slice that unlocks the next slice
- keep tasks narrow enough to verify
- make the system more inspectable when blocked
- prefer reusable tools over repeated manual debugging
- preserve momentum by choosing the next unblocked step instead of stopping early

For large changes, start by identifying:

1. the immediate next artifact
2. how it will be verified
3. what likely blocks it
4. what fallback path exists if the first approach fails

## Execution Loop

Use this loop for substantial work:

1. Restate the current goal in one sentence.
2. Identify the smallest next deliverable.
3. Define the verification method before editing code.
4. Implement the smallest viable change.
5. Run the narrowest useful verification.
6. Record what changed, what passed, and what remains.
7. Commit the completed slice if it is coherent.
8. Continue to the next unblocked slice whenever possible.

Do not treat "I understand the code now" as a milestone. A milestone must produce an artifact, a passing check, or a written decision.

## Planning

For non-trivial work:

- keep the product/architecture goal in `PROJECT_PLAN.md`
- keep sequencing, checkpoints, blockers, and fallback paths in `EXECUTION_PLAN.md`
- update the execution plan when reality diverges from the original plan

If a setback happens:

- do not thrash across multiple approaches at once
- reduce scope until the next step is testable
- capture the failed assumption explicitly
- choose the next best unblocked path

## Verification Requirements

Agents must establish how work will be tested before declaring progress complete.

Use these verification layers where applicable:

### 1. Unit-Level Verification

Use for:

- parsers
- data transforms
- edge-case handling
- request/response shape validation

### 2. Fixture-Based Verification

Use for:

- replayable behavior
- prompt/compiler work
- regression protection
- safety cases

### 3. Integration Verification

Use for:

- client/server routing
- end-to-end request/response behavior
- fallback logic

### 4. Manual Smoke Verification

Use for:

- editor UX
- debounce feel
- interactive behavior not yet captured in automation

Manual checks are useful, but they are not enough on their own when an automated or fixture-based check is feasible.

Every phase should answer:

- What proves this works?
- What output proves it failed?
- What will catch regressions later?

## Tool-Building Bias

Be clever about building tools that increase leverage across the whole project.

If the same manual debugging step happens twice, consider turning it into a script, fixture generator, test helper, or logging toggle.

Prioritize small internal tools that make the system more legible:

- request/response capture
- local stub servers
- fixture normalizers
- replay runners
- prompt snapshot diffing
- safety regression corpora
- experiment runners

Prefer tools that:

- are deterministic
- produce diffable artifacts
- can run in CI or locally
- reduce repeated human judgment

## Progress Expectations

Agents should continue making progress whenever possible.

That means:

- if the main task is blocked, work on the best adjacent unblocker
- if implementation is blocked, improve verification or tooling
- if verification is blocked, improve observability
- if a large task is fuzzy, break it into a narrower one and complete that

Only stop to ask the user when:

- a decision has material product or architectural consequences
- secrets, credentials, or external access are required
- two reasonable paths exist with different tradeoffs that the user should choose between
- continuing would risk damaging important work

## Commit Cadence

Commit work along the way in small, coherent slices.

Preferred commit boundaries:

- one logical code change
- one planning artifact update
- one test/tooling addition

Each commit should be useful on its own and leave the tree in a more understandable state.

Avoid:

- giant mixed-purpose commits
- delaying all commits until the very end
- committing broken exploratory edits unless explicitly intended

Before committing:

- confirm what changed
- make sure the change matches the stated slice
- record what was verified and what was not

## Code Change Expectations

When editing code:

- follow `.rules`
- prefer existing files over creating new ones unless a new component is justified
- keep changes narrow until behavior is verified
- add tests when the behavior can be captured
- add logging or fixtures when tests are not yet enough

For docs-only work under `docs/`, follow `docs/AGENTS.md`.

## What Agents Should Read First

For most code tasks, start with:

1. the relevant section of `.rules`
2. the relevant local files and tests
3. `PROJECT_PLAN.md` and `EXECUTION_PLAN.md` if the task is part of the Local Zeta2 work

Do not front-load broad repo exploration without a concrete question to answer.

## What I Need From The User

Group requests instead of asking piecemeal.

### Product / Direction Decisions

Ask for these together when needed:

- preferred milestone ordering if priorities changed
- acceptable tradeoffs between latency, safety, and fidelity
- what counts as "good enough" for the current phase

### Environment / Access

Ask for these together when needed:

- API keys or credentials
- external service access
- hardware/runtime constraints
- permission to use a specific external dependency or service

### Validation / Acceptance

Ask for these together when needed:

- acceptance criteria for subjective behavior
- examples of good and bad outcomes
- which regressions are unacceptable
- whether a manual smoke test is sufficient for the current slice

### Coordination / Review

Ask for these together when needed:

- whether to optimize for fast iteration or minimal churn
- whether to keep work in draft/instrumentation form longer
- whether to stop after the current milestone or continue to the next one

## Current Project Bias: Local Zeta2 in Zed

For the current project, agents should prefer this order:

1. native routing correctness
2. instrumentation and capture
3. replayability
4. output safety
5. prompt/compiler work
6. local inference tuning

Do not jump to prompt tweaking or quantization experiments before native routing, capture, and replay are in place.

## Good Agent Behavior In This Repo

- explain the next step before doing substantial work
- verify work, not just edits
- leave artifacts that help the next session
- commit coherent slices
- tighten scope when blocked

## Bad Agent Behavior In This Repo

- making broad speculative changes without a verification plan
- treating manual inspection as the only test
- repeatedly asking the user for one missing detail at a time
- changing unrelated areas "while here"
- doing large uncommitted stretches of work
