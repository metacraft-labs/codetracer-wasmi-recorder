{
  description = "CodeTracer Wasmi Recorder";

  inputs = {
    mcl-blockchain.url = "github:metacraft-labs/nix-blockchain-development";
    nixpkgs.follows = "mcl-blockchain/nixpkgs";
    flake-utils.follows = "mcl-blockchain/flake-utils";
    fenix.follows = "mcl-blockchain/fenix";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      fenix,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };

        # `.rustfmt.toml` is upstream wasmi's, and it sets nightly-only options
        # (`imports_granularity`, `imports_layout`). A stable rustfmt ignores
        # them and reports differences in files that are correctly formatted,
        # so the formatter is the nightly that `.github/workflows/rust.yml`
        # pins as RUST_NIGHTLY_VERSION. Only rustfmt comes from it; the code
        # is compiled and linted by the stable toolchain below.
        nightlyRustfmt =
          (fenix.packages.${system}.toolchainOf {
            channel = "nightly";
            date = "2025-03-30";
            sha256 = "sha256-gt1sxei5nfzKPtM5aAs93CwliYcGaFNsgWz0rKfssb8=";
          }).rustfmt;
      in
      {
        devShells.default = pkgs.mkShell {
          packages = [
            # `rustfmt` first, so PATH resolves `rustfmt` / `cargo-fmt` to it.
            nightlyRustfmt
            pkgs.rustc
            pkgs.cargo
            pkgs.clippy
            # The sibling `codetracer_trace_writer_nim` crate's build.rs
            # compiles the Nim trace writer and links libzstd; the capnp
            # crate it pulls in runs `capnp` over its schema.
            pkgs.nim
            pkgs.nimble
            pkgs.capnproto
            pkgs.zstd
            pkgs.pkg-config
            pkgs.just
          ];

          # `cargo <subcommand>` looks for `cargo-<subcommand>` in
          # `$CARGO_HOME/bin` BEFORE it searches PATH. On any machine with
          # rustup — including the self-hosted macOS runner — that directory
          # holds rustup's proxies, so `cargo fmt` and `cargo clippy` run
          # rustup's `cargo-fmt` / `cargo-clippy` instead of the shell's, and
          # fail with "'cargo-fmt' is not installed for the toolchain".
          #
          # The shell therefore gets its own CARGO_HOME with an empty `bin/`,
          # so subcommand lookup falls through to PATH. `registry/` and `git/`
          # are symlinks to the real CARGO_HOME, and so are its config and
          # credentials when present: the download cache is shared, and only
          # the proxy directory is left behind.
          shellHook = ''
            _wasmi_real_cargo_home="''${CARGO_HOME:-$HOME/.cargo}"
            _wasmi_cargo_home="''${XDG_CACHE_HOME:-$HOME/.cache}/codetracer-wasmi-recorder/cargo-home"
            if [ "$_wasmi_real_cargo_home" != "$_wasmi_cargo_home" ]; then
              mkdir -p "$_wasmi_cargo_home" \
                "$_wasmi_real_cargo_home/registry" "$_wasmi_real_cargo_home/git"
              # Re-pointed on every entry, so a changed CARGO_HOME is followed
              # rather than left sharing the previous one's cache. Only a link
              # is ever replaced; a real file placed here is left alone.
              for _wasmi_entry in registry git config.toml credentials.toml; do
                if [ -e "$_wasmi_real_cargo_home/$_wasmi_entry" ] &&
                  { [ -L "$_wasmi_cargo_home/$_wasmi_entry" ] ||
                    [ ! -e "$_wasmi_cargo_home/$_wasmi_entry" ]; }; then
                  ln -sfn "$_wasmi_real_cargo_home/$_wasmi_entry" "$_wasmi_cargo_home/$_wasmi_entry"
                fi
              done
              export CARGO_HOME="$_wasmi_cargo_home"
            fi
            unset _wasmi_real_cargo_home _wasmi_cargo_home _wasmi_entry
          '';
        };
      }
    );
}
