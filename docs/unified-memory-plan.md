# Plan: getting closer to the console's one memory (GTA VI on VIctory, RX 9060 XT 16 GB, 32 GB RAM)

Written 2026-11-27 for whichever agent continues. Read [memory-handoff.md](memory-handoff.md) first: it has the user's rules, how sessions are run, and what the logs showed.

## The problem

The console has one 16 GB pool that the CPU and GPU both read. On this PC the same bytes exist more than once:

- Every texture is in the game's RAM (in the console's tiled layout) and again on the card (in the card's layout).
- Most buffers the GPU reads are in the game's RAM and again in an upload copy.

So the game holds 29–30 GB of a 31 GB PC and 14.2 GB of a 14.6 GB video budget at the same time. When free RAM falls under about 3 GB, frames go from ~23 ms to 40–100 ms to prepare. When the card is full, textures are evicted and come back as pop-in.

It cannot be made exact: the card cannot sample the console's texture layout, so a texture always has two forms. But most of the cost can be removed. The steps below are ordered by what I'd do first.

## Already done (commits eddd5377, 3999fc5a; being played as session play55)

- Spare upload chunks are freed only when unused (they were freed and recreated 60–100 times a second while RAM was short).
- Texture blocks are drained under 40 % full again, not 75 %.
- Texture blocks hold one size class each (`VICTORY_TEXTURE_CLASSES=0` turns it off). In simulation: 2.6 GB of blocks for 2 GB of textures, against 3.3 GB.

Check play55 first: `mem-play55.csv` (free RAM, `faults_s`, `pagefile_mb`) and the log line "blocks of textures by the size of their pieces".

## What play55 showed (2 minutes, 90 seconds of them in the world)

- Free RAM stayed at 5–6 GB, the layer never reported memory pressure, and the page file stayed at 378 MB. So nothing was paged.
- The frames were still slow while the world streamed in: 13–18 fps, 35–92 ms to prepare a frame, 30,000–60,000 page faults a second.
- Those faults were new upload buffers being zeroed. The spare scratch chunks were capped at 128 MB on a PC of 32 GB or less, the lists needed up to 294 MB, and the difference was freed and recreated 250–400 times a second. Commit faf44fe8 raises the cap to 384 MiB. Not yet played.
- **So step 1 below has no evidence behind it yet.** Low free RAM made sessions play52–54 worse, but streaming is slow without it too. Do step 1 only if a longer session shows `pagefile_mb` rising during slow stretches.
- The size classes could not be judged in 90 seconds: blocks were 10.1 GB for 8.0 GB held, about as at the same point of earlier sessions. Per class: small 31 blocks 94 % full, medium 97 blocks 78 %, large 27 blocks 73 %. Look again after 10 minutes of play.
- The session was too short for steady-state numbers. The next one should run at least 5–10 minutes in the open world.

## What play56 showed (2.5 minutes in the world, then a crash)

- **The crash** is the known one: `device removed: invalid call (0x887A0001)`, "the host device went while textures picked by index were made ready", no breadcrumbs. Same signature as `trace-log-play43-crash.txt` and `trace-log-test48-debug-crash.txt`. Log: `trace-log-play56-crash.txt`. Not tied to the day's changes, not cleared of them either; size classes are off by default again (d1b1faab).
- **The spare-chunk fix works:** 436 upload buffers in the session against 22,665 in play55's two minutes.
- **The card's rules deadlock. This is the first thing to fix.** Sequence in play56:
  1. World load leaves the card at 95 % of budget as Windows counts it. `resource::room` refuses every new texture there (hard ceiling, c05e6d62); past 98 % `room_for_short` refuses short ones too.
  2. The sweep (`let_go_idle`) counts the free room inside blocks as free, sees 82–87 %, lets go of 1.4 GB and then stops.
  3. No block is under 40 % full, so `free_blocks` drains none, and no block goes back to the card.
  4. End state from 11:06:20 on: 10.4 GB of blocks for 7.1 GB held, card at 99.7 %, nothing made, nothing let go, 7–17 fps.
- **Done in d75e7ed2 (not yet played):** past the ceiling, the texture pass makes a slot's texture if it fits the largest hole (`resource::holes_state`, `resource::hole_for`, `pool::largest_hole`), and makes no block for it (`pool::NoNewBlock`). What fits no hole after all gets a committed texture of its own size. `VICTORY_TEXTURE_HOLES=0` turns it off. Check the new log line "past the card's ceiling, with room only inside the blocks of textures". What it does not do: bring the card's raw use back down. Blocks still only go back when they empty.
- The design it came from, for reference: a texture that fits a hole in an existing block costs the card nothing, so it must be made whatever the raw count says; only *growing a block* must stop at the ceiling. Concretely:
  - `pool::texture` gets a "no new block" state (raw use at or past the hard ceiling). In it the `make` closure returns `None`.
  - `resource.rs` near line 3129 must then return `None` and not fall back to `CreateCommittedResource`, which would grow the card's use anyway.
  - `resource::room` lets the pass run while blocks have room, even past the ceiling.
  - Check what `draw.rs` (`heap_textures`, around lines 2586–3132) does with a texture that could not be made, so it retries later and does not spin every frame.
  - Then try `VICTORY_TEXTURE_CLASSES=1` again: classes only pay off once holes are reused.
- Budget note: with the other programs closed the card's budget rose from 14.58 to 15.24 GB.
- Kernel time still came in bursts of about 30 s (write watch 40–50 ms, 150–200 ms of "watches" a frame) with 4+ GB of RAM free and no paging. During them 33–44 textures a frame were re-read "through the kernel". Not explained yet.

## Step 1: let the RAM copy of a texture go to disk, and bring it back cheaply

Once a texture is on the card, its RAM copy is only read again when the texture is re-uploaded (evicted and wanted again, moved, regrown, or rechecked). Windows already pages such memory out. The cost is reading it back one 4 KiB page at a time, inside whatever thread touches it.

- **1a. Batch the read-back.** Before a texture's title memory is read for an upload (`crates/victory-gpu/src/texture.rs`, the `copy_mapped` / "read through the kernel" path), call `PrefetchVirtualMemory` on its range. One call reads the whole range from the page file in large requests. Do it on the helper threads that already detile, never on the submitting thread.
- **1b. Choose what Windows trims.** After a texture has uploaded and stayed unchanged for some seconds, remove its title pages from the working set ourselves (`VirtualUnlock` on a range that is not locked does this), so Windows trims cold texture bytes and not the hot pages the write watch and the game's own threads need. Skip textures that are rechecked often (aliased ones, `VICTORY_ALIASED_RECHECK_EVERY`).
- **How to verify:** in `mem-playNN.csv`, `pagefile_mb` may rise but `faults_s` and the write-watch milliseconds in the log should fall. Only do 1b if 1a alone is not enough; 1b can backfire if the game itself reads those bytes.

Unverified: I have not proven the slow frames are hard page faults. play55's `pagefile_mb` and `faults_s` are meant to settle that. If the page file does not grow during slow stretches, skip step 1.

## Step 2: stop copying buffers (about 3.4 GB of RAM, and CPU time)

The layer can open the game's memory as a GPU heap so the card reads it in place (`crates/victory-gpu/src/inplace.rs`). Today that covers ~370 MB of ~3.8 GB of "windows of title memory". The rest is turned down because it is *view memory* (the game's page pool, mapped as views of a section).

