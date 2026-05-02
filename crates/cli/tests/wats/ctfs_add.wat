;; Minimal "add two i32s" wasm module used by tests/ctfs_audit.rs.
;;
;; The default entry-point name (`""`) returns no value; for the audit
;; smoke test we invoke the named export `add` instead via `--invoke add`,
;; which lets us exercise the recorder's call-arg staging path
;; (arg0=7, arg1=35 -> ValueRecord::Int).
(module
    (func $add (export "add")
        (param $a i32) (param $b i32)
        (result i32)
        local.get $a
        local.get $b
        i32.add)

    ;; The CLI uses the empty-named export as the implicit entry-point when
    ;; `--invoke` is not given.  Provide a no-op to keep the structural CLI
    ;; shape (no recorder is opened in that path; the explicit `--invoke
    ;; add` route is the audit fixture).
    (func (export "")
        nop))
