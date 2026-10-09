{
  description = "Qbook — терминальная читалка с синхронным переводом";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      lib = nixpkgs.lib;
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAll = f: nixpkgs.lib.genAttrs systems (s: f nixpkgs.legacyPackages.${s});
    in
    {
      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          name = "qbook-dev";

          packages = with pkgs; [
            cargo
            clippy
            rust-analyzer
            rustfmt
            sqlite
          ];
        };
      });

      packages = forAll (
        pkgs:
        let
          qbook =
            extra:
            pkgs.rustPlatform.buildRustPackage (
              {
                pname = "qbook";
                version = "0.1.0";
                src = lib.fileset.toSource {
                  root = ./.;
                  fileset = lib.fileset.unions [
                    ./Cargo.toml
                    ./Cargo.lock
                    ./src
                  ];
                };
                cargoLock.lockFile = ./Cargo.lock;

                meta = {
                  description = "Terminal reader for EPUB/text with synchronised translation switching";
                  mainProgram = "qbook";
                };
              }
              // extra
            );
        in
        {
          default = qbook { };
          with-translate = qbook { cargoFeatures = [ "translate" ]; };
        }
      );

      formatter = forAll (pkgs: pkgs.nixfmt-rfc-style);
    };
}
