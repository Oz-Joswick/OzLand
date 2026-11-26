<#
.SYNOPSIS
    Builds a release of the game and publishes it, so your friends' copies
    update themselves the next time they start the game.

.DESCRIPTION
    1. Bumps the version in Cargo.toml (or uses -Version).
    2. Builds the release executable.
    3. Writes release\ with:
         manifest.txt        version + a hash of the assets (what copies check)
         walkabout.exe       the game (downloaded by every update)
         assets.zip          the assets (downloaded only when they changed)
         walkabout-full.zip  everything, for a friend's first download
    4. Uploads them as a GitHub release if the GitHub CLI (gh) is installed
       and -Repo is set; otherwise tells you which files to upload where.

    Settings are remembered in tools\release-*.txt, so after the first time
    a release is just:  .\tools\release.ps1

.EXAMPLE
    .\tools\release.ps1 -Repo yourname/walkabout
    First release to GitHub: files go to github.com/yourname/walkabout/releases.

.EXAMPLE
    .\tools\release.ps1 -Version 1.0.0 -Url https://example.com/walkabout/
    Any web host: upload the files from release\ to that address yourself.
#>
param(
    # New version, e.g. 0.2.0. Default: the next patch version.
    [string]$Version,
    # GitHub repository (owner/name) to publish to with the gh CLI.
    [string]$Repo,
    # Where the files will be downloadable from (default for GitHub:
    # https://github.com/<Repo>/releases/latest/download/).
    [string]$Url,
    # Also put the ready-to-share folder and zip in your Downloads folder.
    [switch]$Share,
    # Build only: don't upload, even if the GitHub CLI is installed.
    [switch]$NoPublish
)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)

# --- Remembered settings ---------------------------------------------------
$repoFile = "tools\release-repo.txt"
$urlFile = "tools\release-url.txt"
if (-not $Repo -and (Test-Path $repoFile)) { $Repo = (Get-Content $repoFile -Raw).Trim() }
if (-not $Url -and (Test-Path $urlFile)) { $Url = (Get-Content $urlFile -Raw).Trim() }
if (-not $Url -and $Repo) { $Url = "https://github.com/$Repo/releases/latest/download/" }
if (-not $Url) { throw "First time: give -Repo owner/name (GitHub) or -Url <address the files will be downloadable from>." }
if (-not $Url.EndsWith("/")) { $Url += "/" }
Set-Content $urlFile $Url -Encoding ascii
if ($Repo) { Set-Content $repoFile $Repo -Encoding ascii }

