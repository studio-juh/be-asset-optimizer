[CmdletBinding()]
param(
    [string] $DestinationDirectory
)

$ErrorActionPreference = "Stop"

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$modelDirectory = if ([string]::IsNullOrWhiteSpace($DestinationDirectory)) {
    Join-Path $repositoryRoot "src-tauri\resources\realesrgan\models"
}
else {
    [System.IO.Path]::GetFullPath($DestinationDirectory)
}
$releaseUrl = "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.2.3.0/realesrgan-ncnn-vulkan-20211212-windows.zip"

$modelFiles = @(
    @{
        Name = "realesrgan-x4plus.bin"
        Sha256 = "713EE713B0353AFAA27976F0563A64A5043BD70B9BD8936C2E26E25EBCDBCDDF"
    },
    @{
        Name = "realesrgan-x4plus.param"
        Sha256 = "35330ECECCEA33B6C397A72548E788D5D53BECEE4734C50B7FADA36E89F10A86"
    },
    @{
        Name = "realesrnet-x4plus.bin"
        Sha256 = "26BCCFCC82D9E8260C0C6B0DFFB34AB297982740882D1F33C6D423F70B562C40"
    },
    @{
        Name = "realesrnet-x4plus.param"
        Sha256 = "35330ECECCEA33B6C397A72548E788D5D53BECEE4734C50B7FADA36E89F10A86"
    }
)

function Test-ModelFile {
    param(
        [Parameter(Mandatory = $true)] [string] $Path,
        [Parameter(Mandatory = $true)] [string] $ExpectedSha256
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return $false
    }

    $stream = [System.IO.File]::OpenRead($Path)
    try {
        $sha256 = [System.Security.Cryptography.SHA256]::Create()
        try {
            $actualSha256 = -join ($sha256.ComputeHash($stream) | ForEach-Object { $_.ToString("X2") })
        }
        finally {
            $sha256.Dispose()
        }
    }
    finally {
        $stream.Dispose()
    }

    return $actualSha256 -eq $ExpectedSha256
}

$allModelsReady = $true
foreach ($model in $modelFiles) {
    $destination = Join-Path $modelDirectory $model.Name
    if (-not (Test-ModelFile -Path $destination -ExpectedSha256 $model.Sha256)) {
        $allModelsReady = $false
        break
    }
}

if ($allModelsReady) {
    Write-Host "AI model components are ready."
    exit 0
}

$temporaryRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("be-asset-optimizer-models-" + [guid]::NewGuid().ToString("N"))
$archivePath = Join-Path $temporaryRoot "realesrgan-windows.zip"
$extractDirectory = Join-Path $temporaryRoot "extracted"

try {
    New-Item -ItemType Directory -Path $temporaryRoot | Out-Null
    New-Item -ItemType Directory -Path $extractDirectory | Out-Null
    New-Item -ItemType Directory -Path $modelDirectory -Force | Out-Null

    Write-Host "Downloading official Real-ESRGAN model components..."
    $webClient = New-Object System.Net.WebClient
    try {
        $webClient.DownloadFile($releaseUrl, $archivePath)
    }
    finally {
        $webClient.Dispose()
    }

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [System.IO.Compression.ZipFile]::ExtractToDirectory($archivePath, $extractDirectory)

    foreach ($model in $modelFiles) {
        $matches = @(Get-ChildItem -LiteralPath $extractDirectory -Recurse -File -Filter $model.Name)
        if ($matches.Count -ne 1) {
            throw "Expected exactly one $($model.Name) in the official archive, found $($matches.Count)."
        }

        if (-not (Test-ModelFile -Path $matches[0].FullName -ExpectedSha256 $model.Sha256)) {
            throw "SHA-256 verification failed for $($model.Name)."
        }

        Copy-Item -LiteralPath $matches[0].FullName -Destination (Join-Path $modelDirectory $model.Name) -Force
    }

    Write-Host "AI model components were downloaded and verified."
}
finally {
    if (Test-Path -LiteralPath $temporaryRoot) {
        Remove-Item -LiteralPath $temporaryRoot -Recurse -Force
    }
}
