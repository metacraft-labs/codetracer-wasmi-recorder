## Reprobuild dev env + build recipe for codetracer-wasmi-recorder.
##
## A fork of the upstream ``wasmi-labs/wasmi`` Rust workspace augmented
## with CodeTracer recording hooks. The shipping binary is the
## ``wasmi_cli`` member of the cargo workspace (``crates/cli``); the
## remaining workspace members compile as its transitive deps.
##
## Per ``codetracer-specs/Repo-Requirements.md`` §2.8 the recipe
## expresses build and test execution NATIVELY through typed-tool edges
## (``cargo.build``, ``cargo.test``). It does NOT delegate to
## ``shell(command = "bash scripts/...")`` wrappers — delegation defeats
## the engine's incremental-build, action-cache, per-test invalidation,
## and the CI sharding the engine grows into per
## ``reprobuild-specs/CI-Sharding.md``. This mirrors the sibling
## Rust-recorder recipes (``codetracer-evm-recorder``,
## ``codetracer-trace-format``).
##
## **This repo is a LEAF (from reprobuild's perspective).** The root
## ``Cargo.toml`` is a real cargo workspace whose members resolve against
## each other via in-repo ``path = "..."`` deps. ``crates/cli`` adds two
## CROSS-REPO cargo ``path`` deps to the sibling
## ``codetracer-trace-format`` repo:
##
##   codetracer_trace_types      = { path = "../../../codetracer-trace-format/codetracer_trace_types" }
##   codetracer_trace_writer_nim = { path = "../../../codetracer-trace-format/codetracer_trace_writer_nim" }
##
## Those are consumed by CARGO directly (cargo reads the relative path in
## ``Cargo.toml`` and compiles the sibling crates itself) — they are NOT
## a reprobuild ``uses:``-library consumption. reprobuild's SC-11
## develop-mode src-threading only threads a landed sibling's Nim
## ``src/`` onto reprobuild's OWN ``nim.c`` edges; it has no bearing on a
## cargo ``path`` dependency, which cargo resolves without reprobuild's
## help. So there is no ``uses: "<sibling>"`` edge and no
## develop-override / vcs LockedDep for the trace-format sibling — the
## same treatment ``codetracer-trace-format``'s own recipe applies to its
## cross-repo ``codetracer-trace-format-nim`` build.rs input.
##
## **Cross-repo note — the Nim FFI static library.** The consumed
## ``codetracer_trace_writer_nim`` member crate's ``build.rs`` compiles
## the ``codetracer-trace-format-nim`` FFI entry point into a native
## static library via a direct ``nim c --app:staticlib`` call (running
## ``nimble install --depsOnly`` first to fetch its ``stew`` / ``results``
## requirements) and links libzstd. Because that ``nim c`` / ``capnpc``
## runs INSIDE cargo's build.rs — out of reprobuild's reach — the
## toolchain floor for THIS repo mirrors exactly what
## ``codetracer-trace-format`` and the downstream recorder repos (evm,
## cairo, …) declare for the identical dependency: ``nim`` + ``nimble`` +
## ``capnp`` + ``zstd`` (+ ``pkg-config`` off Windows) on top of the Rust
## toolchain.
##
## **Per-test platform gating.** ``just test`` is ``cargo test
## --locked`` — one whole-workspace cargo run. No test FILE in this repo
## carries a per-file host gate: the ``crates/cli`` integration tests
## (``tests/run.rs``, ``tests/ctfs_audit.rs``) drive the just-built
## ``wasmi_cli`` via ``assert_cmd`` on portable ``.wat`` fixtures, and the
## wasmi / wasi / wast / core / ir crate tests are portable interpreter
## tests. The only ``cfg(target_os = …)`` / ``cfg(unix)`` conditionals in
## the tree live in library ``src/`` (platform-conditional compilation of
## VM code, not test selection), and no test is ``#[ignore]``d. So the
## corpus runs identically on every host cargo supports, and the single
## whole-workspace ``cargo.test`` execute edge below matches the repo's
## own ``just test`` one-for-one — there is no per-OS partition to model.
##
## **Tool provisioning.** ``defaultToolProvisioning "path"`` matches the
## canonical Rust-recorder recipes: the dev shell puts ``cargo`` /
## ``rustc`` / ``nim`` / ``nimble`` / ``capnp`` / ``zstd`` on ``PATH``
## (and ``PKG_CONFIG_PATH`` for libzstd), so the weak-local PATH resolver
## is the right default. Without it ``repro build`` refuses to run with
## "typed tool provisioning is required for uses declarations".

import repro_project_dsl

