//! CodeTracer recorder integration for the Wasmi CLI.
//!
//! This module wraps [`codetracer_trace_writer_nim`] for the wasmi CLI driver.
//! It implements the canonical CTFS recorder pattern documented in section 5.6
//! of `/tmp/isonim-migration.txt` and used by the sibling Rust-native VM
//! recorders (PolkaVM 1.55, Miden 1.56, TON 1.57).
//!
//! Audit scope (this iteration — see AUDIT-CTFS-2026-05.md):
//!   * Default-Ctfs CLI scaffold via `--trace-out` + `--trace-format` (a).
//!   * Top-level [`wasmi::Func::call`] boundary as a [`register_call`]
//!     site (b).
//!   * Wasmi function-signature `&[Val]` parameters staged via
//!     [`TraceWriter::arg`] as `arg{i}`-named call-args (c).
//!   * `wasmi::Error` (host trap, validation error, OOG, ...) routed through
//!     [`register_special_event`] with `EventLogKind::Error` and metadata
//!     `wasmi_trap` (d / i).
//!
//! Out of scope (deferred to follow-up audits):
//!   * Per-instruction / per-source-line [`register_step`] from the wasmi
//!     interpreter loop.  Closing this needs a hook in the executor's
//!     instruction-dispatch loop (`crates/wasmi/src/engine/executor/`).
//!   * Intra-program function-call boundaries (`call` / `call_indirect`
//!     opcodes between user-defined wasm functions inside the same module).
//!   * DWARF-driven argument names (currently we use positional `arg0..argN`
//!     placeholders, mirroring Miden 1.56 stack[0..3] -> s0..s3).
//!   * WASI host-fn output capture (`fd_write`, `fd_read`) routed through
//!     [`register_special_event`] with `EventLogKind::Write` /
//!     `EventLogKind::Read`.
//!   * Wasm threads proposal (wasmi does not currently support it).

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Error as AnyhowError, Result};
use codetracer_trace_types::{
    EventLogKind, Line, TypeKind, TypeRecord, TypeSpecificInfo, ValueRecord, NONE_VALUE,
};
use codetracer_trace_writer_nim::{
    create_trace_writer, TraceEventsFileFormat, TraceWriter as TraceWriterTrait,
};
use wasmi::{Error as WasmiError, FuncType, Val};

/// Synthetic source path used for the top-level wasmi entry-point call frame.
///
/// The wasmi CLI does not yet recover DWARF source paths from the loaded
/// `.wasm` module; until that lands every step / call lives inside this
/// virtual file.  The frontend renders it as the entry-point pseudo-source
/// (mirrors Miden 1.56 `<masm-program>` placeholder).
const WASMI_VIRTUAL_PATH: &str = "<wasmi-program>";

/// Synthetic source line for the top-level wasmi entry-point.
const WASMI_VIRTUAL_LINE: Line = Line(1);

/// CodeTracer recorder bound to a single wasmi CLI invocation.
///
/// One instance of this struct corresponds to one `.ct` (or legacy `.bin` /
/// `.json`) container.  The lifetime is the entire wasmi CLI run: open
/// before [`wasmi::Func::call`], stage args, drive on success / error,
/// close on drop (the underlying writer flushes in its `Drop` impl).
pub struct WasmiRecorder {
    /// Boxed writer.  We use the trait object so the rest of the recorder is
    /// agnostic to which backend was selected on the CLI (CTFS vs. CBOR+Zstd
    /// vs. JSON).  Same shape as PolkaVM 1.55 / Miden 1.56 recorders.
    writer: Box<dyn codetracer_trace_writer_nim::TraceWriter + Send>,

    /// Output directory the writer will populate on close.  Surfaced for
    /// diagnostics and test assertions.
    #[allow(dead_code)]
    out_dir: PathBuf,
}

