{
  description = "An unofficial annil implementation";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

    crane.url = "github:ipetkov/crane";

    flake-utils.url = "github:numtide/flake-utils";
  };

  nixConfig = {
    extra-substituters = [ "https://annil-server.cachix.org" ];
    extra-trusted-public-keys = [ "annil-server.cachix.org-1:ioHVMApnJQ8UDnQRzkGR4hDVJ0xTwpphc/6bffyxXXA=" ];
  };

  outputs = { self, nixpkgs, crane, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};

        craneLib = crane.mkLib pkgs;

        commonArgs = {
          src = craneLib.cleanCargoSource ./.;
          strictDeps = true;

          buildInputs = [ ];
        };

        annil-server = craneLib.buildPackage (commonArgs // {
          cargoArtifacts = craneLib.buildDepsOnly commonArgs;
        });
      in {
        checks = { inherit annil-server; };

        packages.default = annil-server;

        apps.default = flake-utils.lib.mkApp { drv = annil-server; };

        devShells.default = craneLib.devShell {
          checks = self.checks.${system};

          packages = [ ];
        };
      });
}
