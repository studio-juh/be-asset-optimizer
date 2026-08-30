[CmdletBinding()]
param(
    [string] $DestinationRoot
)

$ErrorActionPreference = "Stop"

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$componentRoot = if ([string]::IsNullOrWhiteSpace($DestinationRoot)) {
    Join-Path $repositoryRoot "src-tauri\resources\realesrgan"
}
else {
    [System.IO.Path]::GetFullPath($DestinationRoot)
}
$modelDirectory = Join-Path $componentRoot "models"

$componentGroups = @(
    @{
        Url = "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.2.5.0/realesrgan-ncnn-vulkan-20220424-windows.zip"
        Files = @(
            @{
                Name = "realesrgan-ncnn-vulkan.exe"
                Destination = $componentRoot
                Sha256 = "07E49F7CBB4EDE01AE4DD4C399D3A7E5846E3D2085C3128EFF881E55CB7B1A0C"
            },
            @{
                Name = "vcomp140.dll"
                Destination = $componentRoot
                Sha256 = "8F72EF2E483465444B2059FC6744D6CB22CD8D8A27F6FA56BEFD2A42DCD0F78B"
            },
            @{
                Name = "realesrgan-x4plus.bin"
                Destination = $modelDirectory
                Sha256 = "713EE713B0353AFAA27976F0563A64A5043BD70B9BD8936C2E26E25EBCDBCDDF"
            },
            @{
                Name = "realesrgan-x4plus.param"
                Destination = $modelDirectory
                Sha256 = "35330ECECCEA33B6C397A72548E788D5D53BECEE4734C50B7FADA36E89F10A86"
            }
        )
    },
    @{
        Url = "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.2.3.0/realesrgan-ncnn-vulkan-20211212-windows.zip"
        Files = @(
            @{
                Name = "realesrnet-x4plus.bin"
                Destination = $modelDirectory
                Sha256 = "26BCCFCC82D9E8260C0C6B0DFFB34AB297982740882D1F33C6D423F70B562C40"
            },
            @{
                Name = "realesrnet-x4plus.param"
                Destination = $modelDirectory
                Sha256 = "35330ECECCEA33B6C397A72548E788D5D53BECEE4734C50B7FADA36E89F10A86"
            }
        )
    }
)

function Test-ComponentFile {
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

$allComponentsReady = $true
foreach ($group in $componentGroups) {
    foreach ($component in $group.Files) {
        $destination = Join-Path $component.Destination $component.Name
        if (-not (Test-ComponentFile -Path $destination -ExpectedSha256 $component.Sha256)) {
            $allComponentsReady = $false
            break
        }
    }
}

if ($allComponentsReady) {
    Write-Host "AI components are ready."
    exit 0
}

Add-Type -AssemblyName System.IO.Compression.FileSystem

foreach ($group in $componentGroups) {
    $missingComponents = @($group.Files | Where-Object {
        $destination = Join-Path $_.Destination $_.Name
        -not (Test-ComponentFile -Path $destination -ExpectedSha256 $_.Sha256)
    })
    if ($missingComponents.Count -eq 0) {
        continue
    }

    $temporaryRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("be-asset-optimizer-components-" + [guid]::NewGuid().ToString("N"))
    $archivePath = Join-Path $temporaryRoot "realesrgan-windows.zip"
    $extractDirectory = Join-Path $temporaryRoot "extracted"

    try {
        New-Item -ItemType Directory -Path $temporaryRoot | Out-Null
        New-Item -ItemType Directory -Path $extractDirectory | Out-Null

        Write-Host "Downloading official Real-ESRGAN components..."
        $webClient = New-Object System.Net.WebClient
        try {
            $webClient.DownloadFile($group.Url, $archivePath)
        }
        finally {
            $webClient.Dispose()
        }

        [System.IO.Compression.ZipFile]::ExtractToDirectory($archivePath, $extractDirectory)

        foreach ($component in $missingComponents) {
            $matches = @(Get-ChildItem -LiteralPath $extractDirectory -Recurse -File -Filter $component.Name)
            if ($matches.Count -ne 1) {
                throw "Expected exactly one $($component.Name) in the official archive, found $($matches.Count)."
            }

            if (-not (Test-ComponentFile -Path $matches[0].FullName -ExpectedSha256 $component.Sha256)) {
                throw "SHA-256 verification failed for $($component.Name)."
            }

            New-Item -ItemType Directory -Path $component.Destination -Force | Out-Null
            Copy-Item -LiteralPath $matches[0].FullName -Destination (Join-Path $component.Destination $component.Name) -Force
        }
    }
    finally {
        if (Test-Path -LiteralPath $temporaryRoot) {
            Remove-Item -LiteralPath $temporaryRoot -Recurse -Force
        }
    }
}

Write-Host "AI components were downloaded and verified."
