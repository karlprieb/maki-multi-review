{
  description = "maki-multi-review - multi-perspective code review plugin for maki";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    { nixpkgs, rust-overlay, ... }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forEachSystem =
        f:
        nixpkgs.lib.genAttrs systems (
          system:
          f (
            import nixpkgs {
              inherit system;
              overlays = [ (import rust-overlay) ];
            }
          )
        );
    in
    {
      devShells = forEachSystem (pkgs: {
        default = pkgs.mkShell {
          packages = [
            (pkgs.rust-bin.stable."1.95.0".default.override {
              extensions = [
                "rust-src"
                "rust-analyzer"
              ];
            })
            pkgs.cargo-nextest
            pkgs.just
            pkgs.stylua
            pkgs.nixfmt
            pkgs.pkg-config
            pkgs.openssl
            pkgs.perl
            pkgs.python3
          ];

          OPENSSL_NO_VENDOR = "1";

          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [
            pkgs.openssl
            pkgs.stdenv.cc.cc.lib
          ];
        };
      });

      formatter = forEachSystem (pkgs: pkgs.nixfmt);
    };
}
