use crate::{
    args::Args,
    display::{DisplayExportedFuncs, DisplayFuncType, DisplaySequence, DisplayValue},
    recorder::maybe_open_recorder,
};
use anyhow::{anyhow, bail, Error, Result};
use clap::Parser;
use context::Context;
use std::{path::Path, process};
use wasmi::{Func, FuncType, Val};

mod args;
mod context;
mod display;
mod recorder;
mod utils;

#[cfg(test)]
mod tests;

fn main() -> Result<()> {
    let args = Args::parse();
    let wasm_file = args.wasm_file();
    let wasi_ctx = args.wasi_context()?;
    let mut ctx = Context::new(wasm_file, wasi_ctx, args.fuel(), args.compilation_mode())?;
    let (func_name, func) = get_invoked_func(&args, &ctx)?;
    let ty = func.ty(ctx.store());
    let func_args = utils::decode_func_args(&ty, args.func_args())?;
    let mut func_results = utils::prepare_func_results(&ty);
    typecheck_args(&func_name, &ty, &func_args)?;

    if args.verbose() {
        print_execution_start(args.wasm_file(), &func_name, &func_args);
    }
    if args.invoked().is_some() && ty.params().len() != args.func_args().len() {
        bail!(
            "invalid amount of arguments given to function {}. expected {} but received {}",
            DisplayFuncType::new(&func_name, &ty),
            ty.params().len(),
            args.func_args().len()
        )
    }

    // Open the CodeTracer recorder, if `--trace-out` was provided.  The
    // recorder is `None` for the default `wasmi` invocation (stock runtime
    // mode) and `Some(_)` for the audit / replay use case.
    let program_name = args
        .wasm_file()
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("<wasm>");
    let mut recorder =
        maybe_open_recorder(program_name, args.trace_out(), args.trace_format())?;

    // If the recorder is enabled, stage the CLI-provided argv slice as
    // canonical CTFS call args (`arg0..argN-1`) and open the top-level
    // call frame BEFORE invoking the wasm function.  This mirrors the
    // PolkaVM 1.55 Ecalli pre-call staging pattern (writer.arg(...) for
    // each A0..A5, then register_call) and the Miden 1.56 stack-arg
    // staging pattern.
    if let Some(rec) = recorder.as_mut() {
        rec.register_top_level_call(&func_name, &ty, &func_args);
    }

    let call_result = func.call(ctx.store_mut(), &func_args, &mut func_results);

    match call_result {
        Ok(()) => {
            // Close the call frame on the recorder side first so the .ct
            // container is structurally well-formed even if downstream
            // post-print logic panics.
            if let Some(rec) = recorder.as_mut() {
                rec.register_top_level_return(&func_results);
            }
            // Explicitly finish (flush + close all three trace streams)
            // BEFORE we print anything else, so a writer-side error
            // surfaces with a non-zero exit instead of being silently
            // dropped in `Drop`.  `finish()` consumes the recorder; we
            // `.take()` it out of the `Option` so the value is moved.
            // This wiring closes the audit gap flagged in
            // AUDIT-CTFS-2026-05.md (the previous `drop(recorder)` left
            // the trace streams half-flushed in the ad-hoc happy path).
            if let Some(rec) = recorder.take() {
                rec.finish()?;
            }

            print_remaining_fuel(&args, &ctx);
            print_pretty_results(&func_results);
            Ok(())
        }
        Err(error) => {
            // Route the wasmi runtime trap through
            // register_special_event(EventLogKind::Error, "wasmi_trap", ...)
            // before the CLI bails out.  This mirrors EVM 1.39 / Cairo 1.50
            // / PolkaVM 1.55 / Miden 1.56 / TON 1.57 trap routing — the
            // canonical CTFS error channel surfaces the wasm trap on
            // `ct/load-events` even though the process exits non-zero.
            //
            // Done BEFORE the i32_exit_status branch because a clean WASI
            // exit (proc_exit) is reported as an i32_exit_status rather
            // than a real trap; we want a recorder breadcrumb either way.
            if let Some(rec) = recorder.as_mut() {
                if error.i32_exit_status().is_some() {
                    // Treat WASI proc_exit as a normal return (the wasm
                    // ran to completion from the contract's POV).
                    rec.register_top_level_return(&func_results);
                } else {
                    rec.register_trap(&error);
                }
            }
            // Same finish() rationale as the success path: surface
            // writer-side errors instead of swallowing them in `Drop`.
            // For the WASI proc_exit branch we also need the trace to
            // be flushed BEFORE `process::exit` (which skips destructors
            // entirely).  This is the load-bearing reason we cannot
            // rely on `Drop`.
            if let Some(rec) = recorder.take() {
                if let Err(finish_err) = rec.finish() {
                    eprintln!("warning: failed to finalise CodeTracer trace: {finish_err:#}");
                }
            }

            if let Some(exit_code) = error.i32_exit_status() {
                // We received an exit code from the WASI program,
                // therefore we exit with the same exit code after
                // pretty printing the results.
                print_remaining_fuel(&args, &ctx);
                print_pretty_results(&func_results);
                process::exit(exit_code)
            }
            bail!("failed during execution of {func_name}: {error}")
        }
    }
}

