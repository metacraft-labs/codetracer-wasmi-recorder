;; Minimal trap fixture used by tests/ctfs_audit.rs to verify the
;; read-side CTFS Error special-event path.
(module
  (func $boom (export "boom")
    unreachable)

  (func (export "")
    nop))
