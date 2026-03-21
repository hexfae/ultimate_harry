{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    crane.url = "github:ipetkov/crane";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    self,
    nixpkgs,
    flake-parts,
    crane,
    rust-overlay,
  } @ inputs:
    flake-parts.lib.mkFlake {inherit inputs;} {
      systems = ["x86_64-linux" "aarch64-linux"];

      perSystem = {
        config,
        self',
        inputs',
        pkgs,
        system,
        ...
      }: let
        pkgs = import inputs.nixpkgs {
          inherit system;
          overlays = [(import rust-overlay)];
        };

        rustToolchain = pkgs.rust-bin.nightly.latest.default.override {
          extensions = ["rust-src" "rust-analyzer" "rustc-codegen-cranelift-preview"];
        };

        craneLib = (crane.mkLib pkgs).overrideToolchain (p: rustToolchain);

        commonArgs = {
          src = craneLib.cleanCargoSource ./.;
          strictDeps = true;
          buildInputs = [];
          nativeBuildInputs = with pkgs; [rustToolchain clang mold];
        };
      in {
        packages.default = craneLib.buildPackage commonArgs;
        devShells.default = pkgs.mkShell {
          inherit (commonArgs) buildInputs nativeBuildInputs;
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath commonArgs.buildInputs;
        };
      };

      flake = {
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
            token-file = lib.mkOption {
              type = lib.types.path;
              description = "Path to the token file";
            };
          };

          config = lib.mkIf cfg.enable {
            users = {
              groups.harry = {};
              users.harry = {
                isSystemUser = true;
                group = "harry";
              };
            };
            systemd.services.harry = {
              wantedBy = ["multi-user.target"];
              serviceConfig = {
                ExecStart = "${self.packages.${pkgs.stdenv.hostPlatform.system}.default}/bin/harry";
                User = "harry";
                Group = "harry";
                WorkingDirectory = /var/lib/harry;
                StateDirectory = "harry";
                LogsDirectory = "harry";
                Restart = "on-failure";
                Environment = ["TOKEN_FILE=${toString cfg.token-file}"];
              };
            };
          };
        };
      };
    };
}
