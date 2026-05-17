{
  description = "PureScript Alexandrite development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        rustToolchain = pkgs.rust-bin.selectLatestNightlyWith (toolchain: toolchain.default.override {
          extensions = [ "rust-src" "rust-analyzer" "clippy" "rustfmt" ];
          targets = [ "wasm32-unknown-unknown" ];
        });

      in
      {
        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            rustToolchain
            just
            pkg-config
            openssl
            
            # Node.js and frontend tools
            nodejs_22
            pnpm
            
            # WASM tools
            wasm-pack
            
            # Rust utilities
            cargo-nextest
            cargo-llvm-cov
            cargo-criterion
          ];

          shellHook = ''
            export PATH="$PATH:$HOME/.cargo/bin"
            echo "PureScript Alexandrite Dev Shell"
            echo "Rust: $(rustc --version)"
            echo "Node: $(node --version)"
            echo "PNPM: $(pnpm --version)"
          '';
        };
      }
    );
}
