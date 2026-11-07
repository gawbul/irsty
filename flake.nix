{
  description = "Irsty, a modern, keyboard-driven terminal IRC client";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      # packaging/nix/package.nix (the nixpkgs package), built from this
      # checkout and Cargo.lock rather than a release tag, so there are no
      # hashes to update.
      packages = forAllSystems (
        pkgs:
        let
          inherit (pkgs) lib;
          irsty = (pkgs.callPackage ./packaging/nix/package.nix { }).overrideAttrs {
            version = (lib.importTOML ./Cargo.toml).workspace.package.version;
            src = lib.fileset.toSource {
              root = ./.;
              fileset = lib.fileset.unions [
                ./Cargo.toml
                ./Cargo.lock
                ./crates
                ./assets
              ];
            };
            cargoDeps = pkgs.rustPlatform.importCargoLock { lockFile = ./Cargo.lock; };
          };
        in
        {
          inherit irsty;
          default = irsty;
        }
      );

      checks = forAllSystems (pkgs: {
        irsty = self.packages.${pkgs.stdenv.hostPlatform.system}.irsty;
      });

      formatter = forAllSystems (pkgs: pkgs.nixfmt);
    };
}
