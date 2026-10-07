[CmdletBinding()]
param(
    [switch] $SkipBuild
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
Set-StrictMode -Version Latest

$repositoryRoot = [System.IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
$tauriRoot = Join-Path $repositoryRoot "src-tauri"
$targetRoot = Join-Path $tauriRoot "target\store\msix"
$stageRoot = Join-Path $targetRoot "stage"
$outputRoot = Join-Path $targetRoot "output"
$verifyRoot = Join-Path $targetRoot "verify"

function Assert-WorkspacePath {
    param([Parameter(Mandatory = $true)] [string] $Path)

    $resolvedParent = [System.IO.Path]::GetFullPath((Split-Path -Parent $Path))
    if (-not $resolvedParent.StartsWith($repositoryRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to modify a path outside the repository: $Path"
    }
}

function Reset-Directory {
    param([Parameter(Mandatory = $true)] [string] $Path)

    Assert-WorkspacePath -Path $Path
    if (Test-Path -LiteralPath $Path) {
        Remove-Item -LiteralPath $Path -Recurse -Force
    }
    New-Item -ItemType Directory -Path $Path -Force | Out-Null
}

function Get-MakeAppxPath {
    $sdkRoot = "C:\Program Files (x86)\Windows Kits\10\bin"
    $candidates = @(Get-ChildItem -LiteralPath $sdkRoot -Directory -ErrorAction SilentlyContinue | ForEach-Object {
        $tool = Join-Path $_.FullName "x64\makeappx.exe"
        if (Test-Path -LiteralPath $tool -PathType Leaf) {
            [PSCustomObject]@{ Version = [version]$_.Name; Path = $tool }
        }
    } | Sort-Object Version -Descending)

    if ($candidates.Count -eq 0) {
        throw "MakeAppx.exe was not found. Install the Windows 10/11 SDK before building the Store package."
    }
    return $candidates[0].Path
}

function Convert-ToStoreVersion {
    param([Parameter(Mandatory = $true)] [string] $AppVersion)

    $match = [regex]::Match($AppVersion, '^(\d+)\.(\d+)\.(\d+)')
    if (-not $match.Success) {
        throw "App version must start with major.minor.patch: $AppVersion"
    }

    $parts = @(
        ([int]$match.Groups[1].Value + 1),
        [int]$match.Groups[2].Value,
        [int]$match.Groups[3].Value,
        0
    )
    if ($parts | Where-Object { $_ -lt 0 -or $_ -gt 65535 }) {
        throw "Store package version fields must be between 0 and 65535."
    }
    return ($parts -join ".")
}

function Get-Sha256Hash {
    param([Parameter(Mandatory = $true)] [string] $Path)

    $stream = [System.IO.File]::OpenRead($Path)
    try {
        $algorithm = [System.Security.Cryptography.SHA256]::Create()
        try {
            return ([System.BitConverter]::ToString($algorithm.ComputeHash($stream))).Replace("-", "")
        }
        finally {
            $algorithm.Dispose()
        }
    }
    finally {
        $stream.Dispose()
    }
}

$packageJson = Get-Content -LiteralPath (Join-Path $repositoryRoot "package.json") -Raw | ConvertFrom-Json
$appVersion = [string]$packageJson.version
$storeVersion = Convert-ToStoreVersion -AppVersion $appVersion
$applicationExe = Join-Path $tauriRoot "target\release\be-asset-optimizer.exe"
$manifestTemplate = Join-Path $repositoryRoot "store\AppxManifest.xml.template"
$iconRoot = Join-Path $tauriRoot "icons"
$makeAppx = Get-MakeAppxPath

if (-not $SkipBuild) {
    Write-Host "Building Be Asset Optimizer $appVersion..."
    Push-Location $repositoryRoot
    try {
        & npm.cmd run tauri build -- --no-bundle
        if ($LASTEXITCODE -ne 0) {
            throw "Tauri release build failed with exit code $LASTEXITCODE."
        }
    }
    finally {
        Pop-Location
    }
}

if (-not (Test-Path -LiteralPath $applicationExe -PathType Leaf)) {
    throw "Application executable not found: $applicationExe"
}

Reset-Directory -Path $stageRoot
New-Item -ItemType Directory -Path $outputRoot -Force | Out-Null
Reset-Directory -Path $verifyRoot

$assetRoot = Join-Path $stageRoot "Assets"
$noticeRoot = Join-Path $stageRoot "notices"
New-Item -ItemType Directory -Path $assetRoot, $noticeRoot -Force | Out-Null

Copy-Item -LiteralPath $applicationExe -Destination (Join-Path $stageRoot "be-asset-optimizer.exe")
Copy-Item -LiteralPath (Join-Path $iconRoot "StoreLogo.png") -Destination $assetRoot
Copy-Item -LiteralPath (Join-Path $iconRoot "Square44x44Logo.png") -Destination $assetRoot
Copy-Item -LiteralPath (Join-Path $iconRoot "Square71x71Logo.png") -Destination $assetRoot
Copy-Item -LiteralPath (Join-Path $iconRoot "Square150x150Logo.png") -Destination $assetRoot
Copy-Item -LiteralPath (Join-Path $repositoryRoot "LICENSE") -Destination $noticeRoot
Copy-Item -LiteralPath (Join-Path $repositoryRoot "PRIVACY.md") -Destination $noticeRoot
Copy-Item -LiteralPath (Join-Path $repositoryRoot "THIRD_PARTY_NOTICES.md") -Destination $noticeRoot

$manifest = (Get-Content -LiteralPath $manifestTemplate -Raw).Replace("{{PACKAGE_VERSION}}", $storeVersion)
[System.IO.File]::WriteAllText(
    (Join-Path $stageRoot "AppxManifest.xml"),
    $manifest,
    [System.Text.UTF8Encoding]::new($false)
)

$outputPath = Join-Path $outputRoot "Be-Asset-Optimizer_$($appVersion)_x64.msix"
Write-Host "Packing unsigned Store submission MSIX $storeVersion..."
& $makeAppx pack /d $stageRoot /p $outputPath /o
if ($LASTEXITCODE -ne 0) {
    throw "MakeAppx pack failed with exit code $LASTEXITCODE."
}

Write-Host "Verifying package contents..."
& $makeAppx unpack /p $outputPath /d $verifyRoot /o
if ($LASTEXITCODE -ne 0) {
    throw "MakeAppx verification unpack failed with exit code $LASTEXITCODE."
}

$verifiedManifest = [xml](Get-Content -LiteralPath (Join-Path $verifyRoot "AppxManifest.xml") -Raw)
$identity = $verifiedManifest.Package.Identity
if ($identity.Name -ne "StudioJuh.BeAssetOptimizer" -or
    $identity.Publisher -ne "CN=32199432-A020-4365-AE9B-74806B8E24F8" -or
    $identity.Version -ne $storeVersion) {
    throw "Packed manifest identity does not match the Partner Center identity."
}

$hash = Get-Sha256Hash -Path $outputPath
$result = Get-Item -LiteralPath $outputPath
Write-Host ""
Write-Host "Microsoft Store package ready:"
Write-Host "  Path: $($result.FullName)"
Write-Host "  App version: $appVersion"
Write-Host "  Store version: $storeVersion"
Write-Host "  Size: $($result.Length) bytes"
Write-Host "  SHA-256: $hash"
Write-Host "  Signature: unsigned (Microsoft Store signs after certification)"
