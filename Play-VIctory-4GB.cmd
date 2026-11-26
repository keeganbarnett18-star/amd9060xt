@echo off
setlocal EnableExtensions DisableDelayedExpansion
if not defined VICTORY_GAME_DIR set "VICTORY_GAME_DIR=%~dp0..\GTA6.Xbox.v1.0.2.1"
if not exist "%VICTORY_GAME_DIR%\game_scarlett_master.exe" (
  echo Set VICTORY_GAME_DIR to your own dump folder containing game_scarlett_master.exe. 1>&2
  exit /b 2
)
set "runner=%~dp0target\release\victory.exe"
if not exist "%runner%" set "runner=%~dp0victory-3d-rtx-pareto-residency.exe"

set "VICTORY_CACHE_DIR=%LOCALAPPDATA%\VIctory\cache-pareto"
set "VICTORY_AMBIENT_AIR=1.5"
set "VICTORY_AMBIENT=constant:0.8"
set "VICTORY_TEXTURE_NO_WAIT=1"
set "VICTORY_RELAY=2"
set "VICTORY_WIDE_BACK=1"
set "VICTORY_CARD_REHASH_RUNS=1"
set "VICTORY_JOIN_COPIES=1"
set "VICTORY_INDIRECT_WINDOW=1"
set "VICTORY_RELAY_COPY=1"
set "VICTORY_CARD_STEPS=2"
set "VICTORY_INDIRECT_BATCH=2"
set "VICTORY_MAP_THREADS=1"
set "VICTORY_CARD_AWAKE=1"
set "VICTORY_SHOW_AFTER=1"
set "VICTORY_THIN=1"
set "VICTORY_JOIN_PARTS=8"
set "VICTORY_UPLOAD_AHEAD=1"
set "VICTORY_LEVELS_ON_DEMAND=1"

:: 4 GB VRAM tuning: reduce card-buffer thrashing and driver background recompile threads
set "VICTORY_CARD_BUFFERS_MIN_KB=4096"
set "VICTORY_CARD_BUFFERS_MB=256"
set "VICTORY_D3D12_BACKGROUND=0"
if not defined VICTORY_LOG set "VICTORY_LOG=warn"

rem Series X: on an 8 GB card series-s drops the head/skin meshes (faces render
rem as eyes and teeth only), as Play-VIctory.cmd notes.
"%runner%" run "%VICTORY_GAME_DIR%\game_scarlett_master.exe" --console series-x --execute
exit /b %errorlevel%
