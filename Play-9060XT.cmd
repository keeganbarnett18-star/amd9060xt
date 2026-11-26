@echo off
setlocal EnableExtensions DisableDelayedExpansion
rem Radeon RX 9060 XT (RDNA 4, 8 or 16 GB) on a 32 GB PC: Play-VIctory.cmd's
rem settings, plus the AMD-side ones. Anything set before this script runs wins.
rem   set VICTORY_GAME_DIR=F:\path\to\your\dump & Play-9060XT.cmd

rem The threaded submission paths. THIN stays off: in the first runs on this
rem card it left the head skin of people undrawn (eyes and teeth only), as
rem Play-VIctory.cmd notes for NVIDIA; with today's build faces drew with it on,
rem but on a 9800X3D it measured slower (36.3 ms a frame against 34.7, +2.4 GB).
rem JOIN_PARTS=8 as the 7900 XTX findings; 0 if the opening video repeats frames.
if not defined VICTORY_THIN set "VICTORY_THIN=0"
if not defined VICTORY_JOIN_PARTS set "VICTORY_JOIN_PARTS=8"

rem The texture write guard on: textures are hashed every eighth frame unless
rem a write was seen, not at every check. Measured on this card over the same
rem stretch of the prologue: 39.3 ms a frame against 46.5 (texture refreshes
rem 2.7 ms against 13.8). Play-VIctory.cmd keeps it off for colour noise on
rem skin seen elsewhere; none showed here. Set it to 0 if skin looks noisy.
if not defined VICTORY_TEXTURE_GUARD set "VICTORY_TEXTURE_GUARD=1"

rem 1080p out. Measured on this card: 1664x936 took the card from 93 % to 61 %
rem busy and the frame stayed at about 35 ms (the preparing thread and the
rem title set it), so a lower size buys nothing here yet.
rem The title's modes: 1280x720 1664x936 1920x1080 2560x1440 3840x2160.
if not defined VICTORY_OUTPUT set "VICTORY_OUTPUT=1920x1080"
rem Lower still if the card is the limit: VICTORY_RENDER_SCALE=0.667 (0.5-1.0).

rem 32 GB of RAM: runs on the 7900 XTX PC used 40-46 GB private, so Windows
rem leans on the page file. Keep a system-managed page file on a fast drive.

call "%~dp0Play-VIctory.cmd" %*
exit /b %errorlevel%
