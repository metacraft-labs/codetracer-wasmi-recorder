# Source-line stepping, calls and locals in the wasmi recorder

Status: design, not implemented. Estimate: 8–9 working days (breakdown in §6).

## 1. Where the recorder is today

`wasmi_cli --trace-out <dir>` writes one call (the invoked export, at the
synthetic location `<wasmi-program>:1`), its arguments as `arg0..argN`, its
first result, and a trap as an error event. There is one step per trace. No
source path, no line, no call between wasm functions, no locals. The module
header of `crates/cli/src/recorder.rs` lists these as out of scope.

`codetracer-wasm-recorder` (wazero) records the same modules with a step per
source line, a call and return per function entered (including inlined
subroutines), parameters, locals and return values decoded from DWARF, and
column-aware steps. `codetracer-specs/CLI/recorders/wasm.md` requires the two
recorders to produce interchangeable CTFS traces ("a consumer must not need to
know which runtime recorded a module"), while allowing their coverage to
differ as long as the trace says which runtime produced it. The spec sets no
narrower scope for wasmi; it also leaves open whether `wasmi_cli` is a
user-facing recorder at all (no packaging, no `ct record` discovery). That
open question does not block this work, but it decides its priority.

## 2. What has to exist

| Need | wazero recorder | wasmi today |
| --- | --- | --- |
| DWARF index: functions, lines, params/locals with locations, inlined subroutines, types | `internal/wasmdebug/` (~1.2k lines Go) | none |
| Map from an executing instruction to a wasm code offset | interpreter keeps the original offset per op | the translator knows the offset (`update_pos`), but drops it; the executor runs register-machine IR with no back-map |
| Per-instruction hook | inline in the interpreter loop | none; one `match` loop in `engine/executor/instrs.rs::execute` |
| Call / return events for wasm-to-wasm calls | at frame push/pop | none |
| Value decoding (base types, pointers, structs, arrays, Rust `String`/`&str`/`Vec`, tuples) | `variable_readers.go`, `rust_variable_readers.go` (~750 lines) | four wasm primitive kinds only |
| DWARF-bearing test programs | `test_code/*.rs` (+ committed `.wasm`) | four hand-written `.wat` files with no debug info |

## 3. Options considered

**A. Hook the interpreter (chosen).** The repository is a fork of wasmi, so
the executor can be changed directly. Cost is concentrated in the side table
and the value readers; the untraced path can be kept free of overhead.

**B. Instrument the module with `ct-instrument` and serve the hooks from the
wasmi host.** Rejected: the instrumentation layer
(`Recording-Backends/WASM-Instrumentation-Layer.md` §2) deliberately records
only host-boundary crossings. It cannot see locals or the operand stack, and
per-event hooks were measured at ~18× slowdown. It is not a step-level
recorder.

**C. Single-step through fuel.** Setting fuel to one instruction and resuming
gives a stop per IR instruction, but wasmi charges fuel per basic block
(`ConsumeFuel { block_fuel }`), not per instruction, and each resume crosses
the public API. It yields neither a code offset nor frame access. Rejected.

**D. Replay the module in wazero and keep wasmi as an executor only.**
Defeats the point of a second runtime and is what the spec forbids ("neither
recorder silently falls back to the other").

## 4. Design (option A)

### 4.1 DWARF index (`crates/cli/src/dwarf/`)

Parse the module's custom `.debug_*` sections with `gimli` (already in
`Cargo.lock` at 0.31 through `addr2line`). Build, keyed by wasm code offset
(DWARF addresses in wasm are offsets from the start of the Code section
payload):

- line rows: `offset -> (file, line, column)`, as a sorted vector;
- functions: low/high pc, linkage name and demangled name, declaration
  file/line, return type, parameters and locals with their location
  expressions and pc ranges;
- inlined subroutines: pc ranges, call file/line/column, their own
  parameters and locals.

Location expressions to support, in the order wazero needs them:
`DW_OP_WASM_location` local / global / operand-stack, `DW_OP_fbreg` with the
frame base from `DW_OP_WASM_location global 0` (`__stack_pointer`), and
`DW_OP_piece`. A variable whose location cannot be evaluated is reported as
unreadable for that step, not dropped silently.

### 4.2 IR-to-offset side table (`crates/wasmi/src/engine/translator/`)

`FuncTranslator::update_pos` already receives every operator's original
position. When a trace side table is requested (an `EngineConfig` flag, off
by default), `InstrEncoder::push_instr` records `(instr index, wasm offset -
code section start)`. Instructions rewritten in place (fused compare-and-
branch, result-register retargeting, `copy` elision) keep the entry of the
instruction they replaced; at line granularity that is exact enough, and the
test in §5 pins it. The table is stored next to the instructions in
`CompiledFuncEntity`, so lazy compilation produces it on first call like the
code itself.

### 4.3 Executor hook (`crates/wasmi/src/engine/executor/`)

A `TraceHook` trait object in the store's inner data, with
`on_instr(func, instr_index, frame)`, `on_call(callee, frame)`,
`on_return(frame)`, `on_host_call` and `on_trap`. The `execute` loop checks
one flag before dispatch; the check is compiled only under a `trace` cargo
feature, so stock wasmi builds are unchanged and a non-tracing run of the
recorder pays one predictable branch. The hook maps `instr_index` through the
side table and fires a step only when the DWARF line row changes, so the
writer sees one step per source line, as in wazero. Calls and returns come
from `CallInternal*`, `CallIndirect*`, `ReturnCall*` (a tail call is a return
followed by a call) and every `Return*` variant; a trap unwinds the open
frames with returns so the container stays well formed.

Frame access for value reading: a wasm local `i` lives in register `i` of the
current frame (`FrameRegisters`), globals in the instance, memory through the
instance's default memory. Operand-stack locations map to the translator's
dynamic registers only at the same instruction and are reported unreadable
rather than guessed.

### 4.4 Recorder (`crates/cli/src/recorder.rs`)

Replace the synthetic `<wasmi-program>` location with the DWARF paths
(registered once each with their line lengths for column-aware steps), name
arguments from `DW_TAG_formal_parameter`, decode values through a port of
the wazero readers, and record the runtime (`wasmi`) in the trace metadata as
the spec requires. Modules without DWARF keep today's behaviour: one call at
the synthetic location, said so on stderr.

## 5. Tests

- Test programs: reuse the Rust sources the wazero recorder records
  (`codetracer-wasm-recorder/test_code/*.rs`: integers, structs, tuples,
  strings, vectors, pointers, inlined functions) plus one C program. The
  repository refuses committed binaries, so the tests build the `.wasm` at
  test time; the dev environment needs a `wasm32-wasip1` Rust target and a
  wasm-capable clang (neither is provided by it today). A missing toolchain
  fails the test with a message, never skips.
- Per program: the sequence of `(file, line)` steps, the call tree with
  argument names and values, and the locals at chosen steps, asserted through
  the Nim reader as `tests/ctfs_audit.rs` already does.
- Cross-runtime: record the same `.wasm` with both recorders and compare
  steps, calls and values; this is the spec's interchangeability rule as a
  test. Differences that are accepted (e.g. operand-stack variables) are
  listed in the test, not tolerated by a loose comparison.
- Side table: for a `.wat` with hand-written `.debug_line`, each IR
  instruction maps to the expected offset, including fused instructions.
- Red-first controls: a run with the hook removed must fail the step
  sequence; a run with the side table disabled must fail the line test.
- Overhead: the wasmi benches with the `trace` feature on and no hook
  installed stay within noise of the stock build.

## 6. Estimate

| Piece | Days |
| --- | --- |
| DWARF index with gimli (lines, functions, variables, inlines, location expressions) | 1.5 |
| Translator side table, lazy compilation, fused/rewritten instructions | 1 |
| Executor hook: steps, calls, tail calls, returns, host calls, trap unwinding; feature gate | 1.5 |
| Value readers (base types, pointers, structs, arrays, Rust strings/vectors/tuples) | 1.5–2 |
| Recorder wiring: paths, columns, argument names, runtime metadata, no-DWARF fallback | 0.5 |
| Test toolchain in the dev environment, test programs, tests, cross-runtime parity | 1.5–2 |
| Overhead check and review fixes | 0.5 |
| **Total** | **8–9** |

The largest risks are the register machine's in-place rewrites (§4.2), which
can shift a step by an instruction inside a line, and the operand-stack
variable locations at `-O1` and above, which wasmi cannot answer without a
second side table from stack slots to registers.
