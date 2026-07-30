[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$MsuPath,

    [Parameter(Mandatory = $false)]
    [ValidateNotNullOrEmpty()]
    [string]$OutputPath = (Join-Path (Get-Location) 'winhlp32.exe')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Invoke-ExpandArchiveFile {
    param(
        [Parameter(Mandatory = $true)][string]$Archive,
        [Parameter(Mandatory = $true)][string]$Destination
    )

    New-Item -ItemType Directory -Force -Path $Destination | Out-Null
    $expandExe = Join-Path $env:SystemRoot 'System32\expand.exe'
    if (-not (Test-Path -LiteralPath $expandExe)) {
        throw "Windows expand.exe was not found at '$expandExe'."
    }

    & $expandExe '-F:*' $Archive $Destination | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "expand.exe failed for '$Archive' with exit code $LASTEXITCODE."
    }
}

if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw 'This utility requires Windows because it calls the Windows msdelta.dll API.'
}

$resolvedMsu = (Resolve-Path -LiteralPath $MsuPath).Path
if (-not $resolvedMsu.EndsWith('.msu', [StringComparison]::OrdinalIgnoreCase)) {
    throw "Expected an .msu package, got '$resolvedMsu'."
}

$resolvedOutput = [IO.Path]::GetFullPath($OutputPath)
$outputDirectory = Split-Path -Parent $resolvedOutput
if ($outputDirectory) {
    New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
}

$tempRoot = Join-Path ([IO.Path]::GetTempPath()) ('hlp-kb917607-' + [Guid]::NewGuid().ToString('N'))
$outerDir = Join-Path $tempRoot 'outer'
$payloadDir = Join-Path $tempRoot 'payload'

