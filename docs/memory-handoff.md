# Handoff: the memory problem (GTA VI on VIctory, RX 9060 XT 16 GB)

Written 2026-11-27 for whoever picks this work up next, and kept up to date as it goes. Nothing from the game itself is ever committed.

## Current state (read this first)

Last updated 2026-11-27, after session play56. The sections further down are older and partly superseded; where they disagree with this one, this one is right. The forward plan is in `F:\gta6xbox\PLAN-unified-memory.md`.

**Commits of this stretch, all on `main` and pushed, newest last:**

| Commit | What | Played? |
|---|---|---|
| eddd5377 | Spare scratch chunks freed only when unused for a look; drain default back to 40 % | play55, play56 |
| 3999fc5a | Texture blocks by size class | play55, play56 (then switched off) |
| faf44fe8 | A PC of 32 GiB or less keeps 384 MiB of spare scratch chunks, not 128 | play56: works (436 upload buffers made in the session, against 22,665 in two minutes of play55) |
| d1b1faab | Size classes off unless `VICTORY_TEXTURE_CLASSES=1` | — |
| d75e7ed2 | Past the card's ceiling a texture that fits a hole in a block is made, and no new block (`VICTORY_TEXTURE_HOLES=0` turns it off) | **not played** |
| 67c08b0a | A texture view whose description does not fit the host texture is refused and logged (guard for the "invalid call" crash) | **not played** |

**Sessions since the table below:**

- **play55** (2 min, log `trace-log-play55.txt`): 5–6 GB RAM free, no paging, still 13–18 fps while the world streamed in. Cause there: spare-chunk cap, fixed by faf44fe8.
- **play56** (2.5 min in the world, then a crash; log `trace-log-play56-crash.txt`, memory `mem-play56.csv`):
  - Crash: `device removed: invalid call (0x887A0001)`, "the host device went while textures picked by index were made ready". Third time this signature is seen (also `trace-log-play43-crash.txt`, `trace-log-test48-debug-crash.txt`). See "Investigations" below.
  - The card's rules deadlocked: 10.4 GB of blocks for 7.1 GB held, card at 99.7 %, nothing made and nothing let go for the last 90 seconds, 7–17 fps. Fixed (unplayed) by d75e7ed2.
  - Kernel memory calls slowed 10–30× in bursts of about 30 s with RAM free. See "Investigations" below.

- **play57** (about 7.5 min in the world, closed by the user, no crash; log `trace-log-play57.txt`, memory `mem-play57.csv`; build with d75e7ed2 and 67c08b0a):
  - No device loss, and the view guard refused nothing (0 warnings). Inconclusive for the crash: it struck at 2.5 min once but not in most sessions.
  - The card stayed at 90–94 % of its budget (13.7–14.3 GB of 15.1), never past the 95 % ceiling, so the holes fix never had to act (0 slots from holes). The deadlock did not recur, and loading/unloading kept moving all session (57,819 let go, 10,574 made again).
  - Gameplay 16–21 fps; 11:44:30–11:47:00 a steady 30 fps with almost no streaming (probably a cutscene or a pause). One slow burst at 11:47:30–11:48:00 (write watch 80 ms), again right as streaming resumed.
  - RAM: 27–30 GB private, 4.3–7 GB free, page file flat.

- **play58 result (done, switch already set back to 1):** the user found driving worse with `VICTORY_TEXTURE_NO_WAIT=0`. The log shows texture refreshes spending 7 → 34 → 93 → 180 ms a frame waiting for the runtime's remapping as driving went on, at 11–14 fps. **Conclusion: the bottleneck while streaming is the runtime remapping title memory (the page pool's views); the kernel reads under `NO_WAIT=1` are the cheaper way around it, not the cause.** Step 2 (pool in place) and anything that remaps less often are what can help driving. The log is `trace-log-play58-nowait0.txt` once the game closes, or `trace-log.txt` if it was not copied.
- The test as it was planned, for reference: `F:\gta6xbox\trace-crash.cmd` set `VICTORY_TEXTURE_NO_WAIT=0` (it was 1). Purpose: driving is close to unplayable while the world streams; in play57 the heavy-streaming moments had 68 textures a frame read through the kernel and 88 ms of frame prep, against 3–7 and 33–40 ms in normal play. Expectation: probably worse. `texture.rs` `no_wait` documents that waiting for the runtime's remap lock gave 0.3–3.7 s frames in runs 99–101. Either result says whether the runtime's remapping during streaming is the bottleneck. **Put it back to 1 after the test.**

