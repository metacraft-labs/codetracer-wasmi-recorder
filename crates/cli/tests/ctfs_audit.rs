//! CTFS audit smoke tests for the wasmi recorder integration (mission
//! goals #5 / #6 — see `AUDIT-CTFS-2026-05.md`).
//!
//! These tests pin the audit invariants documented in section 5.6 of
//! `/tmp/isonim-migration.txt` and mirror the audit-test idiom used by the
//! sibling Rust-native VM recorders:
//!   * PolkaVM 1.55 — `tests/test_ctfs_audit.rs::ctfs_writer_produces_ct_container`
//!   * Miden 1.56   — `tests/test_ctfs_audit.rs`
//!   * TON 1.57     — `tests/ctfs_audit.rs`
//!   * Leo 1.59     — `tests/ctfs_audit.rs`
//!
//! Each test runs the just-built `wasmi_cli` binary via `assert_cmd` (same
//! harness as the existing `tests/run.rs`) on a fixture `.wat` module,
//! passing `--trace-out <tmpdir>` so the recorder is enabled.  The
//! resulting trace directory is then walked structurally.  The read-side
//! assertions use `codetracer_trace_writer_nim::NimTraceReaderHandle`
//! so the audit pins concrete CTFS event content instead of only checking
//! that the directory is non-empty.

use assert_cmd::Command;
use codetracer_trace_types::ValueRecord;
use codetracer_trace_writer_nim::NimTraceReaderHandle;
use std::path::PathBuf;

/// Root for the test fixture .wat files (shared with `tests/run.rs`).
fn wat_path(name: &str) -> PathBuf {
    let mut p = PathBuf::new();
    p.push("tests");
    p.push("wats");
    p.push(format!("{name}.wat"));
    p
}

fn cmd() -> Command {
    Command::cargo_bin("wasmi_cli").expect("could not locate wasmi_cli binary")
}

fn record_add_trace(arg0: &str, arg1: &str) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let trace_out = tmp.path().join("trace");

    cmd()
        .arg(wat_path("ctfs_add"))
        .arg("--invoke")
        .arg("add")
        .arg("--trace-out")
        .arg(&trace_out)
        .arg(arg0)
        .arg(arg1)
        .assert()
        .success();

    (tmp, trace_out)
}

fn open_ctfs_reader(trace_out: &std::path::Path) -> NimTraceReaderHandle {
    let ct_path = ct_file_in(trace_out);
    NimTraceReaderHandle::open(&ct_path.to_string_lossy())
        .unwrap_or_else(|e| panic!("failed to open Nim CTFS reader for {ct_path:?}: {e}"))
}

fn bytes_from_json_array(value: &serde_json::Value) -> Vec<u8> {
    value
        .as_array()
        .unwrap_or_else(|| panic!("expected byte array JSON, got {value:#}"))
        .iter()
        .map(|byte| {
            byte.as_u64()
                .unwrap_or_else(|| panic!("expected byte value, got {byte:#}")) as u8
        })
        .collect()
}

fn decode_value_record(value: &serde_json::Value) -> ValueRecord {
    let bytes = bytes_from_json_array(value);
    cbor4ii::serde::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("failed to decode ValueRecord from {bytes:?}: {e}"))
}

fn value_as_i64(value: &ValueRecord) -> Option<i64> {
    match value {
        ValueRecord::Int { i, .. } => Some(*i),
        _ => None,
    }
}

fn string_from_json_bytes(value: &serde_json::Value) -> String {
    String::from_utf8(bytes_from_json_array(value))
        .unwrap_or_else(|e| panic!("event data was not UTF-8: {e}"))
}

fn ct_file_in(trace_out: &std::path::Path) -> PathBuf {
    let mut ct_files = Vec::new();
    collect_ct_files(trace_out, &mut ct_files);
    assert!(
        !ct_files.is_empty(),
        "expected a .ct container below {trace_out:?}"
    );
    ct_files.sort();
    ct_files.remove(0)
}