try {
    Invoke-ExpandArchiveFile -Archive $resolvedMsu -Destination $outerDir

    $innerCab = Get-ChildItem -LiteralPath $outerDir -File -Filter '*.cab' |
        Where-Object { $_.Name -match 'KB917607' -and $_.Name -match 'x64' } |
        Select-Object -First 1
    if (-not $innerCab) {
        throw 'Could not locate the x64 KB917607 payload CAB inside the MSU.'
    }

    Invoke-ExpandArchiveFile -Archive $innerCab.FullName -Destination $payloadDir

    $manifestPath = Join-Path $payloadDir '_manifest_.cix.xml'
    if (-not (Test-Path -LiteralPath $manifestPath)) {
        throw 'The expanded payload does not contain _manifest_.cix.xml.'
    }

    [xml]$manifest = Get-Content -LiteralPath $manifestPath -Raw
    $target = @($manifest.SelectNodes("//*[local-name()='File']") | Where-Object {
        $name = $_.GetAttribute('name')
        $name -match '^amd64_microsoft-windows-winhstb_[^\\]+\\winhlp32\.exe$'
    }) | Select-Object -First 1

    if (-not $target) {
        throw 'Could not locate the amd64 winhlp32.exe target in the update manifest.'
    }

    $targetHashNode = $target.SelectSingleNode("./*[local-name()='Hash']")
    $deltaNode = $target.SelectSingleNode("./*[local-name()='Delta']")
    $sourceNode = $deltaNode.SelectSingleNode("./*[local-name()='Source']")
    $sourceHashNode = $sourceNode.SelectSingleNode("./*[local-name()='Hash']")
    $basisNode = $deltaNode.SelectSingleNode("./*[local-name()='Basis']")
    if (-not $targetHashNode -or -not $sourceNode -or -not $sourceHashNode) {
        throw 'The winhlp32.exe manifest entry is missing its target hash, delta source, or delta hash.'
    }
    if ($sourceNode.GetAttribute('type') -ne 'PA30') {
        throw "Expected a PA30 delta source, got '$($sourceNode.GetAttribute('type'))'."
    }
    if ($basisNode) {
        throw 'This script expects the KB917607 x64 winhlp32.exe delta to have no Basis file.'
    }

    $deltaName = $sourceNode.GetAttribute('name')
    $deltaPath = Join-Path $payloadDir $deltaName
    if (-not (Test-Path -LiteralPath $deltaPath)) {
        throw "The PA30 delta blob '$deltaName' is missing from the payload."
    }

    $expectedLength = [UInt64]::Parse($target.GetAttribute('length'), [Globalization.CultureInfo]::InvariantCulture)
    $expectedHash = $targetHashNode.GetAttribute('value').ToLowerInvariant()
    $expectedDeltaHash = $sourceHashNode.GetAttribute('value').ToLowerInvariant()
    $actualDeltaHash = (Get-FileHash -LiteralPath $deltaPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualDeltaHash -ne $expectedDeltaHash) {
        throw "PA30 delta SHA-256 is $actualDeltaHash, expected $expectedDeltaHash."
    }
    $deltaBytes = [IO.File]::ReadAllBytes($deltaPath)

    if (-not ('WinHelpReference.MsDelta' -as [type])) {
        Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;

namespace WinHelpReference
{
    [StructLayout(LayoutKind.Sequential)]
    public struct DeltaInput
    {
        public IntPtr lpStart;
        public UIntPtr uSize;
        [MarshalAs(UnmanagedType.Bool)]
        public bool Editable;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct DeltaOutput
    {
        public IntPtr lpStart;
        public UIntPtr uSize;
    }

    public static class MsDelta
    {
        [DllImport("msdelta.dll", SetLastError = true, CallingConvention = CallingConvention.Winapi)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool ApplyDeltaB(
            long ApplyFlags,
            DeltaInput Source,
            DeltaInput Delta,
            out DeltaOutput Target);

        [DllImport("msdelta.dll", SetLastError = true, CallingConvention = CallingConvention.Winapi)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool DeltaFree(IntPtr Memory);

        public static byte[] ApplyFromEmpty(byte[] delta)
        {
            if (delta == null) throw new ArgumentNullException(nameof(delta));

            IntPtr deltaMemory = IntPtr.Zero;
            DeltaOutput output = default(DeltaOutput);
            try
            {
                deltaMemory = Marshal.AllocHGlobal(delta.Length);
                Marshal.Copy(delta, 0, deltaMemory, delta.Length);

                var sourceInput = new DeltaInput
                {
                    lpStart = IntPtr.Zero,
                    uSize = UIntPtr.Zero,
                    Editable = false
                };
                var deltaInput = new DeltaInput
                {
                    lpStart = deltaMemory,
                    uSize = new UIntPtr((uint)delta.Length),
                    Editable = false
                };

                if (!ApplyDeltaB(0L, sourceInput, deltaInput, out output))
                {
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "ApplyDeltaB failed");
                }

                ulong outputLength64 = output.uSize.ToUInt64();
                if (outputLength64 > Int32.MaxValue)
                {
                    throw new InvalidOperationException("Reconstructed target is too large for this utility.");
                }

                byte[] target = new byte[(int)outputLength64];
                Marshal.Copy(output.lpStart, target, 0, target.Length);
                return target;
            }
            finally
            {
                if (output.lpStart != IntPtr.Zero)
                {
                    DeltaFree(output.lpStart);
                }
                if (deltaMemory != IntPtr.Zero)
                {
                    Marshal.FreeHGlobal(deltaMemory);
                }
            }
        }
    }
}
'@
    }

    $targetBytes = [WinHelpReference.MsDelta]::ApplyFromEmpty($deltaBytes)
    if ([UInt64]$targetBytes.LongLength -ne $expectedLength) {
        throw "Reconstructed winhlp32.exe length is $($targetBytes.LongLength), expected $expectedLength."
    }

    [IO.File]::WriteAllBytes($resolvedOutput, $targetBytes)
    $actualHash = (Get-FileHash -LiteralPath $resolvedOutput -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualHash -ne $expectedHash) {
        Remove-Item -LiteralPath $resolvedOutput -Force -ErrorAction SilentlyContinue
        throw "Reconstructed winhlp32.exe SHA-256 is $actualHash, expected $expectedHash."
    }

    Write-Host "Extracted: $resolvedOutput"
    Write-Host "Size:      $expectedLength bytes"
    Write-Host "SHA-256:   $actualHash"
    Write-Host "Delta:      $deltaName ($actualDeltaHash)"
    Write-Host "Manifest:   $($target.GetAttribute('name'))"
}
finally {
    Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction SilentlyContinue
}
