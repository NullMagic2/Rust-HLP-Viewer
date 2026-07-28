@echo off
setlocal EnableExtensions DisableDelayedExpansion

echo WinHlp32 KB917607 extractor v3.3-native
echo.
rem Standalone extractor for Microsoft's Windows 8.1 KB917607 x64 MSU.
rem No PowerShell, .NET compiler, Visual Studio, or SDK is required.
rem It uses Windows expand/extrac32, certutil, and an embedded x64 helper that
rem calls msdelta.dll ApplyDeltaB directly.
rem
rem Usage:
rem   %~nx0 "Windows8.1-KB917607-x64.msu" [output.exe]

if "%~1"=="" goto :usage
if not exist "%~1" (
    echo ERROR: MSU package not found: "%~1"
    exit /b 2
)

set "MSU=%~f1"
if "%~2"=="" (
    set "OUTPUT=%CD%\winhlp32.exe"
) else (
    for %%I in ("%~2") do set "OUTPUT=%%~fI"
)

set "EXPAND=%SystemRoot%\System32\expand.exe"
set "EXTRAC32=%SystemRoot%\System32\extrac32.exe"
set "CERTUTIL=%SystemRoot%\System32\certutil.exe"
set "FINDSTR=%SystemRoot%\System32\findstr.exe"
if not exist "%EXPAND%" (
    echo ERROR: expand.exe was not found at "%EXPAND%".
    exit /b 3
)
if not exist "%CERTUTIL%" (
    echo ERROR: certutil.exe was not found at "%CERTUTIL%".
    exit /b 3
)
if not exist "%FINDSTR%" (
    echo ERROR: findstr.exe was not found at "%FINDSTR%".
    exit /b 3
)
if not exist "%SystemRoot%\System32\msdelta.dll" (
    echo ERROR: msdelta.dll was not found in System32.
    exit /b 3
)

set "TEMPROOT=%TEMP%\hlp-kb917607-%RANDOM%-%RANDOM%-%RANDOM%"
set "OUTER=%TEMPROOT%\outer"
set "PAYLOAD=%TEMPROOT%\payload"
set "HELPER=%TEMPROOT%\ApplyWinHlpDelta.exe"
set "RAW64=%TEMPROOT%\helper-raw.txt"
set "B64=%TEMPROOT%\helper.b64"

md "%OUTER%" 2>nul || goto :fail_temp
md "%PAYLOAD%" 2>nul || goto :fail_temp
for %%D in ("%OUTPUT%") do if not exist "%%~dpD" md "%%~dpD" 2>nul

echo Extracting outer MSU...
"%EXPAND%" "%MSU%" -F:* "%OUTER%" >nul 2>&1
call :locate_inner_cab
if not defined INNERCAB if exist "%EXTRAC32%" (
    echo expand.exe did not expose the payload CAB; retrying with extrac32.exe...
    rd /s /q "%OUTER%" 2>nul
    md "%OUTER%" 2>nul
    "%EXTRAC32%" /Y /E /L "%OUTER%" "%MSU%" >nul 2>&1
    call :locate_inner_cab
)
if not defined INNERCAB (
    echo ERROR: Could not locate the x64 KB917607 payload CAB inside the MSU.
    goto :fail
)

echo Payload CAB: "%INNERCAB%"
echo Extracting WinHlp32 delta blob 42...
rem Extract the one file we actually need first. This avoids depending on how a
rem particular Windows version lays out a full CAB extraction.
"%EXPAND%" "%INNERCAB%" -F:42 "%PAYLOAD%" >nul 2>&1
call :locate_delta

if not defined DELTAFILE (
    echo Direct expand.exe extraction did not produce blob 42; trying full CAB extraction...
    "%EXPAND%" "%INNERCAB%" -F:* "%PAYLOAD%" >nul 2>&1
    call :locate_delta
)

if not defined DELTAFILE if exist "%EXTRAC32%" (
    echo expand.exe still did not expose delta blob 42; retrying with extrac32.exe...
    rd /s /q "%PAYLOAD%" 2>nul
    md "%PAYLOAD%" 2>nul
    "%EXTRAC32%" /Y /E /L "%PAYLOAD%" "%INNERCAB%" >nul 2>&1
    call :locate_delta
)

if not defined DELTAFILE (
    echo ERROR: Could not extract WinHlp32 delta blob 42 from the payload CAB.
    echo.
    echo CAB directory entries matching 42:
    "%EXPAND%" -D "%INNERCAB%" -F:42 2>nul
    echo.
    echo Files actually extracted under the payload directory:
    dir /s /b /a-d "%PAYLOAD%" 2>nul
    goto :fail
)