# --- Version -----------------------------------------------------------------
$toml = [IO.File]::ReadAllText((Resolve-Path Cargo.toml))
$current = [regex]::Match($toml, '(?m)^version = "([^"]+)"').Groups[1].Value
if (-not $Version) {
    $parts = $current.Split('.')
    $parts[2] = [string]([int]$parts[2] + 1)
    $Version = $parts -join '.'
}
if ($Version -ne $current) {
    $toml = ([regex]'(?m)^version = "[^"]+"').Replace($toml, "version = `"$Version`"", 1)
    [IO.File]::WriteAllText((Resolve-Path Cargo.toml), $toml, (New-Object Text.UTF8Encoding $false))
}
Write-Host "Releasing version $Version (was $current)"

# --- Build -------------------------------------------------------------------
# cargo reports progress on stderr, which PowerShell 5 would treat as an error.
$ErrorActionPreference = "Continue"
cargo build --release 2>&1 | ForEach-Object { "$_" }
$ErrorActionPreference = "Stop"
if ($LASTEXITCODE -ne 0) { throw "The build failed." }

$out = "release"
if (Test-Path $out) { Remove-Item -Recurse -Force $out }
New-Item -ItemType Directory $out | Out-Null
$packageName = [regex]::Match($toml, '(?m)^name = "([^"]+)"').Groups[1].Value
if (-not $packageName) { throw "Could not read the Cargo package name." }
Copy-Item "target\release\$packageName.exe" "$out\walkabout.exe"

# Steam's library (multiplayer through Steam, see src/steam.rs). The build
# puts it beside the executable; it travels with the assets, which unpack
# next to the game.
$steamDll = "target\release\steam_api64.dll"
if (-not (Test-Path $steamDll)) {
    $found = Get-ChildItem target\release\build -Recurse -Filter steam_api64.dll -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $found) { throw "steam_api64.dll wasn't found: run 'cargo build --release' once more." }
    $steamDll = $found.FullName
}
$steamDll = (Resolve-Path $steamDll).Path

# --- Assets hash (contents, not dates: unchanged assets aren't re-downloaded) --
$root = (Resolve-Path assets).Path
$files = Get-ChildItem assets -Recurse -File | Where-Object { $_.Name -ne ".hash" } | Sort-Object FullName
$lines = foreach ($f in $files) {
    $rel = $f.FullName.Substring($root.Length + 1).Replace('\', '/')
    "${rel}:$((Get-FileHash $f.FullName -Algorithm SHA256).Hash)"
}
$lines += "steam_api64.dll:$((Get-FileHash $steamDll -Algorithm SHA256).Hash)"
$sha = [Security.Cryptography.SHA256]::Create()
$digest = $sha.ComputeHash([Text.Encoding]::UTF8.GetBytes(($lines -join "`n")))
$hash = ([BitConverter]::ToString($digest)).Replace("-", "").Substring(0, 16).ToLower()
Set-Content "assets\.hash" $hash -NoNewline -Encoding ascii

# --- Zips (entry names with forward slashes, which every unzipper reads) -------
Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
function New-Zip([string]$Path, [System.Collections.ArrayList]$Entries) {
    $stream = [IO.File]::Open((Join-Path (Get-Location) $Path), [IO.FileMode]::Create)
    $zip = New-Object IO.Compression.ZipArchive($stream, [IO.Compression.ZipArchiveMode]::Create)
    foreach ($e in $Entries) {
        [void][IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, $e[0], $e[1], [IO.Compression.CompressionLevel]::Optimal)
    }
    $zip.Dispose()
    $stream.Dispose()
}
$assetEntries = New-Object System.Collections.ArrayList
foreach ($f in Get-ChildItem assets -Recurse -File) {
    [void]$assetEntries.Add(@($f.FullName, ("assets/" + $f.FullName.Substring($root.Length + 1).Replace('\', '/'))))
}
[void]$assetEntries.Add(@($steamDll, "steam_api64.dll"))
New-Zip "$out\assets.zip" $assetEntries

Set-Content "$out\update.cfg" "# Where this copy of the game checks for updates.`r`nurl = $Url" -Encoding ascii
Set-Content "$out\manifest.txt" "version = $Version`r`nassets = $hash" -Encoding ascii

# The full download for new players: a folder named after the game with
# the game, its assets, the update address and how to play.
$fullName = "$packageName.zip"
$full = New-Object System.Collections.ArrayList
[void]$full.Add(@((Resolve-Path "$out\walkabout.exe").Path, "$packageName/$packageName.exe"))
[void]$full.Add(@((Resolve-Path "$out\update.cfg").Path, "$packageName/update.cfg"))
[void]$full.Add(@((Resolve-Path "tools\how-to-play.txt").Path, "$packageName/How to play.txt"))
foreach ($e in $assetEntries) { [void]$full.Add(@($e[0], ("$packageName/" + $e[1]))) }
New-Zip "$out\$fullName" $full
Remove-Item "$out\update.cfg"

$uploads = @("$out\manifest.txt", "$out\walkabout.exe", "$out\assets.zip", "$out\$fullName")

# --- Share: the folder and zip in Downloads -------------------------------------
if ($Share) {
    $downloads = Join-Path $env:USERPROFILE "Downloads"
    $zipTarget = Join-Path $downloads $fullName
    $folderTarget = Join-Path $downloads $packageName
    Copy-Item "$out\$fullName" $zipTarget -Force
    if (Test-Path $folderTarget) { Remove-Item -Recurse -Force $folderTarget }
    [IO.Compression.ZipFile]::ExtractToDirectory($zipTarget, $downloads)
    Write-Host "Shareable copy: $folderTarget and $zipTarget"
}

# --- Publish ---------------------------------------------------------------
$gh = Get-Command gh -ErrorAction SilentlyContinue
if ($gh -and $Repo -and -not $NoPublish) {
    $ErrorActionPreference = "Continue"
    & $gh.Source release create "v$Version" @uploads --repo $Repo --title "$packageName $Version" --notes "Version $Version. Existing copies update themselves on their next start."
    $ErrorActionPreference = "Stop"
    if ($LASTEXITCODE -ne 0) { throw "Uploading the release failed (is gh logged in? run: gh auth login)." }
    Write-Host ""
    Write-Host "Published version $Version. Friends' games update on their next start."
    Write-Host "New players: download $fullName from https://github.com/$Repo/releases/latest"
} else {
    Write-Host ""
    Write-Host "Built version $Version in .\release. Upload these four files so they're downloadable at:"
    Write-Host "  $Url"
    $uploads | ForEach-Object { Write-Host "  $_" }
    if ($Repo) {
        Write-Host "(Install the GitHub CLI, https://cli.github.com, and run 'gh auth login' to have this done for you.)"
    }
}