- **play59 / play60: step 2, page pool in place (commit 30314070).**
  - Both sessions ran the same build, with similar driving after a fresh restart. The only difference: play59 had `VICTORY_POOL_IN_PLACE=1`, play60 had it off. Logs: `trace-log-play59.txt` and `trace-log.txt` → play60; `mem-play59.csv` and `mem-play60.csv`. Figures below are in-world samples, play59 vs play60:

    | | play59 (pool on) | play60 (pool off) | play57 |
    |---|---|---|---|
    | Median fps | 23.6 | 12.0 | 18.6 |
    | p10 fps | 17.5 | 5.6 | 8.2 |
    | Preparing a frame | 30 ms | 81 ms | 49 ms |
    | Refreshes | 6.1 ms | 23.4 ms | 11.5 ms |
    | Write watch | 2.0 ms | 16.5 ms | 5.9 ms |
    | Private memory | 26.8 GB | 29.9 GB | 28.5 GB |
    | Free RAM at the end | 6.1 GB | 2.2 GB | 5.4 GB |

  - The user found play59 more stable.
  - Settles caused by the new rule: 0. Declared in-place writes: 0 ranges, so the card did not write the placed memory in these runs.
  - **Coverage is small.** 76 buffers/windows placed (332 MB), and 74 let go because the runtime remapped their memory. 1,554 windows were turned down as "not one run of a committed section": they span scattered pool pages. Window copies still came to 2.4 GB.
  - **Cost:** all six pool sections (6,144 MB) are opened as heaps. play59 logged 16 "not enough memory" failures (no shared / upload / read-back buffer; play57/58: 0), and 183,480 slots made from holes (play60: 52,639). The picture still looks soft: on average 30,000 textures picked by index wait (play60: 52,000), and about 800 draws a frame are dropped (play57: 912, play60: 1,176). The drops are not new.
  - My theory during play59 was that the opened sections cause the drops. The A/B showed it wrong for the drops; it may still hold for the 16 memory failures.
  - The frame recorder (`VICTORY_RECORD`) crashes at start: its device hook writes into a read-only page of D3D12Core.dll 1.620 (`trace-log-play59-recorder-crash.txt`). It is off in `trace-crash.cmd`.
  - **Next for step 2:**
    1. Place scattered windows: each view range could be its own placed buffer, or sections opened per range.
    2. Find why opening sections costs budget, and whether opening fewer helps.
    3. Find the cause of the 16 allocation failures.

**PC setup for the tests:** other programs (chat, browser, game launcher, live wallpaper) were closed before sessions; that also raised the card's video budget from 14.58 to 15.24 GB.

**What the next session must show (play at least 5–10 minutes in the open world):**

1. The log line "past the card's ceiling, with room only inside the blocks of textures": slots made from holes should be above 0 once the card is past 95 %, and "that fit none" should stay small.
2. "host copies the title fills: … let go … made again" should keep moving the whole session (in play56 it froze at 11,119).
3. `mem-playNN.csv`: free RAM, `faults_s`, `pagefile_mb`.
4. No crash. If it crashes with the same signature, see "Investigations".

**Corrections to what is written further down:**

- "Block packing never stops" was only part of it. The 75 % drain did cost a lot (1.3 blocks a second emptied and remade), but slow frames also come from upload-buffer churn (fixed) and from the deadlock above.
- "Free RAM decides the frame rate" was too strong. Low RAM makes it worse; play55 and play56 were slow with RAM free.
- `PrefetchVirtualMemory` (step 1 of the plan) has no evidence behind it: the page file did not grow in play55 or play56.

## Investigations

### The "invalid call" crash (done; cause narrowed, not proven)

A subagent read the crash logs and the code. What it verified:

