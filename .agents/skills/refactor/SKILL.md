---
name: refactor
description: Full-workspace code quality analysis and refactoring with validation
user-invocable: true
allowed-tools: Read, Grep, Glob, Bash(cargo check*), Bash(cargo clippy*), Bash(cargo test*), Bash(cargo fmt*), Bash(cargo tree*), Bash(cargo audit*), Bash(cargo outdated*), Bash(cargo machete*), Bash(cargo deny*), Bash(cargo mutants*), Bash(cargo tarpaulin*), Bash(cargo udeps*), Bash(git status*), Bash(git diff*), Bash(git stash*), Bash(git log*), Bash(wc *), Bash(grep*), Edit, Write, Task
---

# Refactor

Full-workspace code quality analysis and incremental refactoring with continuous validation.

This skill is intended for **periodic whole-codebase refactoring passes**, not per-branch or per-diff cleanup. It does not accept `--diff` / `--staged` / `--since` arguments.

Platform note: agents that support frontmatter permissions may use `allowed-tools` and `user-invocable`. Other agents should ignore unsupported fields and follow the workflow below. When platform-specific orchestration or memory features are unavailable, degrade gracefully and continue with the equivalent local workflow.

Arguments (optional):
- Path or crate: `/refactor crates/app` — limit scope to one directory
- No arguments — analyze the entire workspace

## Process

### Phase 1: Exploration

1. Read project instruction and planning files if present, for example `AGENTS.md`, `CLAUDE.md`, and `PRD.md`. Also read any project or agent memory/notes files if present for `intentional_placeholder_*`, `feedback_*`, or similar notes that inform what to skip.
2. Collect baseline metrics — run in parallel, each one optional (if a tool is absent, emit an info line in the report, do not fail):
   - `cargo clippy --workspace --all-targets 2>&1` — warning count.
   - `cargo test --workspace 2>&1` — green baseline? If tests fail or code doesn't compile, report and stop.
   - Glob + `wc -l` — LOC per crate, files > 400 LOC.
   - `cargo tree -d` — duplicated dependencies.
   - `cargo audit` — CVE list. If severity ≥ Medium, flag but continue.
   - `cargo outdated --root-deps-only` — major-behind deps.
   - `cargo machete` — unused `[dependencies]` entries.
   - `cargo udeps` — unused deps (secondary, if machete absent).
   - `git log --numstat --since="6 months ago"` → top-10 files by churn (hotspot candidates for splitting).
   - `grep -rn "TODO\|FIXME\|HACK\|XXX" --include="*.rs"` — **from repo root, not from `crates/`** (see Rules below).
   - Cyclomatic-complexity proxy: `grep -c "match\|if\|for\|while"` on the ten largest `.rs` files.

### Phase 2: Diagnosis

Analyze code across the categories below. For large scope, use parallel subagents or parallel analysis passes if the platform supports them, ideally one per category or related group of categories.

#### Analysis checklist

**Architecture (SOLID)**
- [ ] Files > 400 LOC with multiple public types — splitting candidates.
- [ ] Modules with > 1 responsibility.
- [ ] Hard dependencies between modules (concrete types where a trait would decouple).
- [ ] Open/Closed violations: `match` on types that are expected to grow.

Legitimate exceptions to "1 file = 1 type": Error+ErrorKind pairs, Builder+Target, small private helpers (< 30 LOC), related DTO families (Request/Response), typestate marker types, newtype wrapper collections.

**Module coupling / cohesion**
- [ ] Crate dependency graph is unidirectional (core ← mid ← app).
- [ ] No circular deps between modules within a crate.
- [ ] No excessive `pub` — types/functions public without need (downgrade to `pub(crate)` / `pub(super)`).
- [ ] Crate does not know implementation details of another crate (leaky abstraction).
- [ ] Changing one module does not cascade into unrelated modules.

**Duplication (DRY)**
- [ ] Repeated code blocks (> 5 lines, > 2 occurrences).
- [ ] Similar `match` patterns across different files.
- [ ] Copy-pasted error handling.

