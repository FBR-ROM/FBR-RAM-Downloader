# FBR RAM Downloader NVIDIA

A hack that runs a real overclock: it raises the
HBM2 memory clock of every NVIDIA Tesla V100 in a machine, live, with no VBIOS
flash and no reboot. `--demo` runs a
simulated download so it can be demoed anywhere.

How the cracktro is put together is in [ARCHITECTURE.md](ARCHITECTURE.md).

## How it works

Nvidia locked overclocking on the V100. The driver exposes one memory speed,
877 MHz, versus the hundreds on gaming cards. The firmware is signed, so
you can't flash a faster one. Yet the same chip runs its memory at 1107 MHz in
the V100S: the faster SKU is just an overclock.

The lock has a gap: the driver only polices requests that go through the
driver. With root on the machine you can write the registers directly. The
memory clock is a number in one of them: a 27 MHz reference multiplied by a
divider. Nvidia's setting gives 877.5 MHz; change the setting and you change
the clock.

One write to a multicast address hits all 16 of the card's memory controllers at
once, and the memory switches over in about a ten-thousandth of a second. No
reboot, no driver reload, no firmware touched.

We ported this logic from the crypto-mining-card unlocks (Nvidia crippled those too).
The CMP 100–210 kit (a V100 under another name) changes the clock by putting
the memory to sleep, stopping the clocks, and restarting them; on our cards that
crashed the card or the host 5 of 7 times. The single write from 170tune (a
different mining card hack) kinda worked, because the multicast handled all 16
controllers. We laddered up one step at a time, memtesting at each...
1107 MHz threw errors immediately. 1053 MHz held: +20% memory bandwidth for
free. The driver doesn't know — `nvidia-smi` reports the card at 1053 MHz while
still saying 877 MHz is the max.

### It checks everything

Writing registers can go wrong, so the tool checks everything. After the change
it counts the memory's actual clock ticks to confirm every card is at the new
speed (within 1 MHz). It is all-or-nothing: if one card is off, something
fails, or you quit halfway, every card is written back to where it started.
Nothing is permanent — a reboot, or `v100-unlock restore`, returns stock.

1053 MHz is what held in stability testing; `--sprint` pushes to 1080. Run it
as a systemd unit to apply on boot. This is faster memory, not more: a 16 (or
32) GB card is still 16 (or 32) GB. The speedup shows up in memory-bound work,
like LLM inference, not in pure number crunching.

Tested on P-series and V-series cards, not above. A-series may work or be
portable; a PR to expand the tool is welcome.

Happy cracking.

<3 FBR ROM

## The numbers

| | memory clock | memory bandwidth |
|---|---:|---:|
| stock | 877.5 MHz | 898 GB/s |
| daily (default) | 1053 MHz | +20% |
| `--sprint` | 1080 MHz | +23% |

The change is volatile: a reboot, or `v100-unlock restore`, returns stock
clocks. It is validated on 4× V100-SXM2-16GB; every Tesla V100 model is
accepted, but 1053 MHz is untested on the others (the FHHL starts at 810 MHz,
so for it the daily clock is +30%).

## Running it

One program, `fbr-ram-downloader`. The same command runs the interactive
cracktro or the plain-text headless run; the flags pick which.

```sh
fbr-ram-downloader                 # the real hack, interactive cracktro
fbr-ram-downloader --demo          # the simulated download, interactive
fbr-ram-downloader --headless      # the real hack, plain text (scripts, systemd)
fbr-ram-downloader --headless --dry-run   # survey, write nothing
fbr-ram-downloader --host c4130    # the real hack, on the V100 machine c4130, over ssh
fbr-ram-downloader --sprint        # 1080 MHz instead of the daily 1053 MHz
```

`--headless` runs the V100 agent and streams its plain text to stdout, returning
the agent's own exit code (0 at target, 2 untouched, 3 put back, 4 unknown) —
no terminal, no cracktro. It is the mode for scripts and a systemd unit.
`--dry-run` is the safe first step: it surveys every card and reports exactly
what a run would do — `877.5 → 1053.0 MHz · would change` per card — and writes
nothing. It opens the cards read-only and takes no lock.

The cracktro runs `v100-unlock apply --events` as root and shows its progress.
Closing the agent's stdin cancels it, and it puts every card back — so quitting
the cracktro, a crash, or a dropped connection can never leave a card
half-changed. If the link just goes silent, the cracktro says so within about
15 seconds; check the cards with `v100-unlock status`.

### Flags

