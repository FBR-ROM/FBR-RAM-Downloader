# FBR RAM Downloader — Nix flake.
#
# Two things live here:
#
#   1. A development environment (`devShells.default`) for building, testing
#      and running the cracktro. Enter it with:
#
#          nix develop            # or: nix develop .#x86_64-linux
#
#      Inside, `cargo build`, `cargo test` and `./demo.sh` all work.
#
#   2. An installable package (`packages.default`) that builds the release
#      binary. Build / run / install it with:
#
#          nix build .            # -> result/bin/fbr-ram-downloader
#          nix run .              # run it directly
#          nix profile install .  # install into your nix profile
#
# The build is reproducible: it uses the committed Cargo.lock (exact crate
# versions + hashes) and fetches the crates in the non-sandboxed fetch phase,
# so the sandboxed build itself needs no network.

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
  in
  {
    # Installable packages: `nix build .`, `nix run .`, `nix profile install .`
    packages = forAllSystems (pkgs: {
      default = package pkgs;
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