impl WasmiRecorder {
    /// Create a new recorder rooted at `out_dir`.
    ///
    /// Creates the directory if absent.  The trace will materialise on the
    /// filesystem when the recorder is dropped or when [`Self::finish`] is
    /// called.
    ///
    /// `program` is forwarded to the writer as the program-name field used in
    /// `trace_metadata.json` / the CTFS magic-header program record; it is
    /// not used to open any file.
    pub fn new(
        program: &str,
        out_dir: PathBuf,
        format: TraceEventsFileFormat,
    ) -> Result<Self, AnyhowError> {
        std::fs::create_dir_all(&out_dir).with_context(|| {
            format!(
                "failed to create CodeTracer trace output directory: {}",
                out_dir.display()
            )
        })?;

        let mut writer = create_trace_writer(program, &[], format);

        // Use the correct filename extension so that downstream readers
        // (db-backend `CTFSTraceReader`, Nim `ct_reader_*` FFI) can infer
        // the format from the file extension.  Mirrors the
        // PolkaVM 1.55 / Miden 1.56 / TON 1.57 sibling layout.
        let events_filename = match format {
            TraceEventsFileFormat::Json => "trace.json",
            TraceEventsFileFormat::Binary
            | TraceEventsFileFormat::BinaryV0
            | TraceEventsFileFormat::Ctfs => "trace.bin",
        };
        let events_path = out_dir.join(events_filename);
        let metadata_path = out_dir.join("trace_metadata.json");
        let paths_path = out_dir.join("trace_paths.json");

        TraceWriterTrait::begin_writing_trace_events(&mut *writer, &events_path)
            .map_err(|e| anyhow!("begin_writing_trace_events: {e}"))?;
        TraceWriterTrait::begin_writing_trace_metadata(&mut *writer, &metadata_path)
            .map_err(|e| anyhow!("begin_writing_trace_metadata: {e}"))?;
        TraceWriterTrait::begin_writing_trace_paths(&mut *writer, &paths_path)
            .map_err(|e| anyhow!("begin_writing_trace_paths: {e}"))?;

        // Open the trace at the synthetic top-level location.  Mirrors
        // PolkaVM 1.55 step-7.
        TraceWriterTrait::start(&mut *writer, Path::new(WASMI_VIRTUAL_PATH), WASMI_VIRTUAL_LINE);

        Ok(Self { writer, out_dir })
    }

    /// Flush and close the trace.  Idempotent.  Called explicitly from
    /// `main.rs` after the call frame is closed (success / error / WASI
    /// exit branches all converge here).  We intentionally do this instead
    /// of relying on `Drop` so that the failure case can surface a
    /// diagnostic message — `Drop` cannot return errors.
    pub fn finish(mut self) -> Result<(), AnyhowError> {
        TraceWriterTrait::finish_writing_trace_events(&mut *self.writer)
            .map_err(|e| anyhow!("finish_writing_trace_events: {e}"))?;
        TraceWriterTrait::finish_writing_trace_metadata(&mut *self.writer)
            .map_err(|e| anyhow!("finish_writing_trace_metadata: {e}"))?;
        TraceWriterTrait::finish_writing_trace_paths(&mut *self.writer)
            .map_err(|e| anyhow!("finish_writing_trace_paths: {e}"))?;
        // For the Nim multi-stream backend `close()` is the step that
        // actually serialises the `.ct` container to disk — without it
        // the trace directory ends up containing only the JSON sidecars
        // (or, depending on the streaming-encoder state, can be left
        // empty altogether).  PolkaVM 1.55 / Miden 1.56 / TON 1.57
        // sibling recorders all chain `close()` after the three
        // `finish_writing_*` calls; we mirror that ordering here.
        TraceWriterTrait::close(&mut *self.writer)
            .map_err(|e| anyhow!("close: {e}"))?;
        Ok(())
    }

