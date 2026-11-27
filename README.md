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

Played by hand into the open world (sessions play52 to play54, 27 November
2026), the picture is better than above but the frame rate is not yet steady:

| Session | Median fps | What it showed |
|---|---|---|
| play52 | 18.5 | pop-in, some black textures |
| play53 | 15 (1 in 10 frames under 6.5) | sharper textures early; once the open world loads, textures cycle between their coarse and full copies, one black wall |
| play54 | 5 to 8 in stretches | lighting better, no bad textures, frames slow |

### Known issues

- **Video memory runs full in the open world (being worked on now).** The card
  sits at about 96 % of its budget, so every new texture pushes another out,
  which comes back later as pop-in. In play53 and play54 the packing of texture
  blocks also never stopped (20,100 copies moved in 16 minutes against 2,186
  in play52). Each move goes through the kernel, and the write tracking of the
  console's shared memory went from about 3 ms to 30-47 ms a frame. Blocks
  are drained under 40 % full again. In session play56 the card's rules then
  stopped each other: at 95 % of the budget no new texture is made, the sweep
  counts the room inside blocks (3.3 GB there) as free and lets nothing go,
  and nothing new loads or unloads (7 to 17 fps). Past the ceiling a
  texture that fits a hole in a block is now made, and no new block
  (`VICTORY_TEXTURE_HOLES=0`: as before); it awaits a play session.
- **System memory is tight on 32 GB, and the frame rate follows it.** The
  console has one memory; here a texture lies in the game's memory and again
  on the card. The game holds 29-30 GB in the open world. With 4-5 GB of the
  PC's memory free a frame takes 23 ms to prepare; with 1-3 GB free, 40-100
  ms, as Windows moves the game's memory to the page file and back. Close
  other programs before playing (a browser, Discord, Steam and a live
  wallpaper held some 4 GB here), and keep the page file on an SSD.
- **Dips in busy areas and while new areas load.** The processor is the limit
  (about 9 cores' worth of work); new areas also bring shaders seen for the
  first time, which take up to a second each to build. A second visit is
  smoother.
- **Textures that flash black, or leaves that go see-through for a frame.** Much
  rarer since the prelude upload fix and the short copies below; some remain
  while new areas load.
- **Ray tracing is off.** The title's own structure builder is not run, so its
  ray programs (about 400 dispatches a frame) and the builder's stages (about
  570) are left out, and a stand-in light takes their place. The ray tracing
  build comes later as an option of its own.
- **One crash** ("device removed: invalid call") while descriptor views are
  being made, seen three times now, the last after two and a half minutes in
  the open world (play56); its cause is not found yet.
- The game runs in a window, not fullscreen.

## Roadmap

The goal is the best of both: a good picture (textures that stay sharp, light
that looks right) and steady, stable play on this PC, with little to no
graphics bugs and no crashes. A steady 20 to 25 fps counts for more than a
higher number that jumps around.

1. **Now: video memory, then picture and stability.** The card's memory kept
   with room to spare in the open world, so textures no longer go blurry, black
   or stale as a session goes on and frames stay even; lighting without rays
   that holds steady (done: the stand-in light with a bounded haze and held
   reflections); the interface at the right size (done).
2. **Later: the title's own ray tracing** as a launcher of its own, kept off
   until runs show it stable. The pieces are already in, each behind its own
   switch and off by default (see `docs/` and the commit messages).
3. **Next: the processor's share of a frame**, aiming at a steady 25 to 30 fps.
   The card is only about half busy; the processor spends some 33 to 37 ms a
   frame preparing it, most of that keeping the console's shared memory in
   step with the card's (copies, write tracking, page guards). The plan: copy
   only what changed, spread the work over more cores, track changes more
   cheaply.