if not exist "%DELTAFILE%" (
    echo ERROR: Internal locator selected a nonexistent delta path:
    echo        "%DELTAFILE%"
    goto :fail
)

echo Delta blob: "%DELTAFILE%"
echo Observed delta SHA-256:
"%CERTUTIL%" -hashfile "%DELTAFILE%" SHA256
if errorlevel 1 (
    echo ERROR: Could not hash the extracted delta blob.
    goto :fail
)
echo.
rem Do not reject the PA30 stream based on a hard-coded intermediate hash.
rem Different servicing extraction paths can make that intermediate check brittle.
rem The native helper validates the PA30 signature, reconstructs the target, and
rem this batch then enforces the exact Microsoft target size and SHA-256 below.

echo Preparing embedded native msdelta helper...
"%FINDSTR%" /b /c:"::NATIVE64 " "%~f0" > "%RAW64%"
if errorlevel 1 (
    echo ERROR: Embedded native helper data could not be read from this batch file.
    goto :fail
)
>"%B64%" (
    for /f "usebackq tokens=2" %%A in ("%RAW64%") do echo %%A
)
"%CERTUTIL%" -f -decode "%B64%" "%HELPER%" >nul 2>&1
if errorlevel 1 (
    echo ERROR: Could not decode the embedded native helper.
    goto :fail
)
"%CERTUTIL%" -hashfile "%HELPER%" SHA256 | "%FINDSTR%" /i /c:"44cd799cf5cbc7d131bc37f609d2808266038c2f73d7435e392083983cdebdf1" >nul
if errorlevel 1 (
    echo ERROR: Embedded native helper failed its integrity check.
    goto :fail
)

set "HLP_DELTA=%DELTAFILE%"
set "HLP_OUTPUT=%OUTPUT%"
echo Reconstructing winhlp32.exe with msdelta.dll...
"%HELPER%"
set "HELPER_RC=%ERRORLEVEL%"
set "HLP_DELTA="
set "HLP_OUTPUT="
if not "%HELPER_RC%"=="0" (
    echo ERROR: Native msdelta helper failed with exit code %HELPER_RC%.
    if "%HELPER_RC%"=="20" echo        20 = HLP_DELTA or HLP_OUTPUT environment path missing/too long.
    if "%HELPER_RC%"=="21" echo        21 = Could not open the delta file path.
    if "%HELPER_RC%"=="22" echo        22 = Delta file size is invalid.
    if "%HELPER_RC%"=="23" echo        23 = Memory allocation failed.
    if "%HELPER_RC%"=="24" echo        24 = Could not read the complete delta file.
    if "%HELPER_RC%"=="25" echo        25 = Input does not begin with the PA30 delta signature.
    if "%HELPER_RC%"=="26" echo        26 = msdelta.dll ApplyDeltaB rejected the delta.
    if "%HELPER_RC%"=="27" echo        27 = Reconstructed target failed size/MZ/write validation.
    goto :fail
)

if not exist "%OUTPUT%" (
    echo ERROR: The helper returned success but did not create "%OUTPUT%".
    goto :fail
)
for %%I in ("%OUTPUT%") do if not "%%~zI"=="285696" (
    echo ERROR: Reconstructed winhlp32.exe has size %%~zI bytes; expected 285696.
    del /q "%OUTPUT%" 2>nul
    goto :fail
)
"%CERTUTIL%" -hashfile "%OUTPUT%" SHA256 | "%FINDSTR%" /i /c:"8496f19bc1d898180b57aac61326bdfcf5a48f760811283bbd604aa7a2c13f85" >nul
if errorlevel 1 (
    echo ERROR: Reconstructed winhlp32.exe failed the manifest SHA-256 check.
    echo Expected: 8496f19bc1d898180b57aac61326bdfcf5a48f760811283bbd604aa7a2c13f85
    "%CERTUTIL%" -hashfile "%OUTPUT%" SHA256
    del /q "%OUTPUT%" 2>nul
    goto :fail
)

echo.
echo SUCCESS
for %%I in ("%OUTPUT%") do echo File:      "%%~fI"
echo Size:      285696 bytes
echo SHA-256:   8496f19bc1d898180b57aac61326bdfcf5a48f760811283bbd604aa7a2c13f85
rd /s /q "%TEMPROOT%" 2>nul
exit /b 0

:usage
echo Usage:
echo   %~nx0 "Windows8.1-KB917607-x64.msu" [output.exe]
exit /b 1

