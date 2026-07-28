@echo off
setlocal EnableExtensions
cd /d "%~dp0"

if defined HLP_VIEWER_TARGET_DIR (
  set "CARGO_TARGET_DIR=%HLP_VIEWER_TARGET_DIR%"
) else if defined LOCALAPPDATA (
  set "CARGO_TARGET_DIR=%LOCALAPPDATA%\hv"
) else (
  set "CARGO_TARGET_DIR=%TEMP%\hv"
)

echo Removing all generated Rust, wxDragon/wxWidgets, and packaged output...
if exist "%CARGO_TARGET_DIR%" rmdir /s /q "%CARGO_TARGET_DIR%"
if exist target rmdir /s /q target
if exist build rmdir /s /q build

echo Full build cache removed.
exit /b 0