- `VICTORY_POOL_IN_PLACE=1` already places view memory in its section (`pool_place_for`). It is refused when `VICTORY_LATE_WAIT` is on, because with late wait the layer only knows shader writes by the ranges copies declare, and a window is not known to be read-only when it is made.
- The work: let pool-in-place and late wait coexist. Either place only windows that are provably read-only for the GPU (never the target of a shader write or a copy), or teach the late-wait rules about in-place windows.
- Cost to know: the whole pool section (6.2 GB) is charged to commit when made. The page file is 64 GB here, so that is fine.
- **How to verify:** the log line "title memory in place: N windows M MB" should rise from ~370 MB toward 3 GB, "host copies of title memory" should fall, and private bytes should fall by about the same.

This is the largest RAM saving left, and it also removes refresh copies from every frame, which helps the CPU round that comes next. It is also the riskiest: wrong reads show as flickering geometry. Use the frame replay (`docs/replay.md`) to compare frames before asking the user to play.

## Step 3: waste less of the card

- **3a. Measure the size classes** in play55. If the air inside blocks is still over 1 GB, try `VICTORY_TEXTURE_BLOCK_MB=32`.
- **3b. Move textures on the card, not through RAM.** When a block is drained, the layer re-reads each texture from the game's RAM and uploads it again (`free_blocks` in `resource.rs`). A GPU copy from the old place to the new one (`CopyResource`) would need no RAM read at all.
- **3c. Count what the other ~3 GB of the card is.** Blocks are 9.5 GB, render targets 1.6 GB, card copies of buffers 1.1 GB; about 1.9 GB is unaccounted (pipelines, descriptor heaps, tiles of levels-on-demand textures). Add it to the memory line before cutting anything.
- **3d. Evict by distance, not only by age,** if the title's own streaming hints can be read. Unexplored.

## Step 4: fewer kernel calls per frame (leads into the CPU round)

- "Shared buffers" for list uploads are created and destroyed 60–100 times a second in every session, good or bad (`HostList::upload` in `host.rs`; its comment says to pool them). Pool them like scratch chunks.
- Each creation or release changes the process's address space, and the write watch (`GetWriteWatch`) and guard-page calls wait behind that.

## What hardware would do

- 64 GB of RAM removes the RAM half of the problem outright; nothing in steps 1 and 2 would be needed for frame rate, only for tidiness.
- The 16 GB card is enough if step 3 recovers the air in blocks.

## Rules that still apply

- The user drives the game: no scripted input, launch only with `Start-Process explorer.exe -ArgumentList 'F:\gta6xbox\play-capture.cmd'`, never kill it.
- Verify on logs, tests or replay before asking the user to play.
- `cargo xtask policy` before each commit; push kept changes to `main`; credit sources; update the README after a tested round.