    /// Stage the wasmi function-call arguments as canonical CTFS call args
    /// AND open the call frame.
    ///
    /// `func_name` is the user-facing entry-point name (`""`, `_start`, or
    /// the value passed to `--invoke ...`).  `func_ty` is the wasmi-resolved
    /// signature; we use it for the typed parameter ValType so that the
    /// trace's type table carries the correct wasm primitive kind.
    /// `args` is the CLI-provided argv slice — same `&[Val]` that wasmi's
    /// [`wasmi::Func::call`] consumes.
    ///
    /// We name the args `arg0`..`argN-1` because wasm has no DWARF-less way
    /// to recover formal-parameter names from the module: this is the same
    /// placeholder strategy Miden 1.56 used for `s0..s3`.  When DWARF
    /// support lands (out of scope for this audit), the names will be
    /// upgraded by reading `DW_TAG_formal_parameter` ranges keyed on the
    /// wasm function index.
    pub fn register_top_level_call(
        &mut self,
        func_name: &str,
        func_ty: &FuncType,
        args: &[Val],
    ) {
        // Make sure the function-id is known to the writer.  We treat the
        // entry-point as `<func_name> @ <virtual> : 1` because wasmi has not
        // resolved the actual `.wasm` module's source path yet.  Empty
        // entry-point names (the `""` / start-section case) are normalised
        // to `<start>` for human readability.
        let display_name = if func_name.is_empty() {
            "<start>"
        } else {
            func_name
        };
        let function_id = self.writer.ensure_function_id(
            display_name,
            Path::new(WASMI_VIRTUAL_PATH),
            WASMI_VIRTUAL_LINE,
        );

        // Stage call args.  Each `arg(name, value)` call:
        //   1. Registers a step-local variable so the arg surfaces in
        //      `ct/load-locals` for the entry-point frame.
        //   2. Buffers the (name, value) pair on the writer's pending-args
        //      slot so the next `register_call` attaches it to the
        //      CallRecord.args slice (rendered in the calltrace pane's
        //      `.call-arg` rows post-1.17).
        for (idx, val) in args.iter().enumerate() {
            let name = format!("arg{idx}");
            let (value, ty) = wasmi_val_to_value_record(self.writer.as_mut(), val, func_ty, idx);
            // Make sure the arg's type is registered before the value is
            // staged so the writer's type-table is consistent.
            self.writer.register_raw_type(ty);
            // Stage on the call-record.  The arg() call internally calls
            // `register_variable_with_full_value` so this is also a
            // step-local variable as a side effect.
            let _ = self.writer.arg(&name, value);
        }

        // `ensure_function_id` already returns a `FunctionId` newtype, so
        // forward it directly to `register_call`.
        self.writer.register_call(function_id, Vec::new());
    }

    /// Close the top-level call frame on success.
    ///
    /// `outputs` is the wasmi result slice; the first result (if any) is
    /// recorded as the return value and the rest are dropped because the
    /// canonical CTFS `register_return` API takes a single value.  Wasm
    /// multi-value returns (post-`multi-value` proposal) are common in
    /// practice; capturing them all needs a `Tuple` ValueRecord wrapper —
    /// flagged in AUDIT-CTFS-2026-05.md as an open follow-up.
    pub fn register_top_level_return(&mut self, outputs: &[Val]) {
        let return_value = match outputs.first() {
            Some(val) => self.val_to_value_record(val),
            None => NONE_VALUE,
        };
        self.writer.register_return(return_value);
    }

    /// Route a wasmi runtime trap (out-of-fuel, division-by-zero, integer
    /// overflow on truncation, unreachable, validation error, host-fn
    /// failure, ...) onto the canonical CTFS error channel.
    ///
    /// Mirrors EVM 1.39 / Cairo 1.50 / PolkaVM 1.55 / Miden 1.56 / TON 1.57
    /// trap routing: the special event lands in the `stderr` IO bucket via
    /// the multi-stream writer's `toIOEventKind` collapse, and the metadata
    /// string `wasmi_trap` lets future frontend filtering distinguish wasm
    /// traps from generic stderr.
    pub fn register_trap(&mut self, error: &WasmiError) {
        let msg = format!("{error:#}");
        self.writer
            .register_special_event(EventLogKind::Error, "wasmi_trap", &msg);
        // We still close the call frame to keep the .ct container
        // structurally well-formed (Miden 1.56 audit lesson — letting `?`
        // propagate before `register_return` left the trace open and broke
        // downstream readers).  The placeholder return value mirrors the
        // PolkaVM Trap branch.
        self.writer.register_return(NONE_VALUE);
    }

