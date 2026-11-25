# FBR RAM Downloader NVIDIA

A Game Boy DMG/MGB-styled cracktro that runs a real overclock: it raises the
HBM2 memory clock of every NVIDIA Tesla V100 in a machine, live, with no VBIOS
flash and no reboot. The cracktro is the presentation; the work is done by
[`v100-unlock`](v100-unlock/), a small root tool in this repo. `--demo` runs a
simulated download so the cracktro can be demoed anywhere.

How the cracktro is put together is in [ARCHITECTURE.md](ARCHITECTURE.md).

## How it works

Overclocking is free performance. On a gaming card you raise the core clock,
memory clock, voltage with sliders that talk to Nvidia's driver and tell it to
set the card to whatever you specified. On the V100, Nvidia's datacenter chip,
that door is deliberately shut: the driver exposes only one memory speed,
877 MHz, versus the hundreds on gaming cards. Editing the card's firmware to go
faster doesn't work either — the firmware is signed, and the card rejects
anything without Nvidia's signature.

Yet the same chip runs its memory at 1107 MHz in the V100S. Nvidia locked down
overclocking on the V100 so they could sell you a more expensive, faster SKU
that is just overclocked.

Nvidia left a gap in that lock: the driver can only police requests that go
through the driver. It does not stop you, the user, from writing straight into
the registers and doing whatever you want with them, as long as you have admin
rights on the machine.

The memory clock on a V100 is a number in one of those registers: a 27 MHz
reference clock multiplied by a setting. Nvidia's setting gives you 877.5 MHz.
Change the setting and you change the clock. All it takes is one write to a
multicast address, which hits all 16 of the card's memory controllers at once,
and the memory switches over in about a ten-thousandth of a second — no reboot,
no driver reload.

We didn't come up with this in isolation. People found this vulnerability while
unlocking Nvidia's crypto-mining cards, which Nvidia also deliberately
crippled, and we ported the logic to the V100. The unlock kit for the CMP
100–210 (literally a V100 with a different name) changes the memory clock by
putting the memory to sleep, stopping the clocks, and restarting them; on our
cards that crashed the card or the whole server 5 out of 7 times. The single
write from 170tune (made for a different mining card) worked, because the
multicast handled everything for us.

From there we went up one step at a time, testing the memory for errors at every
step, until 1107 MHz, where it started throwing errors straight away. We ended
up at 1053 MHz: 20% more memory bandwidth for free. The driver doesn't even
know — Nvidia's own tool will report the card running at 1053 MHz while still
saying 877 MHz is the max.

### It checks everything

Writing straight into a chip's registers can go wrong, so the tool checks
everything. After the change it counts the memory's actual clock ticks to make
sure every card really is at the new speed (within 1 MHz). It is all-or-nothing:
if one card is off, something fails, or you quit halfway, every card is put back
to where it started. Nothing is permanent — no firmware is touched, and a
reboot puts it back to stock.

1053 MHz is what held up in our stability testing on these cards; `--sprint`
takes it to 1080 if you want to push it. You can run it as a systemd unit so it
starts on boot.

Keep in mind this is faster memory, not more memory: your 16 (or 32) GB card is
still 16 (or 32) GB. You'll see the speedup in work that is waiting on memory —
running inference on LLMs — not in pure number crunching.

We have tested this on P-series and V-series cards, not above. There is a chance
it can work on A-series cards, or could be ported to; if you figure that out, a
PR to expand the tool would be great.

We are about to announce some more goodies for the non-hyperscale market of
inference enjoyers, and this is our first gift back.

Happy cracking.

— FBR ROM

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

```sh
fbr-ram-downloader --demo          # the simulated download (no hardware)
fbr-ram-downloader                 # the real hack, on this machine
fbr-ram-downloader --host c4130    # the real hack, on the V100 machine c4130, over ssh
fbr-ram-downloader --dry-run       # survey and report, write nothing
fbr-ram-downloader --sprint        # 1080 MHz instead of the daily 1053 MHz
```

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
| `--demo` | The simulated download: no hardware |
| `--host HOST` | Run the real hack on the V100 machine `HOST`, over ssh |
| `--dry-run` | Survey and report, but write nothing |
| `--sprint` | 1080 MHz instead of the daily 1053 MHz |
| `--fast` | Skip the pacing of the status lines |
| `--grey` | Use the Grey Quarttrochrome palette (default: Green) |
| `--dark` / `--light` | Force the terminal theme instead of auto-detecting |
| `--music FILE` | Play a specific track (else auto-detect `music/`) |
| `--no-music` | Start with music off |
| `--dump N [W H]` | Headless: print N frames to stdout |
| `--layout [W H]` | Print the computed layout for a size and exit (debug aid) |

### The `v100-unlock` tool, on its own

On the V100 machine, the agent is a normal CLI:

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

There are two artifacts, and they install differently:

- **The V100 agent** (`v100-unlock`) — a **static** binary that runs as root on
  the V100 machine. It has no system dependencies, so it runs on any Linux
  distro. The V100 machine itself needs no Rust: build the binary anywhere and
  copy it over.
- **The cracktro** (`fbr-ram-downloader`) — the TUI. It needs a Rust toolchain
  and the ALSA development library to build (the audio stack, `rodio` → `cpal`,
  links against ALSA only — not PulseAudio).

### The V100 agent (on the V100 machine, once)

Build the static binary. The repo's `.cargo/config.toml` sets the musl target to
link with `rust-lld` and `+crt-static`, so this one command produces a
self-contained binary — no extra flags:

```sh
rustup target add x86_64-unknown-linux-musl
cargo build -p v100-unlock --release --target x86_64-unknown-linux-musl
```

(`rustup` is the same on every distro; see the table below if you don't have
it. A C compiler is needed for the build scripts — `gcc`/`clang` — but the
result is static and needs none at runtime.)

Copy it to the V100 machine, install it root-owned, and let your user run
exactly that binary as root:

```sh
scp target/x86_64-unknown-linux-musl/release/v100-unlock c4130:
ssh c4130
sudo install -o root -g root -m 755 v100-unlock /usr/local/sbin/v100-unlock
echo "$USER ALL=(root) NOPASSWD: /usr/local/sbin/v100-unlock" | sudo tee /etc/sudoers.d/v100-unlock
sudo chmod 440 /etc/sudoers.d/v100-unlock
```

The cracktro calls `/usr/local/sbin/v100-unlock` by that exact path, and the
sudoers rule names it, so the two have to agree.

### The cracktro

**With Nix (recommended).** The repo ships a [Nix flake](flake.nix) that builds
the cracktro reproducibly — pinned [nixpkgs](flake.lock) and the committed
`Cargo.lock` (exact crate versions + hashes) — so the build is identical on
every machine and the sandboxed build needs no network:

```sh
nix build .             # -> result/bin/fbr-ram-downloader
nix run .               # run it directly
nix profile install .   # install into your nix profile
```

**Without Nix.** Install a Rust toolchain and the ALSA development library, then
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
change, refresh it:

```sh
cargo build --release
cp target/release/fbr-ram-downloader demo/
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
