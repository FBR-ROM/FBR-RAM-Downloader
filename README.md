# FBR RAM Downloader NVIDIA

A Game Boy DMG/MGB-styled cracktro for the FBR RAM-unlock hack: a warp
starfield, a shimmering border title, three 3D models that cycle as they spin
(the FBR nano logo, the v100, and the P5000), a "DOWNLOADING RAM" loader with a
Windows-7-style shimmer, a straight color-cycled scroller, and a
freeze-then-blow-away ending. The hack is pluggable. The real one raises the
memory clock of NVIDIA Tesla V100 GPUs
([below](#the-real-hack-v100-memory-overclock)); `--demo` runs a simulated
download so the UI can be developed and demoed anywhere.

How it works, and why it is built this way, is documented in
[ARCHITECTURE.md](ARCHITECTURE.md).

## Quick demo (no build required)

`demo/` is a self-contained bundle: the release binary, its entire audio stack
(glibc + ALSA, in `demo/lib/`), and the music track (`demo/music/`). It runs on
any x86-64 Linux machine with **no Rust, Nix, or system audio libraries** —
the bundled glibc loader is invoked directly, so everything resolves from
`demo/lib/`.

```sh
./demo/run.sh            # interactive (press q to quit)
./demo/run.sh --dump 60  # headless: print 60 frames to stdout
```

To rebuild the bundle after a code change, see "Rebuilding the demo bundle"
below.

## Color

Per the FBR brand rules (`FBR-DOCS/04 brand and design/Colour.md`), the design
is dictated by the Game Boy DMG/MGB hardware: **exactly 4 colors, no more, no
less.** Two quarttrochrome palettes:

- **Green** (default): `#F1FFF8, #6AB68A, #034F42, #011923`
- **Grey**: `#F5F5F7, #B2AFB8, #4A4B51, #000000`

Every pixel on screen is one of the 4 exact colors (no interpolation). The
"color cycling" is the classic Game Boy 4-shade shimmer — hard-stepping
through the palette over time.

The cracktro draws **foreground glyphs only** — it never paints a background,
so it works on any terminal, any theme, and any font without assuming or
overriding the terminal's own colors.

Because it is foreground-only, a glyph drawn in a color that matches the
terminal's background would vanish. On startup the program asks the terminal
for its actual background color (OSC 11 query) and remaps any of the 4 shades
that would be invisible against it to the nearest visible one
(`canvas.rs::Palette::remap_for`). This handles mid-tone backgrounds (e.g. a
grey theme) that a simple dark/light split gets wrong. `--dark`/`--light`
force the assumption instead of querying; if the terminal doesn't answer, it
falls back to a set of shades that is visible on both a black and a white
background.

## Development with Nix

The repo ships a [Nix flake](flake.nix) that provides both a development
environment and an installable package, so the build is identical on every
machine. It pins [nixpkgs](flake.lock) to a stable release and uses the
committed `Cargo.lock` (exact crate versions + hashes), so builds are
reproducible and the sandboxed build needs no network.

### Development environment

Enter the dev shell — it provides `rustc`, `cargo`, the audio system
libraries (`alsa-lib`, `libpulseaudio`), `pkg-config`, and Python + Pillow
(for the preview GIF):

```sh
nix develop            # or: nix develop .#x86_64-linux
```

Inside the shell:

```sh
cargo build --release   # build the binary
cargo test              # run the unit tests
./demo.sh               # run the cracktro (q to quit)
```

### Installable package

The flake also builds the release binary as a Nix package:

```sh
nix build .             # -> result/bin/fbr-ram-downloader
nix run .               # run it directly
nix profile install .   # install into your nix profile
```

### Updating the toolchain

Bump the `nixpkgs` tag in `flake.nix` (e.g. `nixos-25.11` → `nixos-26.05`)
and re-lock:

```sh
nix flake update nixpkgs
```

> **No Nix?** The build needs a Rust toolchain plus the `alsa` and
> `pulseaudio` development libraries (the audio stack, `rodio` → `cpal`,
> links against them). The flake is the easiest way to get all of it; without
> Nix, install those dev libraries and a Rust toolchain on your system.

### Rebuilding the demo bundle

`demo/` is committed so the quick demo works without a build. After a code
change, refresh it by rebuilding the release binary and copying it (plus the
bundled libs and music) into `demo/`:

```sh
cargo build --release
cp target/release/fbr-ram-downloader demo/
```

The shared libraries in `demo/lib/` (glibc loader, `libc`, `libm`, `libgcc_s`,
`libasound`) only need refreshing if the toolchain or audio stack changes.
`demo/run.sh` invokes the bundled loader with `--library-path demo/lib`, so the
binary and its whole audio stack resolve from the bundle — no system
dependencies.

## Run

```sh
./demo.sh                 # interactive (press q to quit)
./demo.sh --grey          # Grey Quarttrochrome palette
./demo.sh --dump 360 80 24  # headless: print frames to stdout
```

The UI resizes live with the terminal (btop-style) — drag the window and it
reflows, keeping the 3D model centered between the title and the loader. The
bar fills over ~12 s (stub), then the screen freezes and blows away.

Music plays by default (embedded, via `rodio`); press `m` to mute/unmute.

### Flags

| Flag | Effect |
|------|--------|
| `--grey` | Use the Grey Quarttrochrome palette (default: Green) |
| `--dark` / `--light` | Force the terminal theme instead of auto-detecting |
| `--music FILE` | Play a specific track (else auto-detect `music/`) |
| `--no-music` | Start with music off |
| `--dump N [W H]` | Headless: print N frames to stdout |
| `--layout [W H]` | Print the computed layout for a size and exit (debug aid) |
| `--host HOST` | Run the real hack on the V100 machine `HOST`, over ssh |
| `--sprint` | 1080 MHz instead of the daily 1053 MHz |
| `--fast` | Skip the pacing of the status lines |
| `--demo` | The simulated download: no hardware (what `./demo.sh` and `./demo/run.sh` run) |

### Keys

| Key | Effect |
|-----|--------|
| `q` | Quit |
| `m` | Mute / unmute the music (music is on by default) |
| drag | Grab the model and flick it to spin |

## The real hack: V100 memory overclock

The download is real: it raises the HBM2 memory clock of every NVIDIA Tesla
V100 in a machine, live, with no VBIOS flash and no reboot. The work is done
by [`v100-unlock`](v100-unlock/), a small root tool in this repo. The cracktro
runs it, as root on the V100 machine, and shows its progress.

| | memory clock | memory bandwidth |
|---|---:|---:|
| stock | 877.5 MHz | 898 GB/s |
| daily (default) | 1053 MHz | +20% |
| `--sprint` | 1080 MHz | +23% |

Every card changes or none does. v100-unlock measures each card's real clock
afterwards, and puts every card back if one is off, if anything fails, or if
the cracktro goes away mid-run (quit, crash, a closed ssh connection). If the
link just goes silent, the cracktro says so within about 15 seconds; check the
cards with `v100-unlock status`. The change is volatile: a reboot, or
`v100-unlock restore`, returns stock clocks. It adds bandwidth, not capacity:
there is no hidden memory on these cards.

**Setup on the V100 machine, once.** Build the static binary (no Rust is
needed on the V100 machine; from a Mac add
`RUSTFLAGS="-C linker=rust-lld -C target-feature=+crt-static"`), copy it
over, install it root-owned, and let your user run exactly that binary as
root:

```sh
cargo build -p v100-unlock --release --target x86_64-unknown-linux-musl
sudo install -o root -g root -m 755 v100-unlock /usr/local/sbin/v100-unlock
echo "$USER ALL=(root) NOPASSWD: /usr/local/sbin/v100-unlock" | sudo tee /etc/sudoers.d/v100-unlock
sudo chmod 440 /etc/sudoers.d/v100-unlock
```

**Run.**

```sh
fbr-ram-downloader --host c4130   # from your own machine; the music plays there
fbr-ram-downloader                # on the V100 machine itself
v100-unlock status                # every card's clock, measured (read-only)
v100-unlock restore               # back to stock
```

It writes GPU registers as root. It is validated on 4× V100-SXM2-16GB; every
Tesla V100 model is accepted, but 1053 MHz is untested on the others (the
FHHL starts at 810 MHz, so for it the daily clock is +30%).

## Music

Drop a track in `music/` (mp3/ogg/flac/wav/m4a/opus/aac). It auto-plays,
looping, when you run the program — only inside the program, stopped cleanly
on exit. The first audio file in `music/` (alphabetically) is used;
`--music FILE` picks a specific one. Playback is in-process via `rodio`, so
no external player (mpv/ffplay) is needed and the binary is fully
self-contained. If no audio device is available it silently no-ops (the
cracktro still runs).

## Preview GIF

```sh
./demo.sh --dump 360 80 24 > /tmp/frames.txt
python3 make_gif.py /tmp/frames.txt 2 preview.gif
```

Renders with **M+ Nerd Font Mono** (the "60" weight) and truecolor. (Run the
second line inside the [Nix dev shell](#development-with-nix), which provides
Python + Pillow.)

## Tests

```sh
nix develop --command cargo test
```

(or `cargo test` inside the [Nix dev shell](#development-with-nix).)

The layout and the logo's centering are unit-tested: the model never touches
the border, always stays inside its region, clears the loader, is horizontally
centered and sits below centre by exactly `MODEL_V_BIAS`, and the drawn logo
stays within 0.75 cells of its frame center over a full spin (measured 0.50).

## Layout

```
src/
  main.rs    entry point, event loop, live resize, flags, --dump mode
  scene.rs   3D scene: extruded logo + warp starfield (painter's algorithm)
  tui.rs     the cracktro: starfield, 3D title, logo, bar, marquee, blow-away ending
  canvas.rs  4-color Game Boy palettes, shimmer, truecolor ANSI canvas
  music.rs   optional looping background music (auto-detect music/)
  hack.rs    the pluggable Hack trait + StubHack (simulated)
```

## Plugging in the real hack

The cracktro is only the *presentation layer*. Everything it displays comes
from one small interface, [`Hack`](src/hack.rs), so the whole UI is decoupled
from the real RAM-unlock work. Today it is driven by `StubHack`, which fakes a
12-second download so the visuals can be developed standalone.

### The contract

```rust
pub trait Hack {
    fn start(&mut self) -> Result<(), String>;  // called once, before the loop
    fn poll(&mut self) -> HackState;            // once per frame; MUST be non-blocking
    fn finish(&mut self) -> Result<(), String>; // called once on exit (success, quit or error)
}

pub struct HackState {
    pub progress: f32,          // 0.0 ..= 1.0  -> the loader bar and percentage
    pub status: String,         // status line, e.g. "mapping pages"
    pub done: bool,             // true once finished -> freeze + blow-away ending
    pub error: Option<String>,  // Some(..) -> shown, then the program exits with it
}
```

Rules of the road:

- `poll` is called about 30x/second and **must not block**. Kick off work in
  `start` (or a background thread) and have `poll` only read the latest
  snapshot. The frame loop drives the animation; a blocking `poll` stutters it.
- `progress` is the only thing that moves the bar; `done` is the only thing
  that triggers the celebration and the exit. Nothing else in the UI changes.
- `finish` is always called, even on `q`/quit or error, so it is the place to
  stop threads, free buffers, or unlock anything you locked.

### Worked example: a C-backed hack

1. **Put the C source in the repo**, e.g. `src/hack.c`, exposing a tiny C ABI:

    ```c
    #include <string.h>
    static float g_progress; static int g_done;
    int  hack_start(void) { g_progress = 0.0f; g_done = 0; /* ... */ return 0; }
    float hack_progress(void) { return g_progress; }   /* 0..1 */
    int  hack_done(void) { return g_done; }
    int  hack_finish(void) { /* tear down */ return 0; }
    void hack_status(char *buf, size_t len) { snprintf(buf, len, "mapping pages"); }
    ```

    For a long-running job, have `hack_start` spawn a thread that updates
    `g_progress`/`g_done` and returns immediately.

2. **Compile and link it** with a `build.rs` (add `cc` as a build-dependency in
   `Cargo.toml`):

    ```rust
    // build.rs
    fn main() { cc::Build::new().file("src/hack.c").compile("hack"); }
    ```

    ```toml
    [build-dependencies]
    cc = "1"
    ```

3. **Declare the ABI and implement `Hack`** in `src/hack.rs`:

    ```rust
    #[link(name = "hack")]
    extern "C" {
        fn hack_start() -> i32;
        fn hack_progress() -> f32;
        fn hack_status(buf: *mut u8, len: usize);
        fn hack_done() -> i32;
        fn hack_finish() -> i32;
    }

    pub struct CHack;

    impl Hack for CHack {
        fn start(&mut self) -> Result<(), String> {
            if unsafe { hack_start() } == 0 { Ok(()) } else { Err("start failed".into()) }
        }
        fn poll(&mut self) -> HackState {
            let mut buf = [0u8; 128];
            unsafe { hack_status(buf.as_mut_ptr(), buf.len()) };
            let status = String::from_utf8_lossy(&buf)
                .split('\0').next().unwrap_or("").to_string();
            HackState {
                progress: unsafe { hack_progress() }.clamp(0.0, 1.0),
                status,
                done: unsafe { hack_done() } != 0,
                error: None,
            }
        }
        fn finish(&mut self) -> Result<(), String> {
            unsafe { hack_finish() };
            Ok(())
        }
    }
    ```

4. **Swap it in** in `make_hack()` in `src/main.rs` — the single integration
   point used by both the interactive loop and `--dump`:

    ```rust
    fn make_hack() -> Box<dyn hack::Hack> {
        Box::new(hack::CHack)
    }
    ```

That is the entire integration. The starfield, 3D model, loader, scroller and
blow-away ending are driven purely by `HackState`, so they need no changes.
Use `--dump` to eyeball frames headlessly while wiring things up.

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

## Cracktro notes

- **Starfield**: warp effect — stars fly toward the viewer from center
  (`scene.rs::Starfield`). The brightest stars pop in the lightest shade.
- **3D models**: three embedded meshes are cycled through in order — the FBR
  nano logo, the v100, and the P5000 — swapping to the next one every half-turn
  of the *auto* rotation. The user can grab the model with the mouse and spin it
  freely; that user-driven rotation does not count toward the swap, so grabbing
  it never interrupts the cycle. Each model is rotated + perspective-projected,
  backface-culled, depth-sorted, and flat-shaded, then rasterized to one
  brightness sample per cell, which the TUI draws as a **dithered block**
  (`░▒▓█`): every covered cell gets at least `░`, so the silhouette is always
  solid, and the character density carries the shading (ordered-dithered, 4x4
  Bayer). Color is the mid palette shade, brightening to the lightest for
  highlights. The projection pre-compensates for the ~1:2 cell aspect, so each
  model stays square.
- **Lighting**: a key light that slowly precesses around the model, a rim light
  orbiting the other way, and a gently breathing ambient. The interplay of the
  model's spin with the two moving lights keeps the shading in constant,
  non-repeating motion — this is what reveals the detail in the simplified
  meshes.
- **Centering**: each mesh is translated onto its own centroid before rotating,
  so it spins about its middle and projects symmetrically. Horizontally the
  layout centers it exactly; vertically it is placed deliberately **below**
  centre by `MODEL_V_BIAS` in `tui.rs` (0.5 = dead centre, 1.0 = flush above the
  loader), because the lit top face draws the eye upward — raise the constant to
  move the model further down.
- **Mouse grab + flick**: click and drag the logo to rotate it; release to
  fling it with momentum. It damps back into the auto-spin. (crossterm mouse
  capture; pitch is clamped so it can't be flipped over.)
- **Title**: `CRACKED BY FBR ROM` sits centered in the top border, each
  character color-cycled through the palette (the classic Game Boy shimmer).
  The live terminal size is drawn top-left and the key hints right-aligned,
  each only when it stays clear of the title.
- **Loader**: the progress bar sits just above the scroller with a blank
  breathing row between them. It is shade 2 only, with a shade-1 **dithered
  shimmer** (`█▓▒`) that sweeps the whole bar in complete passes, Windows-7
  style, independent of progress.
- **Scroller**: a straight C64-style scroller along the bottom border,
  chars color-cycled through the palette shades.
- **Hints**: a dim `M MUTE • Q QUIT` sits right-aligned in the top border when
  there is room clear of the title. On terminals below 80x24 the status line
  asks you to `RESIZE FOR BEST FIT`.
- **Responsive layout**: `tui::layout(w, h)` is a closed-form function of the
  terminal size and the single source of truth for every position (see its doc
  comment for the formulas). The model picks the largest size from a fixed
  breakpoint table — `56x28, 48x24, 40x20, 32x16, 24x12, 18x9, 14x7, 10x5`
  cells —
  that fits both the available width and the rows between the title and the
  loader, then is centered with `x = (w - mw) / 2`, `y = (h - mh) / 2` clamped
  into the region. Below the smallest breakpoint the model is omitted rather
  than drawn as a sliver. `--layout W H` prints the result.
- **Resize robustness**: the terminal size is re-read every frame (a cheap
  ioctl) rather than trusted only from `Resize` events, which some terminals
  and multiplexers do not deliver reliably — a stale size would mis-frame the
  whole composition.
- **Containment**: every element is clamped to the inner area; only the frame
  characters ever touch row 0 / row h-1 / column 0 / column w-1.
- **Blow-away ending**: on `done`, the whole screen freezes, then blows
  away — a gust sweeps left to right and every character falls off the
  bottom (`tui.rs::Celebration`), leaving a blinking "RAM UNLOCKED • DONE".
  The celebration runs ~3.5 s, then the program exits.
- **Live resize**: btop-style — the per-frame size poll (above) detects the
  change and reflows the scene/cracktro, and ratatui's `draw` auto-resizes its
  back-buffer to the same size and diffs it, writing only the changed cells.
  So a resize reflows cleanly with no blank flash and no full-screen reprint.
- **Greetings**: edit `GREETINGS` in `src/tui.rs` to change the bottom
  scroller text.
