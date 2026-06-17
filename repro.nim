## Reprobuild dev env + build recipe for codetracer-wasmi-recorder.
##
## A fork of the upstream ``wasmi-labs/wasmi`` Rust workspace
## augmented with CodeTracer recording hooks. The shipping binary
## is the ``wasmi_cli`` member of the cargo workspace; the rest of
## the workspace members compile as transitive deps.
##
## Per ``codetracer-specs/Repo-Requirements.md`` §2.8 the recipe
## expresses build and test execution NATIVELY through typed-tool
## edges (`cargo.build`, `cargo.test`). No shell delegation.

import repro_project_dsl

package codetracer_wasmi_recorder:
  uses:
    "rustc >=1.83"
    "cargo >=1.83"

  executable wasmiCli:
    name: "wasmi_cli"

  devEnv:
    activity "default"

  build:
    const binarySuffix = (when defined(windows): ".exe" else: "")
    const cliBinary = "target/release/wasmi_cli" & binarySuffix

    let cliBuild = cargo.build(
      release = true,
      manifestPath = "crates/cli/Cargo.toml",
      actionId = "codetracer-wasmi-recorder.cargo-build",
      extraInputs = @[
        "Cargo.toml", "Cargo.lock",
        "crates/cli", "crates/wasmi", "crates/core", "crates/ir"
      ],
      extraOutputs = @[cliBinary])
    discard collect("default", @[cliBuild])

    let testsBuild = cargo.test(
      noRun = true,
      actionId = "codetracer-wasmi-recorder.cargo-test-build",
      extraInputs = @["Cargo.toml", "Cargo.lock", "crates"],
      extraOutputs = @["target/debug/deps"])

    let testsRun = cargo.test(
      actionId = "codetracer-wasmi-recorder.cargo-test-run",
      after = @[testsBuild.action],
      extraInputs = @[
        "Cargo.toml", "Cargo.lock",
        "crates",
        "target/debug/deps"
      ])

    discard collect("test", @[testsRun.action])