package codetracer_wasmi_recorder:
  defaultToolProvisioning "path"

  uses:
    # Rust toolchain — declared by version so the tarball-direct
    # provisioning entries in repro_dsl_stdlib/packages/cargo.nim /
    # rustc.nim resolve on Windows. On Linux/macOS the dev shell supplies
    # the same versions. The floor matches the workspace
    # (``rust-version = "1.83"`` in the root ``Cargo.toml``).
    "rustc >=1.83"
    "cargo >=1.83"

    # Nim toolchain — the cross-repo ``codetracer_trace_writer_nim``
    # path-dep crate's build.rs compiles the ``codetracer-trace-format-nim``
    # FFI entry point into a static library at cargo build time via
    # ``nim c``. ``nimble`` is invoked by the same build.rs
    # (``nimble install --depsOnly``) to resolve that FFI's ``stew`` /
    # ``results`` nimble requirements.
    "nim >=2.2 <3.0"
    "nimble"

    # Cap'n Proto schema compiler — the trace-format capnp crate pulled in
    # transitively by ``codetracer_trace_writer_nim`` runs ``capnpc`` over
    # its schema at build time.
    "capnp"

    # libzstd headers + library — the Nim FFI's C output ``#include``s
    # ``zstd.h`` and the writer links libzstd (via ``zstd-sys``); build.rs
    # threads the zstd include dir onto the Nim C compile.
    "zstd"

    # pkg-config — the zstd link step consults pkg-config to find libzstd
    # on Linux/macOS. Not on the Windows floor (zstd-sys builds libzstd
    # from source there).
    when not defined(windows):
      "pkg-config"

    # Chocolatey — `choco pack` / `choco push` in this repo's
    # .github/workflows/publish-chocolatey.yml. Declared here rather than
    # installed on the runner: a tool this repo needs is this repo's
    # dependency, and leaving it to the machine means a developer and CI each
    # get whatever their box happens to carry.
    #
    # BOTH halves below are load-bearing and they answer different questions.
    # `platforms: [windows]` on the package (reprobuild's catalog) says where
    # chocolatey CAN exist; this guard says whether THIS recipe needs it here.
    # No package-side declaration can answer the second — which is why Nix
    # keeps meta.platforms beside lib.optionals, and Spack requires() beside
    # depends_on(when=). Do not delete the guard on the grounds that the
    # package now declares its platform.
    when defined(windows):
      "chocolatey"

  # The shipping binary — the ``wasmi_cli`` workspace member. The
  # per-app cargo build edge is emitted in the ``build:`` block below.
  executable wasmiCli:
    name: "wasmi_cli"

  devEnv:
    activity "default"

  build:
    # ---- Primary build edge (the `default` collection) ----------------
    #
    # Native whole-workspace cargo build. The root ``Cargo.toml`` is a
    # real workspace with members; bare ``cargo build`` builds every
    # member including ``wasmi_cli`` — exactly what ``just build`` →
    # ``cargo build --locked`` does. Enrolled into the conventional
    # ``default`` collection per
    # reprobuild-specs/Build-Graph-Collections.md §"`default`" so
    # ``repro build`` (no positional target) materialises this edge's
    # closure.
    #
    # ``locked = true`` because the root ``Cargo.lock`` IS committed
    # (``git ls-files`` tracks it) and the Justfile builds with
    # ``--locked``: the build must fail rather than silently regenerate
    # the lock if a member's ``Cargo.toml`` drifts from the pinned
    # resolution.
    #
    # The union of every workspace member's source root + the root
    # manifest/lock is declared as ``extraInputs`` so the engine tracks
    # the whole workspace tree as the build edge's input set (cargo's own
    # ``.d`` depfiles under ``target/*/deps`` refine this per-crate at
    # action-end via the makeDepfile dependency policy the cargo package
    # declares).
    const binarySuffix = (when defined(windows): ".exe" else: "")
    const cliBinary = "target/release/wasmi_cli" & binarySuffix

    let workspaceInputs = @[
      "Cargo.toml", "Cargo.lock",
      "crates",
      "fuzz",
    ]

    let cliBuild = cargo.build(
      release = true,
      locked = true,
      actionId = "codetracer-wasmi-recorder.cargo-build",
      extraInputs = workspaceInputs,
      extraOutputs = @[cliBinary])
    discard collect("default", @[cliBuild])

    # ---- Test-binary build + run edges (the `test` collection) -------
    #
    # Two-stage shape per Repo-Requirements.md §2.8: ``cargo.test(noRun =
    # true)`` builds every workspace test binary into
    # ``target/debug/deps/<crate>-<hash>`` (the engine tracks the deps
    # directory as the build edge's effect set because the hashed
    # filename floats with input content); the second ``cargo.test``
    # (``noRun`` defaulting to false) then runs the binaries in one cargo
    # invocation — the same whole-workspace pass ``just test`` →
    # ``cargo test --locked`` performs. The execute edge depends on the
    # build edge so the engine only re-runs tests when an input changed
    # since the last successful execution.
    #
    # Per-test execute edges fall out automatically once the
    # ct-test-runner cargo adapter lands per
    # reprobuild-specs/Test-Edges-And-Parallel-Runner.milestones.org
    # §M4 — the whole-binary edge becomes a fan-out point without
    # changing this recipe.

    let testsBuild = cargo.test(
      noRun = true,
      locked = true,
      actionId = "codetracer-wasmi-recorder.cargo-test-build",
      after = @[cliBuild],
      extraInputs = workspaceInputs,
      extraOutputs = @["target/debug/deps"])

    let testsRun = cargo.test(
      locked = true,
      actionId = "codetracer-wasmi-recorder.cargo-test-run",
      after = @[testsBuild.action],
      extraInputs = workspaceInputs & @["target/debug/deps"])

    discard collect("test", @[testsRun.action])