| Flag | Effect |
|------|--------|
| `--demo` | The simulated download: no hardware, no root |
| `--headless` | Run the V100 agent, stream its plain text, no cracktro |
| `--host HOST` | Run the real hack on the V100 machine `HOST`, over ssh |
| `--dry-run` | Survey and report, but write nothing |
| `--sprint` | 1080 MHz instead of the daily 1053 MHz |
| `--fast` | Skip the pacing of the status lines |
| `--grey` | Use the Grey Quarttrochrome palette (default: Green) |
| `--dark` / `--light` | Force the terminal theme instead of auto-detecting |
| `--music FILE` | Play a specific track (else auto-detect `music/`) |
| `--no-music` | Start with music off |
| `--dump N [W H]` | Print N frames to stdout (for verification / GIF) |
| `--layout [W H]` | Print the computed layout for a size and exit (debug aid) |

### The `v100-unlock` agent, on its own

The agent is a normal CLI, installed at `/usr/local/sbin/v100-unlock`:

```sh
v100-unlock status                # every card's clock, measured (read-only)
v100-unlock apply                 # raise to 1053 MHz (+20%)
v100-unlock apply --sprint        # raise to 1080 MHz (+23%)
v100-unlock apply --dry-run       # survey and report, write nothing
v100-unlock restore               # back to stock
v100-unlock apply --demo          # a 4-card simulated run: no hardware, no root
```

`--card BDF` selects a subset (repeatable); `--events` streams machine-readable
progress (what the cracktro reads); `--simulate N` runs N simulated cards.

## Installing

One command. `install.sh` puts `fbr-ram-downloader` on your PATH, installs the
static V100 agent root-owned, and writes the sudoers rule that lets your user
run it as root. Everything it installs is self-contained — the cracktro ships
its own glibc + ALSA (so it runs on any x86-64 Linux regardless of the system's
glibc), and the agent is a static binary with no dependencies. No Rust, no
Nix, no per-distro packages.

```sh
git clone https://github.com/FBR-ROM/FBR-RAM-Downloader
cd FBR-RAM-Downloader
./install.sh                 # installs to /usr/local (uses sudo if needed)
```

Then it is on your PATH:

```sh
fbr-ram-downloader --headless --dry-run   # survey the cards, write nothing
fbr-ram-downloader                        # the real hack, interactive
```

Options:

```sh
./install.sh --prefix /opt   # install to /opt instead of /usr/local
./install.sh --dry-run       # show what would be done, change nothing
./install.sh --no-sudoers    # skip the sudoers rule (manage sudo yourself)
```

The installer is idempotent — re-run it to update. A non-root user without
passwordless sudo can still install the cracktro to a writable prefix
(`./install.sh --prefix ~/.local`); the agent and sudoers rule are skipped with
the exact commands to add them as root.

### With Nix

The repo is a [Nix flake](flake.nix) that builds the cracktro reproducibly
(pinned [nixpkgs](flake.lock) + the committed `Cargo.lock`) and packages the
static agent. Build, run, or install it directly:

```sh
nix build .             # -> result/bin/fbr-ram-downloader
nix build .#agent       # -> result/bin/v100-unlock
nix run .               # run the cracktro directly
nix profile install .   # install the cracktro into your nix profile
```

Or import it from your own flake:

```nix
inputs.fbr.url = "github:FBR-ROM/FBR-RAM-Downloader";

environment.systemPackages = [
  inputs.fbr.packages.${system}.default   # the cracktro
  inputs.fbr.packages.${system}.agent     # the root tool (Linux)
];
```

### Without the installer

The committed `demo/` bundle is the same self-contained files the installer
copies, so it runs straight from the checkout:

```sh
./demo/run.sh            # interactive (press q to quit)
./demo/run.sh --dump 60  # print 60 frames to stdout
```

`run.sh` invokes the bundled glibc loader with `--library-path demo/lib`, so the
binary and its audio stack resolve from the bundle — no system dependencies.

## Building from source

Only needed if you want to change the code. The cracktro links against ALSA
only (the audio stack, `rodio` → `cpal`, not PulseAudio); the agent is static.
Install a Rust toolchain and the ALSA development library, then
`cargo build --release`. The exact packages per distro:

| Distro | Rust | ALSA dev | pkg-config |
|--------|------|----------|------------|
| Debian / Ubuntu | `sudo apt install rustc cargo` | `sudo apt install libasound2-dev` | `sudo apt install pkg-config` |
| Fedora | `sudo dnf install rust cargo` | `sudo dnf install alsa-lib-devel` | `sudo dnf install pkgconf-pkg-config` |
| Arch | `sudo pacman -S rust cargo` | `sudo pacman -S alsa-lib` | `sudo pacman -S pkgconf` |
| openSUSE | `sudo zypper install rust cargo` | `sudo zypper install alsa-oss-devel` | `sudo zypper install pkgconf` |
| Alpine | `sudo apk add rust cargo` | `sudo apk add alsa-lib-dev` | `sudo apk add pkgconf` |
| NixOS | `nix develop` (the flake) | — | — |

