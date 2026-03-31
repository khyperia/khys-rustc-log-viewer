{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";

    # for building rust packages
    naersk.url = "github:nix-community/naersk";
    # for eary pre-built toolchains
    nixpkgs-mozilla = {
      url = "github:mozilla/nixpkgs-mozilla";
      flake = false;
    };
  };
  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      nixpkgs-mozilla,
      naersk,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = (import nixpkgs) {
          inherit system;
          overlays = [
            (import nixpkgs-mozilla)
          ];
        };
      in
      {
        packages = rec {
          viewlog = pkgs.callPackage ./packages/viewlog.nix {
            inherit naersk;
          };
          default = viewlog;
        };
        devShells.default =
          with pkgs;
          mkShell {
            nativeBuildInputs = [
              openssl
            ];
            buildInputs = [
              pkg-config
              clang
              llvmPackages_latest.bintools
              cargo
              rustc
            ];
            packages = [
              gdb
            ];
            shellHook = ''
              export LIBCLANG_PATH="${lib.makeLibraryPath [ llvmPackages_latest.libclang.lib ]}"
              export LD_LIBRARY_PATH="'$LD_LIBRARY_PATH:${
                lib.makeLibraryPath [
                  openssl
                  libxkbcommon
                  wayland
                  vulkan-loader
                ]
              }"
              PKG_CONFIG_PATH="${openssl.dev}/lib/pkgconfig";
            '';
          };
      }
    );
}
