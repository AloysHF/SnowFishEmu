<#
.SYNOPSIS
    Capture screenshots for SnowFish .app packages.

.DESCRIPTION
    Runs the standalone emulator headlessly for every .app package under
    the selected game directory. PNG screenshots are written to
    docs/images. When no binary is supplied, the latest release binary is
    built before capture.

.PARAMETER Frames
    Number of frames to run before capturing. Default: 200. Selected
    applications use capture-specific frame counts.

.PARAMETER InstructionLimit
    Maximum guest instructions allowed during boot and each frame.
    Default: 200000000.

.PARAMETER Binary
    Path to the snowfishemu executable. Default: the release build output.

.PARAMETER GameDirectory
    Directory searched recursively for .app packages.

.PARAMETER OutputDirectory
    Directory where PNG screenshots are written.
#>

param(
    [ValidateRange(0, [int]::MaxValue)]
    [int]$Frames = 200,
    [ValidateRange(1, [uint64]::MaxValue)]
    [uint64]$InstructionLimit = 200000000,
    [string]$Binary = "",
    [Parameter(Mandatory)]
    [string]$GameDirectory,
    [string]$OutputDirectory = ""
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot

if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path $repoRoot "docs\images"
}
if (-not (Test-Path -LiteralPath $GameDirectory -PathType Container)) {
    Write-Error "Game directory not found: $GameDirectory"
    exit 1
}

New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null

if (-not $Binary) {
    $Binary = Join-Path $repoRoot "target\release\snowfishemu.exe"
    if ($env:OS -ne "Windows_NT") {
        $Binary = Join-Path $repoRoot "target/release/snowfishemu"
    }

    Write-Host "Building the latest release binary..." -ForegroundColor Yellow
    try {
        Push-Location $repoRoot
        cargo build --release -p snowfishemu --bin snowfishemu
        if ($LASTEXITCODE -ne 0) {
            throw "Release build failed with exit code $LASTEXITCODE."
        }
    } finally {
        Pop-Location
    }
}

if (-not (Test-Path -LiteralPath $Binary -PathType Leaf)) {
    Write-Error "snowfishemu binary not found: $Binary"
    exit 1
}

# Per-title bring-up recipes (docs/Game-Compatibility.md): the scripted
# phone-key presses (frame:key; 10 = left soft key, 5 = OK, 25 = right
# soft key) and the offline switches each title needs to reach its play
# state. Titles not listed run the default frame count with no presses.
$recipes = @{
    "SanguoTD8_QVGA_SNOWFISHV2" = @{
        Image  = "三国危城无双"
        Frames = 455
        Flags  = @()
        Press  = @("120:10", "300:10", "350:10", "400:10", "450:10")
    }
    "cjdfw_QVGA_SNOWFISHV2" = @{
        Frames = 200
        Press  = @("30:10", "60:10", "90:5", "120:5", "150:10", "180:5")
    }
    "trunk_QVGA_SNOWFISHV2" = @{
        Image  = "五子棋"
        Frames = 200
        Flags  = @()
        Press  = @("30:10", "60:10", "90:10", "120:10", "150:10", "180:10")
    }
    "Mengde_QVGA_SNOWFISHV2" = @{
        Image  = "巨作RPG曹操冢"
        Frames = 200
        Flags  = @()
        Press  = @("30:10", "60:10", "90:10", "120:10", "150:10", "180:10")
    }
    "Mengde_i_QVGA_SNOWFISHV2" = @{
        Image  = "曹操冢超豪华版"
        Frames = 200
        Flags  = @()
        Press  = @("30:10", "60:10", "90:10", "120:10", "150:10", "180:10")
    }
    "yonghengmota_QVGA_SNOWFISHV2" = @{
        Image  = "永恒魔塔"
        Frames = 200
        Flags  = @()
        Press  = @("30:10", "60:10", "90:10", "120:10", "150:10", "180:10")
    }
    "yzxyl_QVGA_SNOWFISHV2" = @{
        Image  = "胭脂戏游龙"
        Frames = 200
        Flags  = @()
        Press  = @("30:10", "60:10", "90:10", "120:10", "150:10", "180:10")
    }
    "StarBall_QVGA_SNOWFISHV2" = @{
        Image  = "新星际撞球"
        Frames = 247
        Flags  = @()
        Press  = @("30:5", "60:5", "90:5", "120:5", "150:5", "213:6", "240:5")
    }
    "Mengde_pe_QVGA_SNOWFISHV2" = @{
        Image  = "新鬼吹灯"
        Frames = 300
        Flags  = @()
        Press  = @("30:5", "60:5", "90:5", "120:5")
    }
    "hjkg_QVGA_SNOWFISHV2" = @{
        Image  = "黄金矿工"
        Frames = 90
        Flags  = @()
        Press  = @("30:11", "65:5", "82:5")
    }
    "yzfb_QVGA_SNOWFISHV2" = @{
        Image  = "宇宙风暴"
        Frames = 132
        Flags  = @()
        Press  = @("30:10", "60:5", "98:5", "130:6")
    }
}

# Sibling resource containers are discovered by the emulator itself (the
# shared snowfishemu_core::res rule, matching the libretro core), so the
# script only names the packages to capture.

