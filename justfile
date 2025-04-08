build-wasm-test-c:
  emcc -O0 -g3 -o wasm_test.wasm wasm_test.c

dwarfdump wasm-path:
  # from llvm
  llvm-dwarfdump {{wasm-path}}

dis wasm-path:
  # from binaryen
  wasm-dis {{wasm-path}}