**Dead code**
- Grep usages **from repo root**, not from `crates/`. Binary crate in `src/` is a legitimate caller (previous incident with `check_git_available` / `ram_usage_percent` was caused by grepping only `crates/`).
- [ ] Unused `use`, functions, types, constants.
- [ ] Commented-out code.
- [ ] Unreachable branches.
- Confidence tagging:
  - `HIGH` — unused private / `pub(crate)` in leaf module, no callers.
  - `MEDIUM` — `pub` in inner crate, no callers found via grep.
  - `LOW` — enum variant, trait impl, or item with `#[allow(dead_code)]` nearby — likely intentional placeholder.
- Skip items already noted as intentional in project or agent memory/notes files (e.g. `ForceSave`, dead enum variants in `vfs_state.rs`/`keyboard.rs`, `format_bytes` x3, `ConflictResolution` x3 across layers).

**Performance**
- [ ] Unnecessary `.clone()` / `.to_string()` / `.to_owned()` where a reference suffices.
- [ ] O(n²) loops (nested iterations over collections).
- [ ] Repeated HashMap/Vec lookups inside a loop.
- [ ] Allocations in hot paths (see also Allocations category below).

**Error boundaries**
- [ ] Consistent error types at crate boundaries (`From` / `thiserror`).
- [ ] `unwrap()` / `expect()` in production code — replace with `?` or explicit handling.
- [ ] Lost error context (`.map_err(|_| ...)` without information).
- [ ] Panic instead of Result at input boundaries (user, DB, network).

**Security**
- Integrate `cargo audit` / `cargo deny check licenses` output here (do not split into a separate step).
- [ ] SQL queries via string concatenation instead of parameterized.
- [ ] Secrets / tokens hardcoded or logged.
- [ ] Unvalidated user input (path traversal, XSS).
- [ ] `fs::*(user_input_path)` without `canonicalize()` + `starts_with()` check — path traversal risk.
- [ ] `Command::new(user_input)` or shell-expansion of user input — command injection.
- [ ] Magic numbers and hardcoded config that should be parameters.

**Style and idioms**
- [ ] Inconsistent naming.
- [ ] Missing `#[must_use]` on pure / builder methods.
- [ ] Over-abstractions with no usage.
- [ ] Uncovered edge cases in tests (empty input, errors, overflow).

**Concurrency** *(Rust-specific)*
- [ ] `Arc<Mutex<T>>` where locked guard is used mostly for reads (grep the ratio of `.lock()` read accesses vs mutations) — `Arc<RwLock<T>>` candidate.
- [ ] `.await` inside a `.lock()` guard — deadlock risk.
- [ ] `tokio::spawn` without an abort handle for long-running tasks (leaks on shutdown).
- [ ] Shared mutable state without `Send` / `Sync` markers where the compiler would accept them.

**API stability**
- [ ] `pub` enum without `#[non_exhaustive]` on a cross-crate boundary.
- [ ] `pub` in a child module re-exported via `pub use` only for one caller — downgrade candidate to `pub(crate)`.
- [ ] Breaking changes to `pub` API without a version bump.

**Allocations / hot path** *(TUI-specific)*
- [ ] `.to_string()` / `format!` / `Vec::new()` / `vec![]` inside `render_*`, `tick_*`, `draw_*`, `update_*` functions.
- [ ] Config cloned per frame — verify `Arc<Config>` pattern is used.
- [ ] `Cow<str>` candidates: functions taking `&str` that sometimes need an owned `String`.

**Testing quality**
- [ ] Non-hermetic tests: `std::env::set_var`, global state mutation inside `#[test]`.
- [ ] `#[ignore]` without a comment explaining why.
- [ ] Assertions without a message on complex equality (`assert_eq!` on a large struct).
- [ ] Coverage gap in critical paths (if `cargo-tarpaulin` is available, highlight files with < 50%).

