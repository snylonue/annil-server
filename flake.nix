{
  description = "An unofficial annil implementation";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

    crane.url = "github:ipetkov/crane";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    flake-parts.url = "github:hercules-ci/flake-parts";
  };

  nixConfig = {
    extra-substituters = [ "https://annil-server.cachix.org" ];
    extra-trusted-public-keys = [
      "annil-server.cachix.org-1:ioHVMApnJQ8UDnQRzkGR4hDVJ0xTwpphc/6bffyxXXA="
    ];
  };

  outputs =
    {
      self,
      nixpkgs,
      crane,
      flake-parts,
      rust-overlay,
      ...
    }@inputs:
    flake-parts.lib.mkFlake { inherit inputs; } (
      { ... }:
      {
        flake = {
          nixosModules.default =
            {
              config,
              lib,
              pkgs,
              ...
            }:
            {
              options = {
                services.annil-server = {
                  enable = lib.mkOption {
                    type = lib.types.bool;
                    default = false;
                    description = ''
                      Whether to run annil-server.
                    '';
                  };

                  package = lib.mkOption {
                    type = lib.types.package;
                    default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
                  };

                  settings = lib.mkOption {
                    type = lib.types.nullOr (lib.types.attrsOf lib.types.unspecified);
                    default = null;
                  };

                  user = lib.mkOption {
                    type = lib.types.str;
                    default = "annil-server";
                  };

                  group = lib.mkOption {
                    type = lib.types.str;
                    default = "annil-server";
                  };
                };

              };

              config =
                let
                  cfg = config.services.annil-server;
                  settingsFile = (pkgs.formats.toml { }).generate "config.toml" cfg.settings;
                in
                lib.mkIf cfg.enable {
                  assertions = [
                    {
                      assertion = (cfg.settings != null) && (cfg.package != null);
                      message = "`settings` should not be empty";
                    }
                  ];

                  systemd.services.annil-server = {
                    description = "annil-server Daemon";
                    after = [
                      "network.target"
                      "nss-lookup.target"
                    ];
                    wantedBy = [ "multi-user.target" ];
                    serviceConfig = {
                      ExecStart = "${cfg.package}/bin/annil-server --config ${settingsFile}";
                      CapabilityBoundingSet = "";
                      AmbientCapabilities = "";
                      NoNewPrivileges = true;
                      User = cfg.user;
                      Group = cfg.group;
                    };
                  };

                  users = {
                    users = lib.mkIf (cfg.user == "annil-server") {
                      annil-server = {
                        isSystemUser = true;
                        home = "/var/lib/annil-server";
                        group = cfg.group;
                        extraGroups = [ "networkmanager" ];
                      };
                    };
                    groups = lib.mkIf (cfg.group == "annil-server") { annil-server = { }; };
                  };
                };
            };
        };

        systems = [ "x86_64-linux" ];

        perSystem =
          {
            self',
            pkgs,
            system,
            ...
          }:
          let
            craneLib = (crane.mkLib pkgs).overrideToolchain (p: p.rust-bin.nightly.latest.minimal);

            commonArgs = {
              src = craneLib.cleanCargoSource ./.;
              strictDeps = true;

              cargoExtraArgs = "--offline";

              buildInputs = [ ];
            };

            annil-server = craneLib.buildPackage (
              commonArgs
              // {
                cargoArtifacts = craneLib.buildDepsOnly commonArgs;
              }
            );
          in
          {
            _module.args.pkgs = import inputs.nixpkgs {
              inherit system;
              overlays = [ (import rust-overlay) ];
            };

            packages = {
              inherit annil-server;
              default = self'.packages.annil-server;
            };

            checks = { inherit annil-server; };

            devShells.default = craneLib.devShell {
              checks = self'.checks.${system};

              packages = [ ];
            };
          };
      }
    );
}