$games = @(Get-ChildItem -LiteralPath $GameDirectory -Recurse -File |
    Where-Object { $_.Extension -ieq ".app" -and $_.BaseName -inotmatch "Res\d*(_[^.]+)?$" } |
    Sort-Object FullName)

if ($games.Count -eq 0) {
    Write-Warning "No .app packages found under $GameDirectory"
    exit 0
}

Write-Host "Using binary: $Binary"
Write-Host "Game dir:     $GameDirectory"
Write-Host "Output dir:   $OutputDirectory"
Write-Host "Frames:       $Frames"
Write-Host "Games:        $($games.Count)"
Write-Host ""

$success = 0
$warned = 0
$failed = 0
$usedNames = [System.Collections.Generic.HashSet[string]]::new(
    [System.StringComparer]::OrdinalIgnoreCase
)

$corpusRoot = (Resolve-Path -LiteralPath $GameDirectory).Path.TrimEnd([char[]]"\/")
foreach ($game in $games) {
    $baseName = [System.IO.Path]::GetFileNameWithoutExtension($game.Name)
    $recipe = if ($recipes.ContainsKey($baseName)) { $recipes[$baseName] } else { @{} }
    $captureFrames = if ($recipe.ContainsKey("Frames")) { $recipe["Frames"] } else { $Frames }
    $frameLabel = if ($captureFrames -eq 1) { "1 frame" } else { "$captureFrames frames" }
    $imageDirectory = $OutputDirectory
    $relativeGamePath = $game.FullName.Substring($corpusRoot.Length + 1)
    $group, $relativePath = $relativeGamePath -split '[/\\]', 2
    if (-not $relativePath) {
        $group = ""
    }
    if ($group -in @("A", "B", "C")) {
        $imageDirectory = Join-Path $OutputDirectory $group
        New-Item -ItemType Directory -Force -Path $imageDirectory | Out-Null
    }
    $imageStem = if ($recipe["Image"]) { $recipe["Image"] } else { $baseName }
    $imageName = "$imageStem.png"
    $outPath = Join-Path $imageDirectory $imageName
    $suffix = 2
    while ($usedNames.Contains($outPath)) {
        $imageName = "$imageStem-$suffix.png"
        $outPath = Join-Path $imageDirectory $imageName
        $suffix++
    }
    [void]$usedNames.Add($outPath)
    Write-Host -NoNewline "  $imageName ($frameLabel) ... "

    # Prevent a previous capture from being reported as the current result.
    if (Test-Path -LiteralPath $outPath -PathType Leaf) {
        Remove-Item -LiteralPath $outPath
    }

    $status = "Fail"
    $detail = "Execution failed"
    $nonzero = $null
    $colors = $null
    $exitCode = -1
    $previousErrorActionPreference = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $captureArgs = @($game.FullName, "--screenshot", $outPath,
            "--screenshot-frames", $captureFrames,
            "--instruction-limit", $InstructionLimit)
        if ($recipe["Flags"]) {
            $captureArgs += $recipe["Flags"]
        }
        foreach ($press in $recipe["Press"]) {
            $captureArgs += @("--press", $press)
        }
        $output = & $Binary @captureArgs 2>&1
        $exitCode = $LASTEXITCODE
        $diagnostics = $output | Out-String
        $diagnostics = [regex]::Replace(
            $diagnostics,
            '\x1B\[[0-?]*[ -/]*[@-~]',
            ''
        )
        if ($diagnostics -match 'frame nonzero=(\d+) colors=(\d+)') {
            $nonzero = [int]$Matches[1]
            $colors = [int]$Matches[2]
        }

        if ($exitCode -ne 0) {
            $diagnosticLines = @($diagnostics -split '\r?\n' |
                ForEach-Object { $_.Trim() } |
                Where-Object { $_ })
            if ($diagnosticLines.Count -gt 0) {
                $detail = $diagnosticLines[$diagnosticLines.Count - 1]
                $detail = $detail -replace '^\d+:\s*', ''
                $detail = $detail -replace '^Error:\s*', ''
            } else {
                $detail = "Exited with code $exitCode"
            }
            Write-Host $detail -ForegroundColor Red
        } elseif (-not (Test-Path -LiteralPath $outPath -PathType Leaf)) {
            $detail = "No screenshot produced"
            Write-Host $detail -ForegroundColor Red
        } elseif ($null -eq $nonzero -or $null -eq $colors) {
            $status = "Warn"
            $detail = "Frame statistics unavailable"
            Write-Host $detail -ForegroundColor Yellow
        } elseif ($nonzero -eq 0 -or $colors -le 1) {
            $status = "Warn"
            $detail = "Blank or single-color frame"
            Write-Host "$detail ($colors colors)" -ForegroundColor Yellow
        } else {
            $status = "Pass"
            $detail = "Rendered $colors colors"
            $size = (Get-Item -LiteralPath $outPath).Length
            Write-Host "OK ($colors colors, $([math]::Round($size / 1KB)) KB)" -ForegroundColor Green
        }
    } catch {
        $detail = $_.Exception.Message
        Write-Host "FAILED ($detail)" -ForegroundColor Red
    } finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }

    switch ($status) {
        "Pass" { $success++ }
        "Warn" { $warned++ }
        default { $failed++ }
    }
}

Write-Host ""
Write-Host "Done: $success succeeded, $warned warned, $failed failed out of $($games.Count) total."