- The three real crashes are `trace-log-test34-windows.txt` (2026-11-26 20:24), `trace-log-play43-crash.txt` and `trace-log-play56-crash.txt`. `trace-log-test48-debug-crash.txt` is **not** a crash: it has no error line and ran to its 900 s limit, and the D3D12 debug layer never loaded there (line 69: `no D3D12 debug layer ... (0x887A002D)`), although `d3d12SDKLayers.dll` sits beside `D3D12Core.dll` in `target-diag\release\D3D12`.
- test34 and play43 are older than the reflection hold (39430372) and than every memory change of this week. None of those is the root cause. Whether they change how often it happens: no evidence either way.
- The view named in the error line ("last: …") is a red herring: it is only the slot being looked at when the device was polled. Slot 10314 of play56 is re-uploaded about every frame.
- All three were first noticed by the buffer-view pass, which polls the device at its start. So the bad call was made during the passes over the heap.
- In-place memory was off in test34 and play43, so it is excluded.

Suspects, in its order:

1. **A texture view whose description does not fit the host texture** (about 40 %). `mirror` → `make_twinned` in `descriptor/host.rs` passed texture SRV/UAV descriptions to the host as the title wrote them, with only the format checked. `mirror` also reads the title's resource pointer with nothing held while the title may release it, so a description can land on a different texture.
2. `UpdateTileMappings` in `tiles.rs` (about 15 %): returns nothing, was active in all three. Against: it runs at submission, not in the pass.
3. Buffer views with formats that lack buffer support. Low: would recur in the same place.

Done about it (commit 67c08b0a, not played): `srv_of` / `uav_of` in `descriptor/host.rs` refuse a texture view whose dimension, levels or layers do not fit the host texture, and log the first 64 with the view words and the texture's description (`a view of a texture whose description does not fit the host texture is not made`).

How to read the next session:

- The warning appears and the device survives → suspect 1 confirmed; then look at why the title's description and the texture disagree (the use-after-free window in `mirror`).
- The device is lost with no such warning → suspect 1 is out; go to `UpdateTileMappings`, and get the debug layer to load (find out why 0x887A002D).
- The warning appears where textures used to draw fine → the check is too strict for something the driver tolerated; the log line says which rule.

### The 30-second slow bursts