Or, the same on every distro, use [rustup](https://rustup.rs) for the toolchain
and the distro's ALSA dev package from the table:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
# + the distro's ALSA dev package from the table above
cargo build --release
```

To rebuild the static agent, the repo's `.cargo/config.toml` sets the musl
target to link with `rust-lld` and `+crt-static`, so this one command produces
a self-contained binary:

```sh
rustup target add x86_64-unknown-linux-musl
cargo build -p v100-unlock --release --target x86_64-unknown-linux-musl
```

The cracktro is a normal dynamically-linked binary; at runtime it needs the ALSA
library present (it is, on any distro that ships a sound system). If there is no
audio device it silently no-ops — the cracktro still runs.

## Development

The [Nix flake](flake.nix) is the dev path: one command gives a shell with
`rustc`, `cargo`, the audio system libraries, `pkg-config`, and Python + Pillow
(for the preview GIF), identical on every machine.

```sh
nix develop            # or: nix develop .#x86_64-linux
```

Inside the shell:

```sh
cargo build --release   # build the cracktro
cargo test --workspace  # run every test (cracktro + v100-unlock)
./demo.sh               # run the cracktro (q to quit)
```

`cargo test --workspace` runs the cracktro's unit tests and the full
`v100-unlock` suite — the all-or-nothing invariants, the write-only register
window, cancellation, the protocol, and a thousands-of-runs chaos soak against
a register-level simulator of the GV100. That suite is the guarantee that the
hardware is never left half-changed; see
[ARCHITECTURE.md §14](ARCHITECTURE.md#14-testing-and-verification).

**No Nix?** The dev shell is just `rustc` + `cargo` + the ALSA dev library +
`pkg-config` (the table above) plus Python 3 with Pillow for `make_gif.py`.

### Rebuilding the demo bundle

`demo/` is committed so the quick start works without a build. After a code
change, refresh both binaries:

```sh
cargo build --release
cp target/release/fbr-ram-downloader demo/
cargo build -p v100-unlock --release --target x86_64-unknown-linux-musl
cp target/x86_64-unknown-linux-musl/release/v100-unlock demo/
```

The shared libraries in `demo/lib/` (glibc loader, `libc`, `libm`, `libgcc_s`,
`libasound`) only need refreshing if the toolchain or audio stack changes.
`demo/run.sh` invokes the bundled loader with `--library-path demo/lib`, so the
binary and its whole audio stack resolve from the bundle — no system
dependencies.

### Updating the toolchain

Bump the `nixpkgs` tag in `flake.nix` (e.g. `nixos-25.11` → `nixos-26.05`) and
re-lock:

```sh
nix flake update nixpkgs
```

## Credits

The V100 memory overclock stands on other people's work:

- [amoghmunikote/cmpunlocker](https://github.com/amoghmunikote/cmpunlocker)
  (GPL-2.0): the base, a runtime unlock of the CMP 170HX through a patched
  driver.
- [asm64-hooligan/cmpunlocker, `mem_overclock`](https://github.com/asm64-hooligan/cmpunlocker/tree/mem_overclock)
  (GPL-2.0): the method, one write of the HBM PLL coefficient to the
  multicast register `0x98BC98` followed by a lock poll, and the memory PLL
  register map, on GA100.
- [cachenetics/170tune](https://github.com/cachenetics/170tune) (MIT): the
  same write from userspace, the all-ones false-lock check, the write-back on
  lock timeout, and the exit-code convention used here. See [NOTICE](NOTICE).
- [duggasco/CMP100-210](https://github.com/duggasco/CMP100-210)
  (PolyForm Noncommercial / CC BY-NC): GV100 facts, the `PMC_BOOT_0` canary,
  never reading `0x98BC98`, and the per-partition PLL layout. No code from it
  is included.
- v100-overclock, this project's own research: found that `0x98BC98` is
  where GV100 devinit writes the HBM clock and that writing it works live;
  showed the CMP kit's switch sequence is unusable on SXM2 (21-35% less
  bandwidth, 5 of 7 attempts crashed the card or host); built the CUDA-free
  clock meter from the DRAM cycle counters; and laddered and qualified 4×
  V100-SXM2-16GB: 1053 MHz daily, 1080 MHz sprint, 1107 MHz the wall, +10.1%
  plain decode and +8-9% speculative decode, byte-identical to stock.

Shoutouts: Helix Collective, Nous Research, Eth Strat, Amp Code, Jensen Huang,
Nvidia, Franck (TP art), Onslaught, Defame, Gina, Ziggy, Rob, Gary, Jeeva, DNV.

## License

See [LICENSE](LICENSE).
