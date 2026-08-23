<#
.SYNOPSIS
  Fails if a Windows binary imports a C runtime DLL that is not part of Windows.

.DESCRIPTION
  The MSVC target links the C runtime dynamically by default, which makes the
  binary import VCRUNTIME140.dll. That DLL ships with the Visual C++
  Redistributable, not with Windows, so on a clean install or a fresh VM the
  program dies before main with a "VCRUNTIME140.dll was not found" box.
  `.cargo/config.toml` links the runtime statically to prevent that; this script
  is what proves it happened, on the artifact rather than on the build command.

  It reads the PE import directory — the actual list of DLLs the loader will
  resolve at startup — rather than scanning for strings, so an unrelated
  mention of a name in the binary cannot make it pass or fail.

  Run it on the exe from the ZIP and on the one extracted from the MSI: the MSI
  carries its own copy, and only checking the loose exe would let a stale
  installer payload through.

.EXAMPLE
  pwsh packaging/windows/assert-standalone.ps1 -Path target/x86_64-pc-windows-msvc/release/fastf.exe
#>
[CmdletBinding()]
param(
  # One or more binaries to check. All are checked before the script fails, so
  # one run reports every offender.
  [Parameter(Mandatory = $true, ValueFromRemainingArguments = $true)]
  [string[]] $Path
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# Imports that mean "this machine needs a redistributable installed".
# api-ms-win-crt-* is the UCRT: shipped with Windows 10+, but it only appears
# in the import table when the CRT is linked dynamically, so its presence still
# means the static link did not take effect.
#
# The version digit in the first three patterns is load-bearing: `msvcrt.dll`
# with no digits is a Windows system DLL (what the mingw target links against),
# while `MSVCR120.dll` is a redistributable.
$forbidden = @(
  '^vcruntime\d',   # VCRUNTIME140.dll, VCRUNTIME140_1.dll — the redist
  '^msvcp\d',       # MSVCP140.dll — the C++ redist
  '^msvcr\d',       # MSVCR120.dll and friends — older redists
  '^api-ms-win-crt', # UCRT forwarders, present only with a dynamic CRT
  '^ucrtbase'
)

function Get-ImportedDll {
  param([string] $File)

  $bytes = [System.IO.File]::ReadAllBytes($File)
  if ($bytes.Length -lt 0x40 -or $bytes[0] -ne 0x4D -or $bytes[1] -ne 0x5A) {
    throw "$File is not a PE image (no MZ header)"
  }

  $peOffset = [BitConverter]::ToInt32($bytes, 0x3C)
  if ([BitConverter]::ToUInt32($bytes, $peOffset) -ne 0x00004550) {
    throw "$File is not a PE image (no PE signature)"
  }

  $sectionCount = [BitConverter]::ToUInt16($bytes, $peOffset + 6)
  $optionalSize = [BitConverter]::ToUInt16($bytes, $peOffset + 20)
  $optional     = $peOffset + 24
  $magic        = [BitConverter]::ToUInt16($bytes, $optional)

  # The data directory sits after the optional header's fixed part, whose size
  # differs between PE32 (0x10b) and PE32+ (0x20b).
  $dataDirs = switch ($magic) {
    0x20b   { $optional + 112 }
    0x10b   { $optional + 96 }
    default { throw ("$File has an unrecognized optional header magic 0x{0:x}" -f $magic) }
  }

  # Directory entry 1 is the import table: an RVA, then a size.
  $importRva = [BitConverter]::ToUInt32($bytes, $dataDirs + 8)
  if ($importRva -eq 0) { return @() }

  # Section headers follow the optional header; 40 bytes each.
  $sections = @(0..($sectionCount - 1) | ForEach-Object {
    $s = $peOffset + 24 + $optionalSize + ($_ * 40)
    [pscustomobject]@{
      VirtualAddress = [BitConverter]::ToUInt32($bytes, $s + 12)
      VirtualSize    = [BitConverter]::ToUInt32($bytes, $s + 8)
      RawSize        = [BitConverter]::ToUInt32($bytes, $s + 16)
      RawPointer     = [BitConverter]::ToUInt32($bytes, $s + 20)
    }
  })

  function Resolve-Rva {
    param([uint32] $Rva)
    foreach ($s in $sections) {
      $span = [Math]::Max($s.VirtualSize, $s.RawSize)
      if ($Rva -ge $s.VirtualAddress -and $Rva -lt ($s.VirtualAddress + $span)) {
        return [int]($Rva - $s.VirtualAddress + $s.RawPointer)
      }
    }
    throw ("RVA 0x{0:x} in $File falls outside every section" -f $Rva)
  }

  function Read-AsciiZ {
    param([int] $Offset)
    $end = $Offset
    while ($end -lt $bytes.Length -and $bytes[$end] -ne 0) { $end++ }
    [System.Text.Encoding]::ASCII.GetString($bytes, $Offset, $end - $Offset)
  }

  # Import descriptors are 20 bytes each and end at an all-zero one. The DLL
  # name RVA is at +12.
  $names = New-Object System.Collections.Generic.List[string]
  $cursor = Resolve-Rva $importRva
  while ($true) {
    $nameRva = [BitConverter]::ToUInt32($bytes, $cursor + 12)
    if ($nameRva -eq 0) { break }
    $names.Add((Read-AsciiZ (Resolve-Rva $nameRva)))
    $cursor += 20
  }
  $names
}

$failed = $false
foreach ($file in $Path) {
  $resolved = (Resolve-Path -LiteralPath $file).Path
  $imports = @(Get-ImportedDll -File $resolved)
  Write-Host "$resolved imports: $($imports -join ', ')"

  $bad = @($imports | Where-Object {
    $name = $_.ToLowerInvariant()
    @($forbidden | Where-Object { $name -match $_ }).Count -gt 0
  })

  if ($bad.Count -gt 0) {
    Write-Host "::error::$resolved depends on the Visual C++ Redistributable: $($bad -join ', '). Expected a statically linked CRT — check that .cargo/config.toml still sets target-feature=+crt-static for x86_64-pc-windows-msvc, and that RUSTFLAGS in the environment is not overriding it."
    $failed = $true
  } else {
    Write-Host "  OK - no redistributable CRT import"
  }
}

if ($failed) { exit 1 }