4. **Then:** stand-in pipelines while new shaders build (less pop-in in places
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
- **Textures with no room are made short** (without their largest levels)
  instead of not at all, so they show coarse rather than black, and are made
  whole again only when the card has room to spare (under 82 %); short copies
  may use the card up to 98 %, everything else stops at 95 %.
- **The card's room kept during long play**: an early idle band from 82 %, a
  sweep that counts what it let go in the last second and lets go of 256 MB at
  most a look, and a fixed headroom for render targets once the world is in.
- **Less churn when the PC's memory is short**: spare upload chunks are freed
  only when unused, and a PC of 32 GB keeps 384 MB of them (they were freed
  and made again 60 to 400 times a second while the world streamed: 436
  upload buffers in session play56 against 22,665 in two minutes before).
  Texture blocks of one size class each are there as an option
  (`VICTORY_TEXTURE_CLASSES=1`; 2.6 GB of blocks for 2 GB of textures against
  3.3 GB in a simulation), off until a session shows them stable.
- **Textures painted through another view of their pages** are read whole and
  looked at again every 30 frames (from Kitty6), within a budget of 8 MB a
  frame.
- **Lighting without ray tracing**: a constant stand-in light with the haze
  bounded, and reflections held between a floor and a cap so water and glass
  no longer go dark or blow out.
- **The interface drawn at 4K and scaled**, so menus fit at 1080p.
- **Fixes for AMD's driver and compiler**: the pipeline library is not mapped
  when it ends within a dword of a page's end, and shader size queries are not
  reused (AMD's compiler crashed on 29 compute pipelines on RDNA4).
- **Ray tracing pieces, off by default** for the later ray tracing build: the
  structure builder's fixes and copies, the renderer's RT port, a stand-in
  light for ray-traced scenes, and `VICTORY_BVH_MISS=1`, which makes every ray
  miss instead of reading unbuilt memory.
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
  beloooo's reflection hold and alexdeliu's size-query fix are from there too.
- **[avatarmanz/VIctory-7900XTX](https://github.com/avatarmanz/VIctory-7900XTX)**:
  renderer ports 6, 7 and RT, threaded recording, frame rate unlocking, the
  4K interface plane, the indirect light band for ray tracing, the bound on
  pages taken from faults.
- **[ka1qux/BRVIctory-ka1qux](https://github.com/ka1qux/BRVIctory-ka1qux)**:
  short textures when the card has no room.
- **[kitty-core/Kitty6](https://github.com/kitty-core/Kitty6)**: whole reads
  and rechecks of textures painted through another view.
- **[TaynanGT/Vice-Tay](https://github.com/TaynanGT/Vice-Tay)**: the interface
  plane at 1080p, as cherry-picked here.
- **[notquiteog/victory-nvidia](https://github.com/notquiteog/victory-nvidia)**:
  the structure builder fixes (sizes bound at once, pipelines queued early,
  memory pushed as written).
- **[xeltax/vicky](https://github.com/xeltax/vicky)**: the structure builder's
  copies.
- **[halfwave50/VIctory-macOS-halfwave50](https://github.com/halfwave50/VIctory-macOS-halfwave50)**:
  the reflection floor, and the rays-miss idea as taken from
  **trustno1-coder**'s VIctory-NVIDIA.
- Ideas, not code: **shadPS4**, **yuzu** and **Xenia**'s texture caches (a cap
  on what one look lets go of, expected and critical memory levels).
- **[DexNom/VIctory](https://github.com/DexNom/VIctory)**: ray tracing fixes,
  wave64 lowering, cube gradients, sparse textures, FidelityFX work.
- **[FQY55kai/ZJOU-VICTORY](https://github.com/FQY55kai/ZJOU-VICTORY)**.
- **[myokex/victory-amd](https://github.com/myokex/victory-amd)**: the guard over
  every view of a page (with Spexivus, joroyt and tightfall), and Joshua Odom's
  reads into title memory.
- **Geckomacabre** and **cxy0072019-sketch**: the texture read-back that makes
  boats float.
- **[XCL](https://github.com/ivall/xcl)** (GPL-2.0-only) was read for how the
  same title behaves under another layer, and found the pipeline library
  read past its page; no code of it is here.
- **Microsoft's [DirectX 12 Agility SDK](https://devblogs.microsoft.com/directx/directx12agility/)**:
  its redistributable runtime is fetched by `scripts\get-agility.ps1` and is
  not part of this repository.
- This fork's changes were made and measured with
  [Claude Code](https://claude.com/claude-code).

## License

GPL-3.0-or-later, as VIctory. See [LICENSE](LICENSE); the one file that is not
covered is named in [docs/asset-policy.md](docs/asset-policy.md).
