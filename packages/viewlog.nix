{
  naersk,
  callPackage,
  pkg-config,
  llvmPackages_latest,
  clang,
  lib,
  libxkbcommon,
  wayland,
  vulkan-loader,
  cargo,
  rustc,
}:
let
  naersk' = callPackage naersk {
    inherit cargo rustc;
  };
in
naersk'.buildPackage {
  src = ../.;
  buildInputs = [
    pkg-config
    clang
    llvmPackages_latest.bintools
  ];
  LIBCLANG_PATH = lib.makeLibraryPath [
    llvmPackages_latest.libclang.lib
    libxkbcommon
    wayland
    vulkan-loader
  ];
}