    /// Decode a wasmi [`Val`] into a [`ValueRecord`] suitable for
    /// [`TraceWriter::register_return`].
    ///
    /// `register_return` does not carry a typed parameter slot, so we use
    /// the value-only conversion here (vs. the param-typed path used by
    /// `register_top_level_call` which carries `arg{i}` typing).
    fn val_to_value_record(&mut self, val: &Val) -> ValueRecord {
        let (record, ty) = wasmi_val_to_value_record_inner(self.writer.as_mut(), val);
        self.writer.register_raw_type(ty);
        record
    }
}

/// Convert a wasmi `[Val]` element at position `idx` to a (value, type)
/// pair, using the function-signature's `ValType` at that index for the
/// typed name.  Falls back to `Raw` for `FuncRef` / `ExternRef` which the
/// canonical CTFS schema does not yet have first-class kinds for.
fn wasmi_val_to_value_record(
    writer: &mut (dyn codetracer_trace_writer_nim::TraceWriter + Send),
    val: &Val,
    func_ty: &FuncType,
    idx: usize,
) -> (ValueRecord, TypeRecord) {
    // The signature may carry fewer params than CLI-provided args (the CLI
    // bails before us in that case, but defensively treat over-supply as
    // Raw-typed).  Wasmi's verify_and_prepare_inputs_outputs catches the
    // count mismatch in `Func::call` itself, so this is a belt-and-braces.
    let _ = func_ty.params().get(idx);
    wasmi_val_to_value_record_inner(writer, val)
}

/// Inner conversion shared by the call-arg path and the return-value path.
/// Routes wasmi's six-variant `Val` enum (I32, I64, F32, F64, V128
/// optionally, FuncRef, ExternRef) onto the canonical
/// [`ValueRecord`]+[`TypeRecord`] pair.
fn wasmi_val_to_value_record_inner(
    writer: &mut (dyn codetracer_trace_writer_nim::TraceWriter + Send),
    val: &Val,
) -> (ValueRecord, TypeRecord) {
    match val {
        Val::I32(i) => {
            let type_id = writer.ensure_type_id(TypeKind::Int, "i32");
            (
                ValueRecord::Int {
                    i: i64::from(*i),
                    type_id,
                },
                TypeRecord {
                    kind: TypeKind::Int,
                    lang_type: "i32".to_string(),
                    specific_info: TypeSpecificInfo::None,
                },
            )
        }
        Val::I64(i) => {
            let type_id = writer.ensure_type_id(TypeKind::Int, "i64");
            (
                ValueRecord::Int { i: *i, type_id },
                TypeRecord {
                    kind: TypeKind::Int,
                    lang_type: "i64".to_string(),
                    specific_info: TypeSpecificInfo::None,
                },
            )
        }
        Val::F32(f) => {
            let type_id = writer.ensure_type_id(TypeKind::Float, "f32");
            (
                ValueRecord::Float {
                    f: f64::from(f32::from(*f)),
                    type_id,
                },
                TypeRecord {
                    kind: TypeKind::Float,
                    lang_type: "f32".to_string(),
                    specific_info: TypeSpecificInfo::None,
                },
            )
        }
        Val::F64(f) => {
            let type_id = writer.ensure_type_id(TypeKind::Float, "f64");
            (
                ValueRecord::Float {
                    f: f64::from(*f),
                    type_id,
                },
                TypeRecord {
                    kind: TypeKind::Float,
                    lang_type: "f64".to_string(),
                    specific_info: TypeSpecificInfo::None,
                },
            )
        }
        // Both reference-flavoured wasmi value variants surface as Raw
        // strings — the canonical ValueRecord schema does not yet have
        // first-class wasm reference kinds.  For ExternRef we render as
        // `<externref>`; for FuncRef as `<funcref>` (or the textual debug
        // for non-null ones).  Mirrors wazero 1.60's hex rendering of
        // raw memory bytes.
        other => {
            let type_id = writer.ensure_type_id(TypeKind::Raw, "wasm-ref");
            (
                ValueRecord::Raw {
                    r: format!("{other:?}"),
                    type_id,
                },
                TypeRecord {
                    kind: TypeKind::Raw,
                    lang_type: "wasm-ref".to_string(),
                    specific_info: TypeSpecificInfo::None,
                },
            )
        }
    }
}

