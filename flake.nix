{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    naersk.url = "github:nix-community/naersk";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = {
    self,
    nixpkgs,
    naersk,
    flake-utils,
    rust-overlay,
  } @ inputs:
    flake-utils.lib.eachDefaultSystem (
      system: let
        overlays = [(import rust-overlay)];
        pkgs = (import nixpkgs) {
          inherit system overlays;
        };
        rust-nightly =
          pkgs.rust-bin.selectLatestNightlyWith
          (toolchain:
            toolchain.default.override {
              extensions = ["rust-src" "rust-analyzer" "rustc-codegen-cranelift-preview"];
            });
        naersk = inputs.naersk.lib.${system}.override {
          cargo = rust-nightly;
          rustc = rust-nightly;
        };
      in {
        defaultPackage = naersk.buildPackage {
          src = ./.;
        };

        devShell = with pkgs;
          mkShell {
            nativeBuildInputs = with pkgs; [
              mold
              clang
              rust-nightly
            ];
          };
      }
    )
    // {
      nixosModules.default = {
        config,
        lib,
        pkgs,
        ...
      }: let
        cfg = config.services.harry;
      in {
        options.services.harry = {
          enable = lib.mkEnableOption "Ultimate Harry";
          configFile = lib.mkOption {
            type = lib.types.path;
            description = "Path to the config file";
          };
        };

        config = lib.mkIf cfg.enable (lib.mkMerge [
          {
            systemd.services.harry = {
              wantedBy = ["multi-user.target"];
              serviceConfig = {
                ExecStart = "${self.defaultPackage."${pkgs.system}"}/bin/harry";
                User = "harry";
                Group = "harry";
                WorkingDirectory = /var/lib/harry;
                StateDirectory = "harry";
                LogsDirectory = "harry";
                Restart = "on-failure";
                Environment = ["CONFIG_FILE=${toString cfg.configFile}"];
              };
            };

            users.users.harry = {
              isSystemUser = true;
              group = "harry";
            };

            users.groups.harry = {};
          }
        ]);
      };
    };
}
