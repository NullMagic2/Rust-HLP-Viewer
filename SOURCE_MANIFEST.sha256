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

echo Removing packaged application output while preserving the native build cache...
if exist build rmdir /s /q build

echo Done. Cache preserved at:
echo   %CARGO_TARGET_DIR%
echo Use clean_all.bat to discard it.
exit /b 0