fn collect_ct_files(root: &std::path::Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_ct_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "ct") {
            out.push(path);
        }
    }
}

/// `--help` advertises `--trace-out` and `--trace-format` (default `ctfs`).
///
/// This pins the audit-(a) gap closure so a future regression that drops
/// the flags or flips the default trips the test.  Same idiom as
/// PolkaVM 1.55 `ctfs_format_advertised_in_record_help`.
#[test]
fn audit_ctfs_help_advertises_trace_flags() {
    let assert = cmd().arg("--help").assert().success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    assert!(
        stdout.contains("--trace-out"),
        "--trace-out flag missing from CLI help:\n{stdout}"
    );
    assert!(
        stdout.contains("--trace-format"),
        "--trace-format flag missing from CLI help:\n{stdout}"
    );
    // The default-Ctfs-CLI invariant.  Same as Leo 1.59 / PolkaVM 1.55.
    assert!(
        stdout.contains("[default: ctfs]"),
        "ctfs is not the default --trace-format value:\n{stdout}"
    );
}

/// `--trace-out <dir>` produces a non-empty trace artefact in the
/// requested directory when the recorder is enabled (default ctfs).
///
/// Pins audit-(g) "canonical CTFS schema match" at the smoke level: the
/// directory is non-empty after the run, so the writer materialised
/// something on close.  The exact filenames vary by format (CTFS multi-
/// stream container vs. legacy CBOR+Zstd trace.bin vs. JSON sidecars), so
/// we walk recursively rather than asserting a specific layout.
#[test]
fn audit_ctfs_writer_produces_trace_files() {
    let (_tmp, trace_out) = record_add_trace("7", "35");

    let count = walk_count_files(&trace_out);
    assert!(
        count > 0,
        "trace output directory is empty after recorder run: {trace_out:?}"
    );
}

/// Read the produced CTFS container back and assert the top-level wasm
/// call record carries the expected function id, staged args, and return
/// value.  This is the read-side assertion follow-up from audit 1.65.
#[test]
fn audit_ctfs_reader_sees_add_call_args_and_return() {
    let (_tmp, trace_out) = record_add_trace("7", "35");
    let reader = open_ctfs_reader(&trace_out);

    let function_names: Vec<_> = (0..reader.function_count())
        .map(|id| reader.function(id).expect("read function name"))
        .collect();
    assert!(
        function_names.iter().any(|name| name.contains("add")),
        "missing registered add function in {function_names:#?}"
    );

    let calls: Vec<_> = (0..reader.call_count())
        .map(|key| reader.call_json(key).expect("read call JSON"))
        .collect();
    assert_eq!(
        calls.len(),
        1,
        "expected one top-level add call; calls={calls:#?}"
    );
    let add_call: serde_json::Value =
        serde_json::from_str(&calls[0]).unwrap_or_else(|e| panic!("invalid call JSON: {e}"));
    let args = add_call["args"]
        .as_array()
        .unwrap_or_else(|| panic!("call args should be an array: {add_call:#}"));
    assert_eq!(args.len(), 2, "add call should have two args: {add_call:#}");

    let arg_names: Vec<_> = args
        .iter()
        .map(|arg| {
            let id = arg["varname_id"]
                .as_u64()
                .unwrap_or_else(|| panic!("arg missing varname_id: {arg:#}"));
            reader.varname(id).expect("read arg varname")
        })
        .collect();
    assert_eq!(arg_names, vec!["arg0", "arg1"]);

    let arg_values: Vec<_> = args
        .iter()
        .map(|arg| value_as_i64(&decode_value_record(&arg["value"])))
        .collect();
    assert_eq!(arg_values, vec![Some(7), Some(35)]);

    let return_value = decode_value_record(&add_call["return_value"]);
    assert_eq!(value_as_i64(&return_value), Some(42));
}

