# VIctory for the Radeon RX 9060 XT

A fork of the [VIctory](https://github.com/Marvisak/VIctory) compatibility layer,
which runs an Xbox Series title natively on Windows through Direct3D 12, tuned
and measured on an **AMD Radeon RX 9060 XT (16 GB)** with a **Ryzen 7 9800X3D**
and **32 GB of RAM**. The goal: a steady frame rate with no crashes, good
lighting and models, and less memory than the console's unified memory
otherwise costs on a PC.

This repository holds code only: no game files, assets or executables. You need
your own legal dump of the game.

## Where it stands

Measured on the PC above, 1080p output, Series X profile, a scripted 17-minute
walk and boat ride through the opening hours (warm shader caches):

| | |
|---|---|
| Frame rate | about **29 fps** (34-35 ms a frame) where the world has loaded |
| Slow moments | about 1 in 5 reports over 40 ms; worst hitch 162 ms |
| Game memory | **26-28 GB** (was 30-35 GB before this fork's memory work) |
| Video memory | about 13 GB, kept under the card's budget |
| Crashes | none in 17- and 20-minute unattended runs |

Its first session after an update runs at about 23 fps while it rebuilds its
shader caches; from the second session on it is back to normal.

### Known issues

- **Dips in busy areas and while new areas load.** The processor is the limit
  (about 9 cores' worth of work); new areas also bring shaders seen for the
  first time, which take up to a second each to build. A second visit is
  smoother. On a 32 GB PC, close other programs: the game needs most of the RAM.
- **Textures that flash black, or leaves that go see-through for a frame.** Much
  rarer since the prelude upload fix below; some remain, especially in combat.
- **One rare crash** ("device removed: invalid call") while descriptor views are
  being made, seen twice in long play sessions; its cause is being traced with
  the D3D12 debug layer.
- **Menus are laid out slightly too large** (left edge cut off in Settings), and
  the game runs in a window, not fullscreen.

## Roadmap

The goal is the best of both: a good picture (textures that stay sharp, light
that looks right) and steady, stable play on this PC, with little to no
graphics bugs and no crashes. A steady 20 to 25 fps counts for more than a
higher number that jumps around.

1. **Now: picture and stability.** Textures that no longer go blurry, black or
   stale as a session goes on, lighting without rays that holds steady, the
   interface at the right size; then the title's own ray tracing as an option
   of its own, kept off until runs show it stable.
2. **Next: the processor's share of a frame**, aiming at a steady 25 to 30 fps.
   The card is only about half busy; the processor spends some 33 to 37 ms a
   frame preparing it, most of that keeping the console's shared memory in
   step with the card's (copies, write tracking, page guards). The plan: copy
   only what changed, spread the work over more cores, track changes more
   cheaply.
3. **Then:** stand-in pipelines while new shaders build (less pop-in in places
   not seen before), and frame pacing if it is still uneven.

60 fps in the open world is not in sight with this design: every frame pays
for keeping two memories in step, which the console does not have to do.

## What this fork changes

Each change was measured before it was kept; numbers are in the commit messages
and in `docs/findings.md`.

- **The graphics card reads game memory in place** instead of through copies:
  the program ships with Microsoft's DirectX 12 Agility SDK runtime, which has
  `ID3D12Device13` (the system's own runtime on this PC did not), and
  `VICTORY_INPLACE=1` opens windows of title memory as heaps. About 3 GB less
  RAM, smaller hitches, same frame rate.
- **Texture uploads are recorded right before the list that needs them**
  (ported from Belobug's fork): a texture the game changed no longer shows its
  old or empty contents for a frame.
- **Memory leaks closed**: a table of view shapes that grew by 2,000 entries a
  second (the game leaves junk in padding bytes), and command lists that each
  kept 1 MB or more of the driver's memory while idle.
- **Video memory kept under budget**: texture blocks are packed, and the
  emptiest are drained when the card passes 92 % of its budget (16.2 -> 13.4 GB).
- From fxceless' fork and the forks it gathers: list calls written down and
  translated on threads of our own, submissions prepared and joined on helper
  threads, texture uploads on their own copy queue, shader writes landed beside
  the write guards, and frame record and replay ([docs/replay.md](docs/replay.md)).

## How to run

You need Windows 10/11 x64, a CPU with AVX2, a Direct3D 12 GPU with shader
model 6.6 and mesh shaders, Rust (1.98 or newer) with the MSVC build tools, and
your own dump of the game.

```powershell
git clone https://github.com/notxuesty/amd9060xt.git
cd amd9060xt
cargo build --release --locked -p victory
powershell -ExecutionPolicy Bypass -File scripts\get-agility.ps1
```

`get-agility.ps1` fetches Microsoft's DirectX 12 Agility SDK runtime from
nuget.org, checks its signature, and puts it in `target\release\D3D12`. Without
it the game still runs, but memory is copied instead of read in place.

Then launch with your dump's folder:

```powershell
$env:VICTORY_GAME_DIR = 'D:\Games\MyDump'
.\Play-9060XT.cmd
```

`Play-9060XT.cmd` has this card's settings; `Play-VIctory.cmd` is the general
one (NVIDIA and AMD, 8 GB and up) and `Play-VIctory-4GB.cmd` the low-memory one.
Any `VICTORY_...` setting you set before launching wins; each is described in a
comment where the code reads it. Upstream's own description, commands and
debugging switches: [docs/README-fxceless.md](docs/README-fxceless.md).

## Credits

This is a tree of other people's work with more on top. The history is kept,
and commits taken from elsewhere name their source and author.

- **[VIctory](https://github.com/Marvisak/VIctory)** by Marvisak and its
  contributors: the loader, the runtime services, the Direct3D 12 back end and
  the shader recompiler.
- **[fxceless/victory-fork](https://github.com/fxceless/victory-fork)**: the
  fork this one started from, and the people whose pull requests it carries.
- **[cool_fox](https://github.com/DoubleGhosty/cool_fox)** by DoubleGhosty and
  codec-d (the CGBJ runtime): pareto memory residency, launch profiles,
  renderer ports and the community continuation this fork merged.
- **[Belobug/VIctory-Belobug-AMD](https://github.com/Belobug/VIctory-Belobug-AMD)**
  by Belobug: the texture prelude upload fix ported here, the RX 9070 XT work,
  the ambient light idea, pipeline workers, and work by romainhedouin.
- **[avatarmanz/VIctory-7900XTX](https://github.com/avatarmanz/VIctory-7900XTX)**:
  renderer ports 6 and 7, threaded recording, frame rate unlocking.
- **[DexNom/VIctory](https://github.com/DexNom/VIctory)**: ray tracing fixes,
  wave64 lowering, cube gradients, sparse textures, FidelityFX work.
- **[FQY55kai/ZJOU-VICTORY](https://github.com/FQY55kai/ZJOU-VICTORY)**.
- **[myokex/victory-amd](https://github.com/myokex/victory-amd)**: the guard over
  every view of a page (with Spexivus, joroyt and tightfall), and Joshua Odom's
  reads into title memory.
- **Geckomacabre** and **cxy0072019-sketch**: the texture read-back that makes
  boats float.
- **[XCL](https://github.com/ivall/xcl)** (GPL-2.0-only) was read for how the
  same title behaves under another layer; no code of it is here.
- **Microsoft's [DirectX 12 Agility SDK](https://devblogs.microsoft.com/directx/directx12agility/)**:
  its redistributable runtime is fetched by `scripts\get-agility.ps1` and is
  not part of this repository.
- This fork's changes were made and measured with
  [Claude Code](https://claude.com/claude-code).

## License

GPL-3.0-or-later, as VIctory. See [LICENSE](LICENSE); the one file that is not
covered is named in [docs/asset-policy.md](docs/asset-policy.md).
