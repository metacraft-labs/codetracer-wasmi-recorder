use std::path::{Path, PathBuf};
use std::println;

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

// TODO
#[derive(Debug, Clone)]
struct DebugInfo {
    // for now minimal info here
    wasm_exe_path: std::path::PathBuf,
    // TODO eventually dwarf/gimli stuff, not sure
    // TODO: eventually the tracer from runtime_tracer but maybe DebugInfo is just PDO for debug info?A
    // maybe there are several kinds, e.g. frame-based, maybe others
    // so maybe our fn-s will know how to deal with them
    // TODO: local_variables: HashMap<VariableId, DebugLocation>,
}

impl DebugInfo {
    fn new(wasm_exe_path: &Path) -> Self {
        DebugInfo { wasm_exe_path: PathBuf::from(wasm_exe_path) }
    }
}

#[derive(Debug, Clone)]
pub struct WasmTracer {
    debug_info: DebugInfo,
    // TODO: tracer: runtime_tracing.Tracer,
    // etc
}

// just to make it build so we can branch-out

impl WasmTracer {
    pub fn new(wasm_exe_path: &Path) -> Self {
        WasmTracer {
            debug_info: DebugInfo::new(wasm_exe_path),
        }
    }

    pub fn load_local_variables(&mut self, address: usize) { // -> ???
        println!("load_local_variables {address}");
        // e.g. here we might call something like
        // some kind of check if we already have the info for the current context
        // let cached = TODO;
        let cached = false;

        if !cached {
            // TODO etc
            // load debuginfo etc
        }
    }
}

    // load wasm file
    // find out its subprogram and scope for a certain address and somehow
    // 
    // print mapping with expr => relevant location
    // x <- 0x000A7321
    // ip; (debuginfo address of some code(?))
    //
    // 

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
