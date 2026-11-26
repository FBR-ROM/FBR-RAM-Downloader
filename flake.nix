# FBR RAM Downloader — Nix flake.
#
# Three things live here:
#
#   1. A development environment (`devShells.default`) for building, testing
#      and running the cracktro. Enter it with:
#
#          nix develop            # or: nix develop .#x86_64-linux
#
#      Inside, `cargo build`, `cargo test` and `./demo.sh` all work.
#
#   2. Installable packages, importable from any other flake:
#
#          packages.<system>.default   the cracktro (built from source)
#          packages.<system>.agent     the static v100-unlock root tool
#
#      Build / run / install them with:
#
#          nix build .            # -> result/bin/fbr-ram-downloader
#          nix build .#agent      # -> result/bin/v100-unlock
#          nix run .              # run the cracktro directly
#          nix profile install .  # install the cracktro into your nix profile
#
#      Or import this flake:
#
#          inputs.fbr.url = "github:FBR-ROM/FBR-RAM-Downloader";
#          ...
#          environment.systemPackages = [
#            inputs.fbr.packages.${system}.default   # the cracktro
#            inputs.fbr.packages.${system}.agent     # the root tool
#          ];
#
# The cracktro is built from source, reproducibly: it uses the committed
# Cargo.lock (exact crate versions + hashes) and fetches the crates in the
# non-sandboxed fetch phase, so the sandboxed build itself needs no network.
# The agent is a committed static binary (no system dependencies), so its
# package just installs it — it runs on any Linux regardless of glibc.

{
  description = "FBR RAM Downloader — Game Boy cracktro TUI";

  inputs = {
    # Pinned to a stable nixpkgs release so the build is reproducible.
    # Bump the tag to update the toolchain and system libraries.
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
  };

  outputs = { self, nixpkgs }:
  let
    systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];

    # Run a function once per supported system, giving it that system's pkgs.
    forAllSystems = f: nixpkgs.lib.genAttrs systems (system:
      f nixpkgs.legacyPackages.${system});

    # The build only needs the Rust sources and the manifest. `models/` and
    # `music/` are runtime-only (loaded from disk; the meshes are already
    # baked into src/model_data.rs), and `target/` is build output — drop them
    # so the source hash stays small and stable.
    cleanSrc = pkgs: pkgs.lib.cleanSourceWith {
      src = ./.;
      filter = path: _type:
        let
          rel = pkgs.lib.removePrefix (toString ./. + "/") path;
          top = pkgs.lib.head (pkgs.lib.splitString "/" rel);
        in !(pkgs.lib.elem top [ "target" "models" "music" ".git" "preview.gif" ]);
    };

    # The installable release binary.
    package = pkgs: pkgs.rustPlatform.buildRustPackage {
      pname = "fbr-ram-downloader";
      version = "0.1.0";

      src = cleanSrc pkgs;
      # Use the committed lock file: exact crate versions + sha256 hashes.
      cargoLock = { lockFile = ./Cargo.lock; };

      # System libraries the audio stack (rodio -> cpal) links against.
      buildInputs = [ pkgs.alsa-lib pkgs.libpulseaudio ];
      nativeBuildInputs = [ pkgs.pkg-config ];

      # The unit tests are run in the dev shell; keep the package build lean.
      doCheck = false;

      meta = {
        description = "Game Boy DMG/MGB-styled cracktro TUI for the FBR RAM-unlock hack";
        license = pkgs.lib.licenses.unlicense;
        platforms = systems;
        mainProgram = "fbr-ram-downloader";
      };
    };

    # The static V100 agent. It is committed (demo/v100-unlock), built as a
    # musl static binary with no system dependencies, so this package just
    # installs it — it runs on any Linux regardless of the system glibc.
    # A plain runCommand (not a stdenv derivation) avoids the fetch/unpack
    # machinery that a single file trips over.
    agent = pkgs: pkgs.runCommand "v100-unlock-0.1.0" {
      preferLocalBuild = true;
      meta = {
        description = "Static root tool that raises the HBM memory clock of every Tesla V100";
        license = pkgs.lib.licenses.unlicense;
        platforms = [ "x86_64-linux" "aarch64-linux" ];
        mainProgram = "v100-unlock";
      };
    } ''
      mkdir -p $out/bin
      install -m 755 ${./demo/v100-unlock} $out/bin/v100-unlock
    '';
  in
  {
    # Installable packages: `nix build .`, `nix run .`, `nix profile install .`
    # The agent is a Linux-only static binary, so it is only offered on Linux.
    packages = forAllSystems (pkgs:
      {
        default = package pkgs;
      }
      // pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
        agent = agent pkgs;
      });
    defaultPackage = nixpkgs.lib.genAttrs systems (system:
      self.packages.${system}.default);

    # Development environment: `nix develop`
    devShells = forAllSystems (pkgs: {
      default = pkgs.mkShell {
        packages = [
          # Rust toolchain (same versions the package build uses).
          pkgs.rustc
          pkgs.cargo
          # System libraries the audio stack links against (dev files + .pc).
          pkgs.alsa-lib
          pkgs.libpulseaudio
          pkgs.pkg-config
          # make_gif.py (preview GIF) needs python3 + Pillow.
          pkgs.python3
          pkgs.python3Packages.pillow
        ];
        shellHook = ''
          echo "FBR RAM Downloader dev shell"
          echo "  cargo build --release   # build the binary"
          echo "  cargo test              # run the unit tests"
          echo "  ./demo.sh               # run the cracktro (q to quit)"
        '';
      };
    });
  };
}
