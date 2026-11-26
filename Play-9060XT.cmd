@echo off
setlocal EnableExtensions DisableDelayedExpansion
rem Radeon RX 9060 XT (RDNA 4, 8 or 16 GB) on a 32 GB PC: Play-VIctory.cmd's
rem settings, plus the AMD-side ones. Anything set before this script runs wins.
rem   set VICTORY_GAME_DIR=F:\path\to\your\dump & Play-9060XT.cmd

rem The threaded submission paths: Play-VIctory.cmd keeps them off for NVIDIA
rem only (faces, repeated video frames); the 7900 XTX findings were taken with
rem them on. Set either to 0 if faces or the opening video look wrong.
if not defined VICTORY_THIN set "VICTORY_THIN=1"
if not defined VICTORY_JOIN_PARTS set "VICTORY_JOIN_PARTS=8"

rem 1080p out: after the preparing thread the card is the next wall
rem (docs/findings.md), and this card has about half a 7900 XTX's shaders.
rem The title's modes: 1280x720 1664x936 1920x1080 2560x1440 3840x2160.
if not defined VICTORY_OUTPUT set "VICTORY_OUTPUT=1920x1080"
rem Lower still if the card is the limit: VICTORY_RENDER_SCALE=0.667 (0.5-1.0).

rem 32 GB of RAM: runs on the 7900 XTX PC used 40-46 GB private, so Windows
rem leans on the page file. Keep a system-managed page file on a fast drive.

call "%~dp0Play-VIctory.cmd" %*
exit /b %errorlevel%
