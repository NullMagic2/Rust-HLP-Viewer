@echo off
setlocal EnableExtensions
cd /d "%~dp0"

rem Keep wxDragon's generated CMake tree off the source path and make the
rem physical cache root deliberately tiny. wxdragon-sys adds several fixed
rem CMake subdirectories of its own, so every character saved here matters.
if defined HLP_VIEWER_TARGET_DIR (
  set "CARGO_TARGET_DIR=%HLP_VIEWER_TARGET_DIR%"
) else if defined LOCALAPPDATA (
  set "CARGO_TARGET_DIR=%LOCALAPPDATA%\hv"
) else (
  set "CARGO_TARGET_DIR=%TEMP%\hv"
)

echo === Rust HLP Viewer 1.0 build ===
echo Cargo/native cache: %CARGO_TARGET_DIR%
echo.

where cargo >nul 2>nul || (
  echo ERROR: cargo was not found. Install Rust with rustup and use the MSVC toolchain.
  exit /b 1
)
where cmake >nul 2>nul || (
  echo ERROR: cmake was not found. wxDragon requires CMake on Windows.
  exit /b 1
)
where ninja >nul 2>nul || (
  echo ERROR: ninja was not found. wxDragon requires Ninja on Windows.
  exit /b 1
)

echo [1/3] Testing the GUI-independent HLP engine...
rem Exclude hlp-viewer here so wxWidgets is not built once for tests and again
rem for release.
cargo test -p hlp
if errorlevel 1 exit /b %errorlevel%

echo [2/3] Building the release viewer...
cargo build --release -p hlp-viewer
if errorlevel 1 exit /b %errorlevel%

echo [3/3] Collecting executable...
if not exist build mkdir build
copy /y "%CARGO_TARGET_DIR%\release\hlp-viewer.exe" "build\hlp-viewer.exe" >nul
if errorlevel 1 exit /b %errorlevel%

echo.
echo Build completed successfully.
echo Viewer: %CD%\build\hlp-viewer.exe
echo Example dump: build\hlp-viewer.exe --dump-file manual.hlp --verbose
exit /b 0
