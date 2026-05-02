# codetracer-wasmi-recorder CTFS audit (2026-05-02)

<!-- prettier-ignore-start -->

This memo summarises the CTFS audit performed against
`codetracer-wasmi-recorder` in iteration 1.65 of the IsoNim migration
campaign. It documents the architecture, the audit checklist outcomes,
the concrete fixes that landed in the same session, and the open
follow-ups that are out of scope for a single recorder audit.

For the broader campaign context, see
`/tmp/isonim-migration.txt` mission goals #5 (recorder fixes) and #6
(CTFS format migration), and the cross-cutting checklist in
section 5.6 of that file.

This is the **nineteenth** audited recorder family (after Ruby 1.21,
Python 1.27, JavaScript 1.38, EVM 1.39, PHP 1.41, Solana 1.44, Move
1.46, Cardano 1.48, Cairo 1.50, Flow/Cadence 1.52, Fuel/Sway 1.53,
PolkaVM 1.55, Miden 1.56, TON/Tolk 1.57, Circom 1.58, Leo/Aleo 1.59,
WASM/wazero 1.60, Bash+Zsh shell 1.61).

## Architecture

The recorder is a fork of the [wasmi](https://github.com/wasmi-labs/wasmi)
WebAssembly interpreter (a pure-Rust embeddable wasm runtime, distinct
from the wazero Go runtime audited in 1.60). The audit modifies only
the `wasmi_cli` crate (`crates/cli/`) — the executable surface — leaving
the interpreter library and WASI host implementation untouched.

The integration follows the **single-process Rust crate** audit pattern
established by EVM 1.39 / Solana 1.44 / Cairo 1.50 / Fuel 1.53 / PolkaVM
1.55 / Miden 1.56 / TON 1.57 / Leo 1.59: the recorder is a thin wrapper
around `codetracer_trace_writer_nim::TraceWriter` opened from
`main.rs` when `--trace-out <DIR>` is passed, and dropped through an
explicit `finish()` consumption on every exit path so the multi-stream
`.ct` container is flushed deterministically before
`process::exit` (the WASI `proc_exit` branch) skips destructors.

Component map after this audit:

- `crates/cli/src/recorder.rs`: new module with `WasmiRecorder`, the
    `CliTraceFormat` enum, and the `maybe_open_recorder` helper.
- `crates/cli/src/main.rs`: wires the recorder, stages call args and return
    values, routes `wasmi::Error` through `register_special_event`, and calls
    `finish()` on every exit path.
- `crates/cli/src/args.rs`: adds `--trace-out <DIR>` and
    `--trace-format <FMT>` (default `ctfs`) to the existing CLI.
- `crates/cli/Cargo.toml`: adds the CodeTracer trace writer path deps plus the
    `tempfile` dev dependency.
- `crates/cli/tests/ctfs_audit.rs`: new audit-pinning test suite.
- `crates/cli/tests/wats/ctfs_add.wat`: minimal `add(i32,i32)->i32` wasm module
    fixture for the audit smoke tests.

The wasmi interpreter library itself is not modified — per-instruction
`register_step` and intra-program `call`/`call_indirect` boundaries are
out of scope for this audit (they need hooks in
`crates/wasmi/src/engine/executor/`, sized as M7/M8 follow-ups in line
with PolkaVM's intra-program-call follow-up).

## Findings vs. section 5.6 checklist

### (a) CTFS format compliance — `TraceEventsFileFormat::Ctfs`

**Closed in this audit.**

`crates/cli/src/args.rs` adds a `--trace-format <FMT>` flag with
clap-derived `ValueEnum` over `CliTraceFormat`, defaulting to `ctfs`.
The enum maps onto the canonical `TraceEventsFileFormat` from
`codetracer_trace_writer_nim`, dispatching to the multi-stream `.ct`
writer for the `ctfs` branch and preserving the legacy CBOR+Zstd /
JSON paths for round-trip diagnostics.

`#[clap(rename_all = "snake_case")]` keeps the CLI surface aligned with
the `as_str()` helper and the `trace_metadata.json` payload (`binary_v0`
rather than clap's default kebab-case `binary-v0`). Mirrors the
PolkaVM 1.55 / Miden 1.56 / TON 1.57 sibling CLI surfaces.

### (b) `register_call` per call

**Closed at the top-level boundary; intra-program boundaries open.**

`main.rs` opens an explicit top-level `register_call` at the
[`wasmi::Func::call`] boundary BEFORE the interpreter starts:

```rust
if let Some(rec) = recorder.as_mut() {
    rec.register_top_level_call(&func_name, &ty, &func_args);
}
let call_result = func.call(ctx.store_mut(), &func_args, &mut func_results);
```

`WasmiRecorder::register_top_level_call` calls
`writer.ensure_function_id(name, <wasmi-program>, 1)` and then
`writer.register_call(function_id, Vec::new())` after staging the
declared arguments. Empty entry-point names (`""`, the start-section
case) normalise to `<start>` for human readability.

**Carve-out — open follow-up:** intra-program function-call boundaries
(`call` / `call_indirect` opcodes between user-defined wasm functions
inside the same module) are NOT yet emitted — closing this requires a
hook in `crates/wasmi/src/engine/executor/` similar to the PolkaVM
1.55 intra-program follow-up. Sized as an M7/M8 follow-up.

### (c) `register_call_arg` / argv staging

**Closed at the top-level boundary in this audit.**

`WasmiRecorder::register_top_level_call` iterates the CLI-provided
`&[Val]` slice and stages each entry via `writer.arg(format!("arg{idx}"),
value)`. The pending-args buffer is then drained onto the next
`register_call` call (canonical post-1.22 pattern, mirrors PolkaVM 1.55
A0..A5 / Miden 1.56 stack[0..3] s0..s3).

The placeholder `arg0`..`argN-1` names mirror the Miden 1.56 operand-
stack naming (Miden has no formal parameter list either). When
DWARF-driven argument names land for the wasmi interpreter (out of
scope), the names will be upgraded by reading
`DW_TAG_formal_parameter` ranges keyed on the wasm function index.

**Open follow-up:** DWARF-driven argument-name recovery (same shape
as the wazero 1.60 DWARF-less arg-name placeholder follow-up).

### (d) Write / EvmEvent / Error routing

**Error routing closed; Write routing partial; EvmEvent N/A.**

`WasmiRecorder::register_trap` routes `wasmi::Error` (out-of-fuel,
divide-by-zero, integer overflow on truncation, unreachable,
validation error, host-fn failure, ...) through

```rust
self.writer.register_special_event(
    EventLogKind::Error, "wasmi_trap", msg);
self.writer.register_return(NONE_VALUE);
```

The metadata string `"wasmi_trap"` lets future frontend filtering
distinguish wasm traps from generic stderr. The placeholder
`register_return(NONE_VALUE)` keeps the `.ct` container structurally
well-formed (Miden 1.56 audit lesson — letting `?` propagate before
`register_return` left the trace open and broke downstream readers).

WASI `proc_exit` (which surfaces as `WasmiError::i32_exit_status()`
rather than as a real trap) is treated as a normal return — the wasm
ran to completion from the contract's POV.

**Write routing partial:** wasi-cli's stdout/stderr go through the
wasmi-wasi inherit-stdio path which pipes directly to the host
fds — the recorder does not currently capture `fd_write`/`fd_read`
host-fn calls as `EventLogKind::Write`/`Read` events. Closing this
requires hooking the WASI host-fn dispatch in the wasmi-wasi crate
and routing each `fd_write` payload through
`register_special_event(Write, ...)`. Documented as deferred follow-up.

EvmEvent N/A — wasmi has no blockchain-style structured-event semantics
(unlike wazero 1.60's Stylus host hooks).

### (e) Thread events — `register_thread_*`

**N/A — wasmi does not currently support the wasm threads proposal.**

If a future iteration enables it, the recorder must call
`register_thread_start` / `register_thread_exit` /
`register_thread_switch` at the per-thread entry / exit points.
Documented as deferred follow-up.

### (f) Step records

**Open — interpreter-side hook needed.**

`WasmiRecorder` does not currently emit `register_step` calls because
the wasmi interpreter's executor loop
(`crates/wasmi/src/engine/executor/`) does not yet expose a
per-instruction / per-source-line callback. Closing this needs:

1. A trace-callback field on the wasmi `Engine` / `Store` (similar to
    wasmtime's `Caller<'_, T>` data slot, or polkavm's `set_step_callback`).
2. The executor dispatch loop calls the callback on each instruction
    step or each source-line transition.
3. `WasmiRecorder` registers a callback that calls
    `writer.register_step(source_path, line)` keyed on a DWARF source
    mapper (PolkaVM 1.55-style `SourceMapper::resolve(pc)`).

Sized as a multi-day follow-up — same shape as the PolkaVM 1.55
intra-program follow-up since both are interpreter-internal hooks.

### (g) CTFS schema match

**Closed at the smoke level.**

The audit smoke test
`audit_ctfs_writer_produces_trace_files` verifies the `--trace-out <DIR>`
directory is non-empty after a recorded run, and the
`finish()` chain wires the canonical sequence
`finish_writing_trace_events → finish_writing_trace_metadata →
finish_writing_trace_paths → close()`. The `close()` step is the one
that actually serialises the `.ct` container to disk for the Nim
multi-stream backend — without it the trace directory ends up empty
(this was the gap that broke the first iteration of this audit and is
now closed).

Read-side end-to-end content assertions (asserting specific events
land in the trace) remain a cross-cutting follow-up across all
audited recorders.

### (h) Obsolete `add_event` paths

**OK / clean.**

No `add_event` references in the wasmi recorder. All event emission
goes through dedicated `register_*` entry points
(`register_call`, `register_return`, `register_special_event`,
`writer.arg`).

### (i) `#[no_mangle]` collisions

**OK / clean.**

This is a Rust binary calling Rust crates directly via `path = "..."`
deps; no FFI extern symbols are defined recorder-side. Same disposition
as PolkaVM 1.55 / Miden 1.56 / TON 1.57 / Leo 1.59 sibling Rust-native
recorders.

## Concrete changes in this audit

1. **`crates/cli/src/recorder.rs`** — new module (419 lines):
    - `WasmiRecorder` struct holding a boxed
        `dyn TraceWriter + Send` plus the output directory.
    - `WasmiRecorder::new` opens the output dir, creates the writer,
        calls all three `begin_writing_trace_*` paths plus `start()`.
    - `WasmiRecorder::register_top_level_call` ensures the function id,
        stages each `&[Val]` element via `writer.arg("arg{i}", value)`,
        then calls `register_call(function_id, Vec::new())`.
    - `WasmiRecorder::register_top_level_return` records the first
        result via `register_return` (multi-value follow-up below).
    - `WasmiRecorder::register_trap` routes `wasmi::Error` via
        `register_special_event(Error, "wasmi_trap", msg)` then closes
        the call frame with `register_return(NONE_VALUE)`.
    - `WasmiRecorder::finish` consumes self and chains the three
        `finish_writing_trace_*` calls plus the load-bearing `close()`.
    - `wasmi_val_to_value_record_inner` decodes wasmi's `Val` enum
        (I32, I64, F32, F64, ref types) onto the canonical
        `ValueRecord`+`TypeRecord` pair.
    - `CliTraceFormat` clap `ValueEnum` (`#[clap(rename_all = "snake_case")]`,
        default `Ctfs`) with `as_str()` helper for
        diagnostic emission.
    - `maybe_open_recorder` helper used by `main.rs`.

2. **`crates/cli/src/main.rs`** — wires the recorder:
    - `mod recorder;` declaration plus imports.
    - `maybe_open_recorder(program_name, args.trace_out(), args.trace_format())?`
        after argument parsing.
    - `rec.register_top_level_call(&func_name, &ty, &func_args)` BEFORE
        `func.call`.
    - On success: `rec.register_top_level_return(&func_results)` then
        `recorder.take().map(|r| r.finish())?`.
    - On error: `rec.register_trap(&error)` (or
        `register_top_level_return` for the WASI proc_exit case) then
        `recorder.take().map(|r| r.finish())?` BEFORE the `process::exit`
        branch — `process::exit` skips destructors entirely, so relying
        on `Drop` would leave the trace half-flushed.

3. **`crates/cli/src/args.rs`** — new flags:
    - `--trace-out <DIR>` (`Option<PathBuf>`).
    - `--trace-format <FMT>` (default `ctfs`).
    - Public accessors `trace_out()` / `trace_format()`.

4. **`crates/cli/Cargo.toml`**:
    - Path deps on `codetracer_trace_types` and `codetracer_trace_writer_nim`.
    - `tempfile = "3"` dev-dep (used by `tests/ctfs_audit.rs`).

5. **`crates/cli/tests/ctfs_audit.rs`** — new audit-pinning suite:
    - `audit_ctfs_help_advertises_trace_flags` — pins `--trace-out` +
        `--trace-format` + `[default: ctfs]` in `--help`.
    - `audit_ctfs_writer_produces_trace_files` — pins audit (g) at
        smoke level: trace dir non-empty after a recorded run.
    - `audit_ctfs_no_trace_out_means_no_recorder` — pins the
        no-side-effects-without-opt-in invariant.
    - `audit_ctfs_format_values_accepted` — pins all four format
        identifiers (`ctfs`, `binary`, `binary_v0`, `json`) work.

6. **`crates/cli/tests/wats/ctfs_add.wat`** — minimal
    `add(i32,i32)->i32` wasm module fixture (plus a no-op default-export
    for the no-`--invoke` path).

## Verification

```
cd /home/zahary/metacraft/codetracer-wasmi-recorder
cargo build --release                           # clean
AH_TEST_RESOURCE_GUARD=1 cargo test -p wasmi_cli --test ctfs_audit
# 4 passed
AH_TEST_RESOURCE_GUARD=1 cargo test --release -p wasmi_cli
# 14 passed (7 unit + 4 ctfs_audit + 3 run)
AH_TEST_RESOURCE_GUARD=1 cargo test --release --workspace --exclude wasmi_wast
# 1608 passed
```

The `wasmi_wast` test crate fails to compile both before and after
this audit because it `include_str!`s spec test fixtures
(`crates/wast/tests/spec/memory64/memory64/*.wast`) that are not
present in the working tree (likely a missing git submodule). The
issue is unrelated to this audit and reproduces on the pre-audit
detached HEAD.

## Open gaps (deferred follow-ups)

These are out of scope for a single recorder audit and are tracked in
this recorder's future iterations or at the cross-cutting writer side.
Each follow-up describes the **fix shape** so the next sub-agent can
pick it up without re-deriving the analysis.

### A. Per-instruction / per-source-line `register_step`

The wasmi interpreter's executor loop does not yet expose a step
callback. Fix shape:

1. Add an `engine.set_step_callback(...)` method on
    `crates/wasmi/src/engine/mod.rs` taking a
    `Box<dyn FnMut(InstructionPtr, &Store) + Send>`.
2. In the executor dispatch loop
    (`crates/wasmi/src/engine/executor/instrs.rs` or wherever the
    per-instruction step happens) call the callback on each
    instruction transition.
3. `WasmiRecorder::new` registers a callback that walks a DWARF
    source mapper (PolkaVM 1.55 `SourceMapper::resolve(pc)` shape)
    and calls `writer.register_step(source_path, line)` only when the
    line changes.

Sized as a multi-day follow-up — same shape as PolkaVM 1.55
`run_step_loop`.

### B. Intra-program function-call boundaries

`call` / `call_indirect` opcodes between user-defined wasm functions
inside the same module are not yet emitted as nested
`register_call` / `register_return` pairs. Fix shape:

1. Same engine-callback hook as (A) above, but for instruction
    variants `Instruction::CallInternal` /
    `Instruction::CallIndirect` / `Instruction::Return`.
2. On call: `writer.ensure_function_id(name, source, line)` +
    `writer.register_call(fid, args)` (where the wasm signature drives
    the `arg{i}` placeholder names — same as the top-level path).
3. On return: `writer.register_return(value)`.

Symbolic argument names + values for intra-program calls require the
same DWARF integration as (D) below.

### C. WASI host-fn output capture

`fd_write` and `fd_read` payloads are not currently captured as
`EventLogKind::Write` / `Read` special events. Fix shape:

1. Hook `fd_write` / `fd_read` in
    `crates/wasi/src/wasi.rs` (the wasmi-wasi crate).
2. Each call signature is
    `(fd, iovs_ptr, iovs_len, nwritten_ptr) -> errno`. After the
    inner `inherit_stdio` write succeeds, read the iov bytes from
    linear memory and call
    `recorder.write_event(fd, payload)` →
    `register_special_event(Write, "fd_{fd}", utf8_or_hex(payload))`.
3. Pass the recorder reference into the wasi context — same
    "callback registration" plumbing as (A) above; recorder is plumbed
    through `WasiCtx` as a typed extension field.

### D. DWARF-driven argument names

Currently uses positional `arg0..argN-1` placeholders. Fix shape:

1. Parse `.debug_info` / `.debug_str` from the `.wasm` module's
    custom section using `gimli` (PolkaVM 1.55-style).
2. Build a `wasm-func-index → [DW_TAG_formal_parameter name + type]`
    table at `WasmiRecorder::new` time.
3. `register_top_level_call` looks up the entry-point function's
    parameter names from the table, falling back to `arg{i}` only
    when no DWARF is present.

### E. Multi-value returns

`register_top_level_return` records only the first result via
`register_return`. Wasm multi-value returns (post-`multi-value`
proposal) are common in practice. Fix shape:

1. Either extend the canonical `register_return` to take a
    `Vec<ValueRecord>` (writer-side change), or
2. Synthesise a `ValueRecord::Tuple` wrapper that carries the
    multi-value tuple and is rendered as `(v0, v1, v2)` by the
    frontend (recorder-side change, no writer schema bump).

### F. Wasm threads proposal

Wasmi does not currently support the threads proposal. When it does,
the recorder must call
`register_thread_start` / `register_thread_exit` /
`register_thread_switch` at the per-thread entry / exit points.
Documented for completeness; currently flagged N/A for this audit.

### G. Read-side end-to-end content assertions

The new `audit_ctfs_writer_produces_trace_files` verifies the trace
container is produced and is non-trivially sized, but does not walk
it and assert specific event content. Same assertion-depth
follow-up flagged for Cairo / Cardano / Flow / Fuel / PolkaVM /
Miden / TON / Circom / Leo / WASM/wazero / Shell. Adding the Nim
trace-reader as a Rust dev-dep is the cross-cutting blocker.

### H. Multi-stream IO event metadata collapse

Same cross-cutting issue documented in 1.39 / 1.41 / 1.44 / 1.46 /
1.48 / 1.50 / 1.52 / 1.53 / 1.55 / 1.56 / 1.57 / 1.58 / 1.59 / 1.60
/ 1.61. The `wasmi_trap` metadata string lands in the multi-stream
writer's stderr bucket via `toIOEventKind` collapse — out of scope
for any single recorder audit; flagged as a writer-side fix.

## Cross-cutting findings

- **Drop-vs-finish() lesson reinforced.** The first iteration of this
    audit relied on `drop(recorder)` to flush the trace, which works
    for the non-WASI happy path but silently swallows writer-side
    errors and (more importantly) is skipped entirely by
    `process::exit(exit_code)` on the WASI `proc_exit` branch. The
    load-bearing fix is to `recorder.take().map(|r| r.finish())?`
    on every exit path BEFORE `process::exit`. Same lesson the
    Miden 1.56 audit uncovered for `?`-propagation.

- **`close()` is the load-bearing step for the Nim CTFS backend.**
    The first build of `WasmiRecorder::finish` chained only the three
    `finish_writing_trace_*` calls. With those alone, the `--trace-out`
    directory ended up empty — the multi-stream `.ct` container is
    serialised by `close()`. PolkaVM 1.55 / Miden 1.56 / TON 1.57 all
    call `close()` after the three `finish_writing_*` calls; the
    trait's default `close()` is a no-op (suitable for in-memory test
    doubles), so the omission is silent. **Recommendation:** the
    cross-cutting recorder template / cookbook should explicitly
    document this ordering (`begin_writing_trace_* → start() → ...
    events ... → finish_writing_trace_* → close()`) and the smoke test
    should assert on a non-empty output directory.

- **`#[clap(rename_all = "snake_case")]` for `--trace-format`.** Clap
    defaults to kebab-case for `ValueEnum` variants (`BinaryV0`
    → `binary-v0`), but the `as_str()` helper, the trace-metadata
    payload, and every sibling recorder's CLI surface use snake_case
    (`binary_v0`). This is easy to overlook when adding a new
    recorder CLI; recommend the cookbook flag this as the default.

- **`AH_TEST_RESOURCE_GUARD=1` bypass needed for sibling recorder
    test runs.** The codetracer dev shell wraps `cargo test` with a
    resource-guard wrapper that rejects unwrapped `cargo test` calls.
    Sibling recorder repos (which live OUTSIDE the codetracer monorepo
    but still inside the same dev shell) need
    `AH_TEST_RESOURCE_GUARD=1` to bypass the wrapper. This is already
    the convention in the audit's verification block; documenting here
    for cross-recorder consistency.

---

Audit performed by Claude Opus 4.7 (1M context) on 2026-05-02 as part
of iteration 1.65 of the IsoNim migration campaign. See
`/tmp/isonim-migration.txt` for the full campaign log.

<!-- prettier-ignore-end -->