A subagent correlated the logs (stopped early for budget). **No single trigger found.** Its scripts are in `F:\gta6xbox\burst-analysis\`.

Measured:

- play56 switched to slow at 11:06:22.0, back to fast at 11:06:55.0, to slow again at 11:07:26.1.
- The time is waiting, not CPU. The submitting thread runs about 23 ms user + 5 ms kernel a frame in both states; kernel-call wall time rises by over 200 ms a frame in the slow state.
- Calls that change page tables (`GetWriteWatch` with reset, `VirtualProtect`) slow 10–30×. The diagnostics address-space walk slows only 2–3×.
- Over 43 logs (4,212 five-second samples, 90 burst onsets): no burst starts when the fast window before it had under about 6 MB/s "unmapped" and under about 4,200 page faults/s. So bursts only start while the title is streaming the world.
- No per-second counter leads a burst.

Ruled out: the memory-map walk as the main trigger, `watch-mem.ps1`, pipeline-library saves, the capture feature (frame 4,500 was never reached), upload-buffer churn, any fixed period, the window/staging sweep, and working-set trims by Windows.

Inferred, not verified: something outside this process's CPU accounting holds the process's working-set lock. Candidates: the title's streaming file reads pinning pages (DirectStorage path, `victory_hle::modules::dstorage`, `VICTORY_DSTORAGE_IO`, not read yet), or the video memory manager.

Next experiment: `VICTORY_TIMELINE=<from s>[+<from s>..],<frames>,<file>` with times inside a burst records waits, who woke them, and long address-space calls. `VICTORY_MEMORY_MAP=0` removes the walk as a perturbation.

## The user's goal and how they work

- Goal: the best picture with steady frames, and little to no texture or lighting bugs. 20 fps is fine if it is steady. No crashes, and no skipped draws or dispatches.
- The user drives the game. Never send keys or move the camera.
- Launch the game only with `Start-Process explorer.exe -ArgumentList 'F:\gta6xbox\play-capture.cmd'`. Never kill it; the user closes it.
- Simulate on the last session's logs before asking the user to play (`F:\gta6xbox\sim-room.ps1`). Don't launch a fix you haven't checked.
- Before every commit run `cargo xtask policy`, then push every kept change to `main` of the user's fork (`notxuesty/amd9060xt`, checked out at `F:\gta6xbox\amd9060xt`). Credit the source of any ported idea in the commit message.
- Update README.md (status, known issues, changes, credits) once a round is tested.
- Next round, after memory is fixed: CPU frame preparation, aiming for a steady 25–30 fps.

## How a session runs

- `play-capture.cmd` → `trace-crash.cmd` runs the diagnostics build (`amd9060xt\target-diag\release\victory.exe`). It logs to `F:\gta6xbox\trace-log.txt`, takes a capture burst every 4,500 frames, and writes the pipeline book.
- Every `VICTORY_*` setting for test runs lives in `F:\gta6xbox\trace-crash.cmd`.
- `F:\gta6xbox\watch-mem.ps1 -Name playNN` writes `mem-playNN.csv` (process private memory, RAM, free memory).
- Logs of past sessions: `trace-log-play50.txt` … `trace-log-play54-live.txt`, plus `mem-play50..54.csv`.
- Look for lines from `victory_gpu::copystat` (blocks drained, host copies moved, let go / made again, short and regrown copies) and the per-frame timings (write watch, copies, refreshes).

## Symptoms, session by session

| Session | What changed | Median fps | Churn (let go / made again) | Packing moves | What the user saw |
|---|---|---|---|---|---|
| play52 | baseline for this round | 18.5 | 103,580 / 26,227 | 2,186 in 13 min | pop-in, some black textures |
| play53 | sweep settle, target headroom, hard ceiling 95 %, sparse drain 75 % | 15 (p10 6.5) | 86,881 / 23,310; 13,184 shorts, 5,093 regrown | 20,100 in 16 min | better textures early; cycling once the open world loads (10:13); one black wall |
| play54 (run A) | short ceiling 98 %, regrow only under 82 %, whole-print budget 8 MB/frame | 5–8 in stretches | — | 4,628 in 5 min | lighting better, no bad textures, fps terrible |

In play54 the write watch took 31–47 ms a frame (3.4 ms in play52). Copies took 33–66 ms and refreshes 30–62 ms. The card sat at about 96 % raw, as Windows counts it.

## Update, 2026-11-27 (later the same morning)

What the logs of play50 to play54 and test47 showed, and what changed (commits eddd5377 and 3999fc5a, pushed, not yet played):

- **Free RAM decides the frame rate.** With 4–5 GB free a frame takes 23 ms to prepare (test47, 30 fps). With 1–3 GB free it takes 40–100 ms. The write watch takes 3 ms against 20–70 ms for the same ~310 askings, and page faults go from 18 to 2,000–12,000 a frame. The page file (F:, NVMe) peaked at 4.1 GB in use.
- **The game holds 29–30 GB private in the open world.** Other programs held 8.2 GB with the game closed. Closing the larger ones frees a few GB.
- **Fixed: spare scratch chunks.** Under "pressure" (3 GiB free or less, which lasts the whole play on this PC) they were all freed every second and made again: 60–100 upload buffers a second. Now only chunks unused for a whole look go (`Spare::unused` in `host.rs`).
- **Fixed: drain default back to 40 %.** At 75 %, 1.3 blocks of 64 MiB a second were drained and made again.
- **New: size classes for texture blocks** (`pool.rs`, `VICTORY_TEXTURE_CLASSES=0` turns it off). A block holds small, medium or large pieces only. The simulation in `tests::classes_leave_less_air` gives 2,586 MB of blocks for 2,051 MB held, against 3,315 MB mixed. The memory line now prints blocks and MB held per class.
- **In-place windows cover only ~370 MB of ~3.8 GB.** The rest is view memory (the page pool), which needs `VICTORY_POOL_IN_PLACE=1`, and that is refused with `VICTORY_LATE_WAIT`. Making those two work together would save about 3.4 GB of RAM; it is the largest RAM saving left.
- **`watch-mem.ps1` now logs `faults_s` and `pagefile_mb`** every 10 s. This PC's performance counters are off (error 0x422), so the card columns stay 0.
- **Not done, in order of promise:**
  1. Check in the next session whether `pagefile_mb` climbs while frames are slow. If so, call `PrefetchVirtualMemory` on a texture's title memory before reading it again.
  2. Pool the "shared buffers" of list uploads (60–100 made a second in every session, good and bad; `HostList::upload` in `host.rs`).
  3. Pool-in-place with late wait.

## What I thought was wrong (before the update above)

1. **Block packing never stops.** Commit c05e6d62 raised `free_blocks`' sparse threshold to 75 % (`sparse_percent()` in `crates/victory-gpu/src/resource.rs`, `VICTORY_DRAIN_PERCENT`). Packing starts above 92 % raw use, and the card never drops below that. So host copies get moved every frame. Each move is a ReadProcessMemory/kernel trip, and those contend with the write watch (GetWriteWatch). That fits the write watch going from 3 ms to 40 ms.
   - Not yet tested: `trace-crash.cmd` now sets `VICTORY_DRAIN_PERCENT=40`, the old behaviour. The user relaunched at 3:32 but closed it after 11 seconds, so nothing was measured.
   - If it helps, change the default in `sparse_percent()` to 40, then commit and push.
   - Better still, only drain while a move actually frees a block, and cap the moves per frame.
2. **The card is always full.** At 96 %, every new texture costs an eviction, and the evicted texture comes back later as pop-in. The pieces involved all fight over the same last few percent:
   - `room()` at 90 %, the hard ceiling at 95 % (`VICTORY_CARD_HARD_PERCENT`) and the short ceiling at 98 %;
   - the target reserve (`VICTORY_TARGETS_WORLD_MB` 1024, `VICTORY_TARGETS_HEADROOM_MB` 512);
   - the idle sweep (`vram_keep_from` in `host.rs`, early band 82 %);
   - regrow at 82 %;
   - 64 MiB texture blocks, which waste room through fragmentation.

   Ideas not yet tried:
   - Smaller blocks (`VICTORY_TEXTURE_BLOCK_MB=32` or `16`). Note that test22-block32 ran before most of this round's changes.
   - Lowering the card's working ceiling, so there is always slack and nothing thrashes.
   - Making eviction prefer textures that haven't been drawn for a long time (true LRU by last frame used) over the current bands.
3. **System RAM is tight too.** `mem-play54.csv` shows the process at 26–29 GB private, out of 31 GB total, with free memory falling to 2.0 GB at times. I first ruled RAM out, but at 2 GB free Windows may be trimming or paging, which would also inflate kernel time. Worth checking: what holds 20 GB of RAM (host copies of textures? staging? the title's own memory?), and whether host copies can be dropped once a texture is on the card.

## Commits from this round (all on main, pushed)

| Commit | Change | Setting |
|---|---|---|
| f007f097 | target reserve: headroom rule | `targets_kept` |
| c05e6d62 | hard ceiling 95 % plus sparse drain 75 % — the drain part is the suspect | |
| 22c987ea | sweep subtracts what it let go in the last second, 256 MB step | `sweep_owed` |
| 6bd9675f | early idle band at 82 % | `vram_keep_from` |
| 6aa4bf1d | short copies ported from ka1qux | |
| 96b7b69e | short-copy ceiling 98 %; whole-texture print budget 8 MB per frame | |
| 6e80b5af | regrow only under 82 % | `room_to_regrow` |
| ef770f44 | Kitty6's aliased-texture recheck | |
| 39430372 | reflection hold in the ambient stand-in | |
| 752acfb0 | ray-tracing node tests all miss | `VICTORY_BVH_MISS=1`, off by default; run B was meant to test it |

## Tools

- `F:\gta6xbox\sim-room.ps1` replays the room rule over a log. Params: `-FloorMB -HeadroomMB -HardPercent`.
- `F:\gta6xbox\burst-sheets.ps1` builds capture contact sheets.
- `victory warm --used` precompiles pipelines.
