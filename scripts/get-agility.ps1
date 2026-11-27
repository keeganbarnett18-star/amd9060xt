# Puts Microsoft's DirectX 12 Agility SDK runtime (D3D12Core.dll and the
# debug layer, d3d12SDKLayers.dll) in a D3D12 folder beside victory.exe, as
# the program asks for (D3D12SDKVersion 620 in apps/victory/src/main.rs).
# The DLLs are Microsoft's redistributable, fetched from nuget.org and not
# kept in this repository. Run from the repository root after building:
#   powershell -ExecutionPolicy Bypass -File scripts\get-agility.ps1
param([string]$Version = '1.620.2', [string[]]$Targets = @('target\release', 'target-diag\release'))
$ErrorActionPreference = 'Stop'
$work = Join-Path $env:TEMP "victory-agility-$Version"
$pkg = Join-Path $work 'd3d12.nupkg'
if (-not (Test-Path $pkg)) {
    New-Item -ItemType Directory -Force $work | Out-Null
    $url = "https://api.nuget.org/v3-flatcontainer/microsoft.direct3d.d3d12/$Version/microsoft.direct3d.d3d12.$Version.nupkg"
    Invoke-WebRequest -Uri $url -OutFile $pkg
}
$out = Join-Path $work 'pkg'
if (-not (Test-Path $out)) { Expand-Archive -Path $pkg -DestinationPath $out }
$bin = Join-Path $out 'build\native\bin\x64'
foreach ($dll in 'D3D12Core.dll', 'd3d12SDKLayers.dll') {
    $sig = Get-AuthenticodeSignature (Join-Path $bin $dll)
    if ($sig.Status -ne 'Valid' -or $sig.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation') {
        throw "$dll is not signed by Microsoft"
    }
}
foreach ($t in $Targets) {
    if (-not (Test-Path $t)) { continue }
    $dest = Join-Path $t 'D3D12'
    New-Item -ItemType Directory -Force $dest | Out-Null
    Copy-Item (Join-Path $bin 'D3D12Core.dll'), (Join-Path $bin 'd3d12SDKLayers.dll') $dest -Force
    "Agility SDK $Version -> $dest"
}