/// Prints the remaining fuel so far if fuel metering was enabled.
fn print_remaining_fuel(args: &Args, ctx: &Context) {
    if let Some(given_fuel) = args.fuel() {
        let remaining = ctx
            .store()
            .get_fuel()
            .unwrap_or_else(|error| panic!("could not get the remaining fuel: {error}"));
        let consumed = given_fuel.saturating_sub(remaining);
        println!("fuel consumed: {consumed}, fuel remaining: {remaining}");
    }
}

/// Performs minor typecheck on the function signature.
///
/// # Note
///
/// This is not strictly required but improve error reporting a bit.
///
/// # Errors
///
/// If too many or too few function arguments were given to the invoked function.
fn typecheck_args(func_name: &str, func_ty: &FuncType, args: &[Val]) -> Result<(), Error> {
    if func_ty.params().len() != args.len() {
        bail!(
            "invalid amount of arguments given to function {}. expected {} but received {}",
            DisplayFuncType::new(func_name, func_ty),
            func_ty.params().len(),
            args.len()
        )
    }
    Ok(())
}

/// Returns the invoked named function or the WASI entry point to the Wasm module if any.
///
/// # Errors
///
/// - If the function given via `--invoke` could not be found in the Wasm module.
/// - If `--invoke` was not given and no WASI entry points were exported.
fn get_invoked_func(args: &Args, ctx: &Context) -> Result<(String, Func), Error> {
    match args.invoked() {
        Some(func_name) => {
            let func = ctx
                .get_func(func_name)
                .map_err(|error| anyhow!("{error}\n\n{}", DisplayExportedFuncs::from(ctx)))?;
            let func_name = func_name.into();
            Ok((func_name, func))
        }
        None => {
            // No `--invoke` flag was provided so we try to find
            // the conventional WASI entry points `""` and `"_start"`.
            if let Ok(func) = ctx.get_func("") {
                Ok(("".into(), func))
            } else if let Ok(func) = ctx.get_func("_start") {
                Ok(("_start".into(), func))
            } else {
                bail!(
                    "did not specify `--invoke` and could not find exported WASI entry point functions\n\n{}",
                    DisplayExportedFuncs::from(ctx)
                )
            }
        }
    }
}

/// Prints a signalling text that Wasm execution has started.
fn print_execution_start(wasm_file: &Path, func_name: &str, func_args: &[Val]) {
    println!(
        "executing File({wasm_file:?})::{func_name}({}) ...",
        DisplaySequence::new(", ", func_args.iter().map(DisplayValue::from))
    );
}

/// Prints the results of the Wasm computation in a human readable form.
fn print_pretty_results(results: &[Val]) {
    for result in results {
        println!("{}", DisplayValue::from(result))
    }
}
