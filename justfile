build-wasm-test-c:
  emcc -O0 -g3 -o wasm_test.wasm wasm_test.c

dwarfdump wasm-path:
  # from llvm
  llvm-dwarfdump {{wasm-path}}

dis wasm-path:
  # from binaryen
  wasm-dis {{wasm-path}}

trace wasm-path:
  env CODETRACER_WASMI_TRACING=1 CODETRACER_WASM_PATH=$(realpath {{wasm-path}}) target/debug/wasmi_cli wasm_test.wasm
