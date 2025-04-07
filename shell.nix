{
  pkgs,
  self',
  inputs',
}:
let
  wasm-rust =
    with inputs'.fenix.packages;
    with latest;
    combine [
      cargo
      rustc
      llvm-tools
      targets.wasm32-unknown-emscripten.latest.rust-std
    ];
in
with pkgs;
mkShell {

  hardeningDisable = [ "all" ];

  packages = [

    cargo
    wasm-rust
    emscripten
    binaryen
    llvm
    rust-analyzer
  ];

}