/// CLI-facing format selector mirroring [`TraceEventsFileFormat`].
///
/// Defined here (rather than in `args.rs`) so the same enum can be reused
/// by the test harness without dragging the entire `clap` parser in.
///
/// `rename_all = "snake_case"` keeps the CLI surface aligned with the
/// `as_str()` helper and the `trace_metadata.json` payload — `binary_v0`
/// rather than clap's default `binary-v0` kebab-case, mirroring the
/// PolkaVM 1.55 / Miden 1.56 / TON 1.57 sibling CLI surfaces that all
/// use snake_case for their format identifiers.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
#[clap(rename_all = "snake_case")]
pub enum CliTraceFormat {
    /// Canonical CodeTracer multi-stream container (recommended).
    Ctfs,
    /// Legacy CBOR + Zstd binary format.
    Binary,
    /// Older CBOR + Zstd binary format (kept for round-trip diagnostics).
    BinaryV0,
    /// Human-readable JSON (slower; useful for debugging).
    Json,
}

impl Default for CliTraceFormat {
    fn default() -> Self {
        CliTraceFormat::Ctfs
    }
}

impl From<CliTraceFormat> for TraceEventsFileFormat {
    fn from(fmt: CliTraceFormat) -> Self {
        match fmt {
            CliTraceFormat::Ctfs => TraceEventsFileFormat::Ctfs,
            CliTraceFormat::Binary => TraceEventsFileFormat::Binary,
            CliTraceFormat::BinaryV0 => TraceEventsFileFormat::BinaryV0,
            CliTraceFormat::Json => TraceEventsFileFormat::Json,
        }
    }
}

impl CliTraceFormat {
    /// Stable lowercase identifier matching the `clap::ValueEnum`
    /// representation; useful for `trace_metadata.json` emission and
    /// diagnostic output.  Matches the Polkavm 1.55 and Fuel 1.53
    /// `OutputFormat::as_str` helper idiom.
    #[allow(dead_code)] // Reserved for future metadata emission paths.
    pub fn as_str(self) -> &'static str {
        match self {
            CliTraceFormat::Ctfs => "ctfs",
            CliTraceFormat::Binary => "binary",
            CliTraceFormat::BinaryV0 => "binary_v0",
            CliTraceFormat::Json => "json",
        }
    }
}

/// Helper used by `main.rs` to map a parsed CLI struct onto a fresh
/// [`WasmiRecorder`] instance.  Returns `Ok(None)` when no `--trace-out`
/// directory was given (recorder disabled — the CLI runs as a stock wasmi
/// runtime).
pub fn maybe_open_recorder(
    program_name: &str,
    trace_out: Option<&Path>,
    format: CliTraceFormat,
) -> Result<Option<WasmiRecorder>> {
    let Some(out_dir) = trace_out else {
        return Ok(None);
    };
    let recorder = WasmiRecorder::new(program_name, out_dir.to_path_buf(), format.into())
        .map_err(|e| anyhow!("failed to open CodeTracer recorder: {e:#}"))?;
    Ok(Some(recorder))
}
