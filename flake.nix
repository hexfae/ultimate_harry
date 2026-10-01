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

          # the cargo-mutants hook's entry: mutation-tests whatever this commit
          # touches, since the pre-commit hook cannot pass a diff to a tool that
          # wants a file and cannot hand it the staged set otherwise.
          #
          # cargo and cargo-mutants are pinned to store paths rather than left to
          # PATH, because a git hook inherits whatever the committer's shell has
          # and that need not be this flake's toolchain at all
          mutantsStaged = pkgs.writeShellScriptBin "mutants-staged" ''
            set -euo pipefail

            cd "$(git rev-parse --show-toplevel)"

            diff_file=$(mktemp)
            trap 'rm -f "$diff_file"' EXIT

            # what the commit will actually contain, as cargo-mutants reads it
            git diff --cached --unified=0 --no-color -- '*.rs' >"$diff_file"

            if [[ ! -s "$diff_file" ]]; then
              echo "cargo-mutants: no staged Rust changes, nothing to mutation-test"
              exit 0
            fi

            PATH=${
              pkgs.lib.makeBinPath [
                pkgs.cargo-mutants
                rustToolchain
              ]
            }:$PATH \
              cargo mutants --in-diff "$diff_file" "$@"
          '';

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
            # mutation-tests only the code this commit touches. the whole tree is
            # ~1200 mutants, far too slow for a commit hook, while a staged diff
            # is normally a handful. cargo-mutants measures whether the tests
            # assert anything about the changed code, which coverage cannot show:
            # a test that passes against a stubbed-out function still counts as
            # coverage and still lets the bug through.
            #
            # deliberately not `--in-place`. that keeps the warm target dir and
            # cuts a mutant from ~27s to ~4s, but it mutates the working tree,
            # which cargo-mutants documents as unsafe when a run is interrupted.
            # a commit hook is exactly where an interrupt happens, so the safe
            # copy is the right trade for a gate
            cargo-mutants = {
              enable = true;
              # the entry must be the absolute store path, not the bare name.
              # git-hooks.nix resolves `entry` to a store path for the hooks it
              # ships a definition for, but cargo-mutants is not one of them, so
              # a bare name is passed through literally and pre-commit then
              # fails with "Executable `mutants-staged` not found" in a clean
              # hook environment. `package` does not put it on PATH for a
              # hook id git-hooks.nix has no definition for
              package = mutantsStaged;
              entry = "${mutantsStaged}/bin/mutants-staged";
              files = "\\.rs$";
              pass_filenames = false;
              require_serial = true;
            };
            # permutes the interleavings of the stop-token registry's mutex, so
            # the Stop button's cross-task hand-off is checked rather than
            # assumed. release mode because loom replays each schedule many
            # times, and the bin target because the crate has no lib target
            loom = {
              enable = true;
              # the flake's own nightly toolchain, for the same reason the
              # rustfmt and clippy hooks use it: nixpkgs' stable cargo cannot
              # parse this crate's nightly syntax
              package = rustToolchain;
              entry = "cargo test --features loom --profile release --bin harry cancellation";
              files = "^src/cancellation\\.rs$";
              pass_filenames = false;
              require_serial = true;
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