:fail_temp
echo ERROR: Could not create temporary working directory "%TEMPROOT%".
rd /s /q "%TEMPROOT%" 2>nul
exit /b 4

:fail
echo.
echo Extraction failed.
echo Temporary files were kept for inspection:
echo   "%TEMPROOT%"
exit /b 5

:locate_inner_cab
set "INNERCAB="
for /r "%OUTER%" %%F in (*.cab) do call :consider_cab "%%~fF"
exit /b 0

:consider_cab
set "CANDIDATE=%~1"
for %%N in ("%CANDIDATE%") do set "CABNAME=%%~nxN"
echo(%CABNAME%| "%FINDSTR%" /i /c:"KB917607" >nul || exit /b 0
echo(%CABNAME%| "%FINDSTR%" /i /c:"x64" >nul || exit /b 0
if not defined INNERCAB set "INNERCAB=%CANDIDATE%"
exit /b 0

:locate_delta
set "DELTAFILE="
rem Prefer the normal extraction location.
if exist "%PAYLOAD%\42" set "DELTAFILE=%PAYLOAD%\42"
if defined DELTAFILE exit /b 0
rem Do NOT use: FOR /R ... IN (42). With a literal filename, FOR /R can
rem synthesize candidate paths even when the file does not exist. DIR /S only
rem emits paths for files that are actually present.
for /f "usebackq delims=" %%F in (`dir /s /b /a-d "%PAYLOAD%\42" 2^>nul`) do (
    if not defined DELTAFILE if exist "%%~fF" set "DELTAFILE=%%~fF"
)
exit /b 0

rem ---------------------------------------------------------------------------
rem Embedded x64 native ApplyDeltaB helper. These lines are data, not commands.
rem It imports only KERNEL32.dll and MSDELTA.dll. The batch verifies both the
rem PA30 input signature and the exact reconstructed Microsoft target hash.
rem ---------------------------------------------------------------------------
::NATIVE64 TVp4AAEAAAAEAAAAAAAAAAAAAAAAAAAAQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
::NATIVE64 AAAAeAAAAA4fug4AtAnNIbgBTM0hVGhpcyBwcm9ncmFtIGNhbm5vdCBiZSBydW4gaW4gRE9TIG1v
::NATIVE64 ZGUuJAAAUEUAAGSGAwBRbGRqAAAAAAAAAADwACIACwIOAAAEAAAABgAAAAAAAAAQAAAAEAAAAAAA
::NATIVE64 QAEAAAAAEAAAAAIAAAYAAAAAAAAABgAAAAAAAAAAQAAAAAQAAAAAAAADAGCBAAAQAAAAAAAAEAAA
::NATIVE64 AAAAAAAAEAAAAAAAABAAAAAAAAAAAAAAEAAAAAAAAAAAAAAALCAAADwAAAAAAAAAAAAAAAAwAAAM
::NATIVE64 AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
::NATIVE64 AAAAAAAAAAAAANggAABwAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAALnRleHQAAAAHAwAAABAA
::NATIVE64 AAAEAAAABAAAAAAAAAAAAAAAAAAAIAAAYC5yZGF0YQAAKAIAAAAgAAAABAAAAAgAAAAAAAAAAAAA
::NATIVE64 AAAAAEAAAEAucGRhdGEAAAwAAAAAMAAAAAIAAAAMAAAAAAAAAAAAAAAAAABAAABAAAAAAAAAAAAA
::NATIVE64 AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
::NATIVE64 AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
::NATIVE64 AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
::NATIVE64 AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
::NATIVE64 AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
::NATIVE64 AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
::NATIVE64 AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
::NATIVE64 AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
::NATIVE64 AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAEFW
::NATIVE64 VldVU0iB7JAIAABIjQ0CEAAASIs91RAAAEiNlCSQBAAAQbgAAgAA/9eJxkiNDcwPAABIjZQkkAAA
::NATIVE64 AEG4AAIAAP/XgcYA/v//gf4B/v//cgwFAP7//z0A/v//dwu5FAAAAP8VfxAAAEjHRCQwAAAAAMdE
::NATIVE64 JCiAAAAAx0QkIAMAAABIjYwkkAQAALoAAACAQbgBAAAARTHJ/xVCEAAASYnGSIP4/3ULuRUAAAD/
::NATIVE64 FTYQAABIjVQkSEyJ8f8VOBAAAIXAD5TASIt0JEhIifFIgcH////7SIH5AwAA/A+SwQjBgPkBdRlM
::NATIVE64 ifH/FeoPAAC5FgAAAP8V7w8AAEiLdCRI/xX8DwAASInHSInBMdJJifD/FfMPAABIicNIhcB1FEyJ
::NATIVE64 8f8Vsg8AALkXAAAA/xW3DwAAx0QkRAAAAABIx0QkIAAAAABMjUwkREyJ8UiJ2kGJ8P8Vwg8AAInF
::NATIVE64 TInx/xV3DwAAhe10DTl0JER1B4A7UHQg6zBIifkx0kmJ2P8VkA8AALkYAAAA/xVdDwAAgDtQdRKA
::NATIVE64 ewFBdQyAewIzdQaAewMwdBlIifkx0kmJ2P8VYA8AALkZAAAA/xUtDwAAD1fADylEJGBIx0QkcAAA
::NATIVE64 AAAPKUQkUEiJXCR4SIm0JIAAAADHhCSIAAAAAAAAAEiNVCRgTI1EJHhMjUwkUDHJ/xUwDwAAicZI
::NATIVE64 ifkx0kmJ2P8VAA8AAIX2dQu5GgAAAP8VyQ4AAEiLfCRQvhsAAABIhf8PhK4AAABIgXwkWABcBAAP
::NATIVE64 hZ8AAACAP00PhZYAAACAfwFaD4WMAAAASMdEJDAAAAAAx0QkKIAAAADHRCQgAgAAADH2SI2MJJAA
::NATIVE64 AAC6AAAAQEUxwEUxyf8VWA4AAEiD+P90TUiJw8dEJGAAAAAASMdEJCAAAAAATI1MJGBIicFIifpB
::NATIVE64 uABcBAD/FWcOAACJx0iJ2f8VFA4AAIX/D5TAgXwkYABcBAAPlcEIwYD5AXUFvhsAAABIi0wkUEiF
::NATIVE64 yXQG/xVIDgAAifH/FfANAACQSIHEkAgAAFtdX15BXsPMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzM
::NATIVE64 zMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzM
::NATIVE64 zMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzM
::NATIVE64 zMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzM
::NATIVE64 zMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMxIAEwA
::NATIVE64 UABfAE8AVQBUAFAAVQBUAAAASABMAFAAXwBEAEUATABUAEEAAAAAAGggAAAAAAAAAAAAAPghAADY
::NATIVE64 IAAAwCAAAAAAAAAAAAAABSIAADAhAAAAAAAAAAAAAAAAAAAAAAAAAAAAAEghAAAAAAAAViEAAAAA
::NATIVE64 AABkIQAAAAAAAHIhAAAAAAAAjCEAAAAAAACcIQAAAAAAAK4hAAAAAAAAuiEAAAAAAADGIQAAAAAA
::NATIVE64 ANIhAAAAAAAAAAAAAAAAAADeIQAAAAAAAOwhAAAAAAAAAAAAAAAAAABIIQAAAAAAAFYhAAAAAAAA
::NATIVE64 ZCEAAAAAAAByIQAAAAAAAIwhAAAAAAAAnCEAAAAAAACuIQAAAAAAALohAAAAAAAAxiEAAAAAAADS
::NATIVE64 IQAAAAAAAAAAAAAAAAAA3iEAAAAAAADsIQAAAAAAAAAAAAAAAAAAAABDbG9zZUhhbmRsZQAAAENy
::NATIVE64 ZWF0ZUZpbGVXAAAARXhpdFByb2Nlc3MAAABHZXRFbnZpcm9ubWVudFZhcmlhYmxlVwAAAEdldEZp
::NATIVE64 bGVTaXplRXgAAABHZXRQcm9jZXNzSGVhcAAAAABIZWFwQWxsb2MAAABIZWFwRnJlZQAAAABSZWFk
::NATIVE64 RmlsZQAAAABXcml0ZUZpbGUAAABBcHBseURlbHRhQgAAAERlbHRhRnJlZQBLRVJORUwzMi5kbGwA
::NATIVE64 TVNERUxUQS5kbGwAAAAAAQ0HAA0BEgEGMAVQBHADYALgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
::NATIVE64 AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
::NATIVE64 AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
::NATIVE64 AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
::NATIVE64 AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
::NATIVE64 AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
::NATIVE64 AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
::NATIVE64 AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
::NATIVE64 AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABAAAAcT
::NATIVE64 AAAUIgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
::NATIVE64 AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
::NATIVE64 AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
::NATIVE64 AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
::NATIVE64 AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
::NATIVE64 AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
::NATIVE64 AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
::NATIVE64 AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
::NATIVE64 AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=
