{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    crane.url = "github:ipetkov/crane";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    git-hooks.url = "github:cachix/git-hooks.nix";
    git-hooks.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-parts,
      crane,
      rust-overlay,
      git-hooks,
    }@inputs:
    flake-parts.lib.mkFlake { inherit inputs; } {
      imports = [ inputs.git-hooks.flakeModule ];
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];

      perSystem =
        {
          config,
          system,
          ...
        }:
        let
          pkgs = import inputs.nixpkgs {
            inherit system;
            overlays = [ (import rust-overlay) ];
          };

          rustToolchain = pkgs.rust-bin.nightly.latest.default.override {
            extensions = [
              "rust-src"
              "rust-analyzer"
              "rustc-codegen-cranelift-preview"
            ];
          };

          craneLib = (crane.mkLib pkgs).overrideToolchain (_: rustToolchain);

          commonArgs = {
            src = craneLib.cleanCargoSource ./.;
            strictDeps = true;
            buildInputs = [ ];
            nativeBuildInputs = with pkgs; [
              rustToolchain
              clang
              mold
            ];
          };
        in
        {
          formatter = pkgs.nixfmt-tree.override { nixfmtPackage = pkgs.nixfmt; };

          # the git hooks still run on commit; only the `nix flake check`
          # derivation is off, since clippy cannot reach the network in the
          # sandbox to fetch this crate's git dependencies
          pre-commit.check.enable = false;

          pre-commit.settings.hooks = {
            # the flake's own nightly toolchain, so the hook formats exactly
            # like `cargo fmt` in the devshell rather than with nixpkgs'
            # stable rustfmt, which cannot parse this crate's nightly syntax
            rustfmt = {
              enable = true;
              packageOverrides = {
                cargo = rustToolchain;
                rustfmt = rustToolchain;
              };
              settings.check = true;
            };
            clippy = {
              enable = true;
              packageOverrides = {
                cargo = rustToolchain;
                clippy = rustToolchain;
              };
              settings.denyWarnings = true;
            };
            treefmt = {
              enable = true;
              package = config.formatter;
            };
            statix.enable = true;
            deadnix = {
              enable = true;
              # a flake's outputs function has to name every input, so the
              # unused ones are structural rather than dead code
              entry = "${pkgs.deadnix}/bin/deadnix --no-lambda-pattern-names";
            };
            detect-private-keys.enable = true;
            ripsecrets.enable = true;
            trim-trailing-whitespace.enable = true;
            end-of-file-fixer.enable = true;
            check-added-large-files.enable = true;
            check-executables-have-shebangs.enable = true;
            check-shebang-scripts-are-executable.enable = true;
            check-merge-conflicts.enable = true;
            check-case-conflicts.enable = true;
            commit-msg = {
              enable = true;
              entry = "${pkgs.commitizen}/bin/cz check --commit-msg-file";
              stages = [ "commit-msg" ];
            };
          };

          packages.default = craneLib.buildPackage commonArgs;
          checks.fmt = craneLib.cargoFmt { inherit (commonArgs) src; };
          devShells.default = pkgs.mkShell {
            inherit (commonArgs) buildInputs nativeBuildInputs;
            inputsFrom = [ config.pre-commit.devShell ];
            packages = [ pkgs.cargo-mutants ];
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath commonArgs.buildInputs;
          };
        };

      flake = {
        nixosModules.default =
          {
            config,
            lib,
            pkgs,
            ...
          }:
          let
            cfg = config.services.harry;
          in
          {
            options.services.harry = {
              enable = lib.mkEnableOption "Ultimate Harry";
              token-file = lib.mkOption {
                type = lib.types.path;
                description = "Path to the token file";
              };
            };

            config = lib.mkIf cfg.enable {
              users = {
                groups.harry = { };
                users.harry = {
                  isSystemUser = true;
                  group = "harry";
                };
              };
              systemd.services.harry = {
                wantedBy = [ "multi-user.target" ];
                serviceConfig = {
                  ExecStart = "${self.packages.${pkgs.stdenv.hostPlatform.system}.default}/bin/harry";
                  User = "harry";
                  Group = "harry";
                  WorkingDirectory = /var/lib/harry;
                  StateDirectory = "harry";
                  LogsDirectory = "harry";
                  Restart = "on-failure";
                  Environment = [ "TOKEN_FILE=${toString cfg.token-file}" ];
                };
              };
            };
          };
      };
    };
}