**Observability**
- [ ] `println!` / `eprintln!` in production code (not `#[test]` / examples) — should be `log::*`.
- [ ] `let _ = <Result>` — errors swallowed without `log::warn!`.
- [ ] Long-running operations without tracing span or progress.

### Phase 3: Report and prioritisation

Each finding gets two tags:

```
Impact:  HIGH / MED / LOW
  HIGH  — > 100 LOC reduction OR removes a bug class OR unblocks a perf fix
  MED   — 20–100 LOC reduction OR consolidates a recurring pattern
  LOW   — < 20 LOC OR stylistic

Risk:    SAFE / CAREFUL / BREAKING
  SAFE      — private internals, no API surface change, covered by tests
  CAREFUL   — pub(crate) API, hot path, thread boundary
  BREAKING  — pub API, schema/config format, semantic change
```

Default execution order: `HIGH-SAFE → MED-SAFE → HIGH-CAREFUL → LOW-SAFE → MED-CAREFUL`. `BREAKING` findings are shown but never executed without explicit user approval.

Output format (terminal, do not create report files):

```
## Findings (ROI-sorted)

### 🟢 HIGH-SAFE (execute automatically)
1. **<short title>** — path/to/file.rs, path/to/other.rs
   Why: <one-sentence justification + evidence>.
   Effort: ~N min · Risk: SAFE (<reason>).

### 🟡 MED-CAREFUL (propose, require approval)
2. **<short title>** — path/to/file.rs
   Why: …
   Effort: ~N min · Risk: CAREFUL (<reason>).

### 🔴 BREAKING (report only, do not execute)
3. …

### 📦 Ecosystem
- cargo-audit: N advisories.
- cargo-outdated: N major-behind deps.
- cargo-machete: N unused crates.
- cargo-tarpaulin: workspace coverage X% (only if available).

## Category counts

| Category     | Found | HIGH | MED | LOW |
|--------------|------:|-----:|----:|----:|
| Architecture |   …   |  …   |  …  |  …  |
| …            |   …   |  …   |  …  |  …  |

Baseline: X clippy warnings, Y workspace LOC, Z tests passing.
```

**Auto-mode:**
- If auto-mode is detected by surrounding context, do not pause for interactive questions.
- Auto-execute all `HIGH-SAFE` and `MED-SAFE` findings.
- Leave `CAREFUL` and `BREAKING` in the report for the user to review.

**Non-auto:**
- Ask the user which categories to fix using the platform's interactive prompt mechanism, with `SAFE` findings preselected when supported.
- Ask for budget: quick cleanup / full refactoring.

### Phase 4: Execution

For each fix:
1. Read the file, understand context.
2. Make the change via `Edit`.
3. `cargo check` — still compiles?
4. If behaviour is affected — `cargo test --workspace`.
5. Move to the next fix.

On compilation or test failure — revert the change and move on.

Commit boundaries: do **not** auto-commit. Let the user review the final diff and commit.

### Phase 5: Validation

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Compare with baseline: warnings removed, LOC deleted/changed, tests still green.

## Rules

- Grep for dead code **from the repository root**, not from `crates/`. Binary crate in `src/` is a legitimate caller.
- Before proposing a dead-code or refactor finding, cross-check project or agent memory/notes files for `intentional_placeholder_*` entries and skip those silently.
- If an ecosystem tool (`cargo-machete`, `cargo-mutants`, `cargo-audit`, etc.) is not in PATH, emit an info line and continue — do not fail.
- `BREAKING` findings are reported but never executed without explicit user consent.
- In auto-mode, do not interrupt execution for `SAFE` actions.
- Do not create abstractions "for the future" — only when duplication already exists.
- Do not change public API without explicit consent.
- Do not touch files you haven't read.
- Do not expand refactoring beyond the agreed scope.
- Atomic changes: one fix at a time, verify after each.
- Output to terminal, not to report files.
- Communicate in the user's language.
