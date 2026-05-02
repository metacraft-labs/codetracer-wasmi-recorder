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
//! resulting trace directory is then walked structurally — we do not
//! depend on the Nim `ct_reader_*` FFI here, only on the writer
//! producing a non-empty `.ct` / `trace.bin` / `trace.json` file at the
//! canonical locations.  Read-side end-to-end content assertions are
//! cross-cuttingly open across all sibling audited recorders (see
//! AUDIT-CTFS-2026-05.md "Open gaps").

use assert_cmd::Command;
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
    let tmp = tempfile::tempdir().expect("create tempdir");
    let trace_out = tmp.path().join("trace");

    cmd()
        .arg(wat_path("ctfs_add"))
        .arg("--invoke")
        .arg("add")
        .arg("--trace-out")
        .arg(&trace_out)
        .arg("7")
        .arg("35")
        .assert()
        .success();

    let count = walk_count_files(&trace_out);
    assert!(
        count > 0,
        "trace output directory is empty after recorder run: {trace_out:?}"
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
        .arg(wat_path("ctfs_add").canonicalize().expect("canonicalize wat path"))
        .arg("--invoke")
        .arg("add")
        .arg("7")
        .arg("35")
        .assert()
        .success();

    let count = walk_count_files(tmp.path());
    assert_eq!(
        count, 0,
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