/// Runtime traps are routed through the canonical CTFS error event path
/// and still close the top-level call with a placeholder return.
#[test]
fn audit_ctfs_reader_sees_trap_error_event() {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let trace_out = tmp.path().join("trace");

    cmd()
        .arg(wat_path("ctfs_trap"))
        .arg("--invoke")
        .arg("boom")
        .arg("--trace-out")
        .arg(&trace_out)
        .assert()
        .failure();

    let reader = open_ctfs_reader(&trace_out);
    let events: Vec<_> = (0..reader.event_count())
        .map(|idx| reader.event_json(idx).expect("read event JSON"))
        .collect();
    let error_events: Vec<serde_json::Value> = events
        .iter()
        .map(|event| serde_json::from_str(event).expect("parse event JSON"))
        .filter(|event: &serde_json::Value| event["kind"] == "error")
        .collect();
    assert_eq!(
        error_events.len(),
        1,
        "expected one wasmi trap error event: {events:#?}"
    );
    let error_content = string_from_json_bytes(&error_events[0]["data"]);
    assert!(
        error_content.contains("unreachable"),
        "trap content should mention unreachable: {error_content}"
    );

    let calls: Vec<_> = (0..reader.call_count())
        .map(|key| reader.call_json(key).expect("read call JSON"))
        .collect();
    assert_eq!(
        calls.len(),
        1,
        "trap path should close one call frame: {calls:#?}"
    );
    let trap_call: serde_json::Value =
        serde_json::from_str(&calls[0]).unwrap_or_else(|e| panic!("invalid call JSON: {e}"));
    assert_eq!(
        bytes_from_json_array(&trap_call["return_value"]),
        vec![255],
        "trap path should close the call with the Nim reader's NONE_VALUE marker: {trap_call:#}"
    );
}

/// When `--trace-out` is unset, the recorder MUST NOT create any side
/// effects on the filesystem.  This is the "default upstream behaviour"
/// invariant — wasmi has shipped as a stock WebAssembly runtime for years
/// and the audit must not regress that for users who do not opt in.
#[test]
fn audit_ctfs_no_trace_out_means_no_recorder() {
    let tmp = tempfile::tempdir().expect("create tempdir");
    // Run inside the tempdir so any accidentally-emitted relative-path
    // artefacts land here and we can detect them.
    cmd()
        .current_dir(tmp.path())
        .arg(
            wat_path("ctfs_add")
                .canonicalize()
                .expect("canonicalize wat path"),
        )
        .arg("--invoke")
        .arg("add")
        .arg("7")
        .arg("35")
        .assert()
        .success();

    let count = walk_count_files(tmp.path());
    assert_eq!(
        count,
        0,
        "wasmi_cli without --trace-out unexpectedly produced files in cwd: {count} entries in {:?}",
        tmp.path()
    );
}

/// `--trace-format` accepts every documented format identifier (audit
/// invariant: a future reordering / rename of the format enum trips this
/// test the way Leo 1.59 / Miden 1.56 audit suites pin format-constant
/// shapes).
#[test]
fn audit_ctfs_format_values_accepted() {
    for fmt in ["ctfs", "binary", "binary_v0", "json"] {
        let tmp = tempfile::tempdir().expect("create tempdir");
        let trace_out = tmp.path().join(format!("trace-{fmt}"));
        cmd()
            .arg(wat_path("ctfs_add"))
            .arg("--invoke")
            .arg("add")
            .arg("--trace-out")
            .arg(&trace_out)
            .arg("--trace-format")
            .arg(fmt)
            .arg("1")
            .arg("2")
            .assert()
            .success();
    }
}

/// Recursive file count helper: returns the number of regular files at
/// or below `root`.  Returns 0 if `root` does not exist, which is the
/// "no recorder, no side effects" invariant the negative test depends on.
fn walk_count_files(root: &std::path::Path) -> usize {
    fn walk(p: &std::path::Path, acc: &mut usize) {
        let Ok(entries) = std::fs::read_dir(p) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, acc);
            } else if path.is_file() {
                *acc += 1;
            }
        }
    }
    let mut count = 0;
    walk(root, &mut count);
    count
}
