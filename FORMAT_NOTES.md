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

set "EXE_NAME=hlp-viewer.exe"
set "OUTPUT_DIR=build"
set "OUTPUT_EXE=%OUTPUT_DIR%\%EXE_NAME%"
set "SAVED_EXE=%TEMP%\hlp-viewer-clean-%RANDOM%-%RANDOM%.exe"

echo Cleaning generated build files while preserving %EXE_NAME%...

if exist "%OUTPUT_EXE%" (
  copy /y "%OUTPUT_EXE%" "%SAVED_EXE%" >nul
) else if exist "%CARGO_TARGET_DIR%\release\%EXE_NAME%" (
  copy /y "%CARGO_TARGET_DIR%\release\%EXE_NAME%" "%SAVED_EXE%" >nul
) else if exist "target\release\%EXE_NAME%" (
  copy /y "target\release\%EXE_NAME%" "%SAVED_EXE%" >nul
) else (
  echo ERROR: No built %EXE_NAME% was found.
  echo Run build_hlp.bat first, then run this script again.
  exit /b 1
)

if errorlevel 1 (
  echo ERROR: Could not preserve %EXE_NAME% before cleaning.
  exit /b 1
)

if exist "%CARGO_TARGET_DIR%" rmdir /s /q "%CARGO_TARGET_DIR%"
if exist target rmdir /s /q target
if exist "%OUTPUT_DIR%" rmdir /s /q "%OUTPUT_DIR%"

mkdir "%OUTPUT_DIR%"
if errorlevel 1 (
  echo ERROR: Could not recreate %OUTPUT_DIR%.
  del /q "%SAVED_EXE%" >nul 2>nul
  exit /b 1
)

move /y "%SAVED_EXE%" "%OUTPUT_EXE%" >nul
if errorlevel 1 (
  echo ERROR: Could not restore %EXE_NAME%.
  exit /b 1
)

echo Done. Generated build files removed.
echo Preserved executable: %CD%\%OUTPUT_EXE%
exit /b 0
