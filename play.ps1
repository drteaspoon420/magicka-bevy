# Magicka rewrite launcher for Windows (PowerShell 5.1 or 7); the same options
# as play.sh. Build the game, convert the game's assets into a local cache on
# first run, then open the main menu (or play an explicit level).
#
#   .\play.ps1                     open the main menu
#   .\play.ps1 --skip-build        open the main menu using the existing build
#   .\play.ps1 Wizard_Castle.lvl   play another level (with its intro scripts)
#   .\play.ps1 --help              all options (hosting, joining, mods)
#
# If scripts are blocked on this PC, run play.cmd instead, or:
#   powershell -ExecutionPolicy Bypass -File play.ps1
#
# The game files are found in this order: MAGICKA_CONTENT in the environment,
# MAGICKA_CONTENT saved in .env, a game install linked or copied to .\Magicka,
# then a search of your Steam libraries (you are asked before it is used, and
# can save the choice to .env).

$ErrorActionPreference = 'Stop'
$Lab = $PSScriptRoot
Set-Location -LiteralPath $Lab
$OnWindows = $env:OS -eq 'Windows_NT'
$Exe = ''
if ($OnWindows) { $Exe = '.exe' }

function Write-Note([string]$text) { [Console]::Error.WriteLine($text) }

# --- settings from .env (parsed, never executed) ------------------------------
$EnvFile = Join-Path $Lab '.env'
$EnvKeys = @('MAGICKA_CONTENT', 'MAGICKA_CACHE', 'MAGICKA_FFMPEG')
if (Test-Path -LiteralPath $EnvFile -PathType Leaf) {
    foreach ($line in [IO.File]::ReadAllLines($EnvFile)) {
        if ($line -notmatch '^\s*(export\s+)?([A-Z_]+)=(.*)$') { continue }
        $key = $Matches[2]; $value = $Matches[3]
        if ($EnvKeys -notcontains $key) { continue }
        if ([Environment]::GetEnvironmentVariable($key)) { continue }  # the real environment wins
        if ($value -match '^"(.*)"$' -or $value -match "^'(.*)'$") { $value = $Matches[1] }
        [Environment]::SetEnvironmentVariable($key, $value)
    }
}

if (Test-Path -LiteralPath (Join-Path $Lab 'Cargo.toml')) {
    # Public repository: the workspace is the root.
    $Rewrite = $Lab
    $DefaultContent = Join-Path $Lab 'Magicka/Content'
    $Cache = $env:MAGICKA_CACHE; if (-not $Cache) { $Cache = Join-Path $Lab '.cache' }
} else {
    $Rewrite = Join-Path $Lab 'output/rewrite'
    $DefaultContent = Join-Path $Lab 'target/Magicka/Content'
    $Cache = $env:MAGICKA_CACHE; if (-not $Cache) { $Cache = Join-Path $Lab 'output/.cache' }
}

# --- finding the game ---------------------------------------------------------
function Test-Interactive {
    ($env:MAGICKA_PLAY_INTERACTIVE -eq '1') -or (-not [Console]::IsInputRedirected)
}
function Read-Answer {
    $line = [Console]::In.ReadLine()
    if ($null -eq $line) { return '' }
    $line
}
function Confirm-Choice([string]$question, [bool]$default) {
    $hint = '[y/N]'; if ($default) { $hint = '[Y/n]' }
    [Console]::Error.Write("$question $hint ")
    switch ((Read-Answer).Trim().ToLowerInvariant()) {
        { $_ -in 'y', 'yes' } { return $true }
        { $_ -in 'n', 'no' } { return $false }
        default { return $default }
    }
}
# Accept either the game install folder or its Content folder.
function Get-ContentOf([string]$dir) {
    $dir = $dir.Trim().Trim('"')
    if ($dir -eq '~') { $dir = $HOME }
    elseif ($dir.StartsWith('~/') -or $dir.StartsWith('~\')) { $dir = Join-Path $HOME $dir.Substring(2) }
    $dir = $dir.TrimEnd('/', '\')
    if (-not $dir) { return $null }
    if (Test-Path -LiteralPath (Join-Path $dir 'Content/Levels') -PathType Container) {
        return (Join-Path $dir 'Content')
    }
    if ((Split-Path -Leaf $dir) -eq 'Content' -and (Test-Path -LiteralPath (Join-Path $dir 'Levels') -PathType Container)) {
        return $dir
    }
    $null
}
# The path with every symbolic link or junction along it resolved.
function Resolve-Physical([string]$path) {
    $full = [IO.Path]::GetFullPath($path)
    $root = [IO.Path]::GetPathRoot($full)
    $current = $root
    foreach ($part in $full.Substring($root.Length).Split([char[]]@('\', '/'), [StringSplitOptions]::RemoveEmptyEntries)) {
        $next = Join-Path $current $part
        $item = Get-Item -LiteralPath $next -Force -ErrorAction SilentlyContinue
        if ($item -and $item.LinkType -and $item.Target) {
            $target = @($item.Target)[0]
            if (-not [IO.Path]::IsPathRooted($target)) { $target = Join-Path $current $target }
            $next = Resolve-Physical $target
        }
        $current = $next
    }
    $current
}
# Every Steam library on this machine, from each Steam root's libraryfolders.vdf.
function Get-SteamLibrary {
    $roots = @()
    if ($OnWindows) {
        foreach ($key in 'HKCU:\Software\Valve\Steam', 'HKLM:\SOFTWARE\WOW6432Node\Valve\Steam', 'HKLM:\SOFTWARE\Valve\Steam') {
            $props = Get-ItemProperty -Path $key -ErrorAction SilentlyContinue
            if ($props) {
                foreach ($name in 'SteamPath', 'InstallPath') {
                    if ($props.PSObject.Properties[$name]) { $roots += $props.$name }
                }
            }
        }
        foreach ($base in ${env:ProgramFiles(x86)}, $env:ProgramFiles) {
            if ($base) { $roots += (Join-Path $base 'Steam') }
        }
    } else {
        $roots += "$HOME/.steam/steam", "$HOME/.steam/root", "$HOME/.local/share/Steam",
            "$HOME/.var/app/com.valvesoftware.Steam/.local/share/Steam",
            "$HOME/snap/steam/common/.local/share/Steam",
            "$HOME/Library/Application Support/Steam"
    }
    foreach ($root in $roots) {
        if (-not (Test-Path -LiteralPath (Join-Path $root 'steamapps') -PathType Container)) { continue }
        $root
        $vdf = Join-Path $root 'steamapps/libraryfolders.vdf'
        if (-not (Test-Path -LiteralPath $vdf -PathType Leaf)) { continue }
        foreach ($line in [IO.File]::ReadAllLines($vdf)) {
            if ($line -match '^\s*"path"\s*"(.*)"\s*$') { $Matches[1].Replace('\\', '\') }
        }
    }
}
function Find-SteamContent {
    $seen = @{}
    foreach ($library in Get-SteamLibrary) {
        $found = Get-ContentOf (Join-Path $library 'steamapps/common/Magicka')
        if (-not $found) { continue }
        $found = Resolve-Physical $found
        if ($seen.ContainsKey($found)) { continue }
        $seen[$found] = $true
        $found
    }
}
function Save-Content([string]$path) {
    if ($path.Contains('"') -or $path.Contains("`n")) {
        Write-Note 'Not saving: the path contains a quote or newline.'; return
    }
    $lines = @()
    if (Test-Path -LiteralPath $EnvFile -PathType Leaf) {
        $lines = @([IO.File]::ReadAllLines($EnvFile) | Where-Object { $_ -notmatch '^\s*(export\s+)?MAGICKA_CONTENT=' })
    }
    $lines += "MAGICKA_CONTENT=`"$path`""
    $tmp = "$EnvFile.tmp"
    [IO.File]::WriteAllText($tmp, (($lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding $false))
    Move-Item -LiteralPath $tmp -Destination $EnvFile -Force
    Write-Note "Saved to $EnvFile; later runs use it. Delete that line to choose again."
}
function Request-Save([string]$path) {
    if ((Test-Interactive) -and (Confirm-Choice 'Save this location to .env for future runs?' $true)) { Save-Content $path }
}
function Resolve-Content {
    if ($env:MAGICKA_CONTENT) {
        if (-not (Test-Path -LiteralPath $env:MAGICKA_CONTENT -PathType Container)) {
            Write-Note "MAGICKA_CONTENT points to a missing folder: $($env:MAGICKA_CONTENT) (set in the environment or $EnvFile)"
            exit 1
        }
        return $env:MAGICKA_CONTENT
    }
    if (Test-Path -LiteralPath $DefaultContent -PathType Container) { return $DefaultContent }
    $found = @(Find-SteamContent)
    foreach ($path in $found) {
        if (-not (Test-Interactive)) {
            Write-Note "Using Magicka from Steam: $path (run interactively to save it to .env)"
            return $path
        }
        if (Confirm-Choice "Found Magicka in Steam: $path`nUse it?" $true) {
            Request-Save $path
            return $path
        }
    }
    if (Test-Interactive) {
        $why = ''; if ($found.Count -gt 0) { $why = ' (or no Steam install was chosen)' }
        Write-Note "Magicka was not found$why."
        while ($true) {
            [Console]::Error.Write('Path to your Magicka install (or its Content folder), empty to quit: ')
            $answer = Read-Answer
            if (-not $answer.Trim()) { exit 1 }
            $content = Get-ContentOf $answer
            if ($content) { Request-Save $content; return $content }
            Write-Note "No Magicka Content folder (with Levels/) at: $answer"
        }
    }
    Write-Note "Magicka game files not found. Link the install as $(Join-Path $Lab 'Magicka'), set MAGICKA_CONTENT,"
    Write-Note 'or run .\play.ps1 in a terminal to search Steam and choose.'
    exit 1
}

# --- arguments ----------------------------------------------------------------
$Mode = 'offline'; $Address = '127.0.0.1:17700'; $Level = ''; $Extra = @()
$SkipBuild = $false; $PrintContent = $false; $ExportOnly = $false; $Record = $false
$Mods = 'load-order'
$argv = @($args)
for ($i = 0; $i -lt $argv.Count; $i++) {
    $a = [string]$argv[$i]
    switch -Regex ($a) {
        '^--(server|host|join)$' {
            if ($Mode -ne 'offline') { Write-Note 'Choose only one of --server, --host or --join'; exit 2 }
            $Mode = $a.Substring(2)
            if ($i + 1 -lt $argv.Count -and ([string]$argv[$i + 1]).Contains(':')) { $i++; $Address = [string]$argv[$i] }
            elseif ($Mode -eq 'join') { Write-Note '--join requires an IP:port'; exit 2 }
            break
        }
        '^--level$' {
            if ($i + 1 -ge $argv.Count) { Write-Note '--level requires a file'; exit 2 }
            $i++; $Level = [string]$argv[$i]; break
        }
        '^--record$' { $Record = $true; break }
        '^--mods$' {
            if ($i + 1 -ge $argv.Count) { Write-Note '--mods requires a comma-separated list of mod Ids'; exit 2 }
            $i++; $Mods = [string]$argv[$i]; break
        }
        '^--no-mods$' { $Mods = 'none'; break }
        '^--skip-build$' { $SkipBuild = $true; break }
        '^--print-content$' { $PrintContent = $true; break }
        '^--export-only$' { $ExportOnly = $true; break }
        '^(--help|-h)$' {
            @'
Usage:
  .\play.ps1                              main menu
  .\play.ps1 [level.lvl] [viewer options]
  .\play.ps1 --server [127.0.0.1:17700] [--level level.lvl] [--no-scripts]
  .\play.ps1 --host [127.0.0.1:17700] [--level level.lvl] [--debug-menu]
  .\play.ps1 --join 127.0.0.1:17700 [--name Name] [--character Wizard] [--robe-colour R,G,B]
             [--role player|spectator|dm|invader]
  .\play.ps1 --skip-build ...
  .\play.ps1 --export-only                build and refresh the asset cache, then exit
  .\play.ps1 --print-content              find the game files (Steam, .env), print the path, exit
  .\play.ps1 --mods id1,id2 ...           enable these mods instead of load_order.toml
  .\play.ps1 --no-mods ...                play without mods
Hosts (--server/--host) accept --lobby players=8,hotjoin=off,spectators=on,dm=on,invaders=on.
Four players and open hot join are the defaults (Versus locks hot join).
Networking defaults to the authored feature lab. Games are open unless MAGICKA_PASSPHRASE
or --passphrase-file FILE makes them private; see MULTIPLAYER.md.
--server runs without a display and skips all asset exports. --host is a playable host.
The server owns player state, pickups and respawns. Stop a headless server with Ctrl+C.
'@ | Write-Output
            exit 0
        }
        '\.lvl$' {
            if (-not $Level) { $Level = $a } else { $Extra += $a }
            break
        }
        default { $Extra += $a }
    }
}
$StartMenu = ($Mode -eq 'offline') -and (-not $Level) -and (-not $Record)
if (-not $Level) {
    if ($Mode -eq 'offline') { $Level = 'Havindr.lvl' }
    else { $Level = Join-Path $Rewrite 'scenarios/feature-lab.lvl' }
}
$Bin = Join-Path $Rewrite 'target/release'
$ModArgs = @()
switch ($Mods) {
    'none' { }
    'load-order' { $ModArgs = @('--mods', (Join-Path $Rewrite 'mods')) }
    default { $ModArgs = @('--mods', (Join-Path $Rewrite 'mods'), '--enable', $Mods) }
}

$Content = Resolve-Content
if ($PrintContent) { Write-Output $Content; exit 0 }

function Invoke-Native([string]$program, [string[]]$arguments) {
    & $program @arguments
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
$Manifest = Join-Path $Rewrite 'Cargo.toml'

if ($Mode -eq 'server') {
    if (-not $SkipBuild) {
        Invoke-Native 'cargo' @('build', '--locked', '-j1', '--release', '--manifest-path', $Manifest, '-p', 'magicka-runtime', '--bin', 'magicka-server')
    }
    & (Join-Path $Bin "magicka-server$Exe") --content $Content --level $Level --bind $Address @ModArgs @Extra
    exit $LASTEXITCODE
}

# Dialog and character voices are xWMA, decoded by the runtime with FFmpeg.
if (-not $env:MAGICKA_FFMPEG -and -not (Get-Command ffmpeg -ErrorAction SilentlyContinue)) {
    Write-Note 'Voice acting requires FFmpeg on PATH or MAGICKA_FFMPEG=C:\path\to\ffmpeg.exe (for example: winget install ffmpeg).'
}

Write-Output '== building (release) =='
# debug-ui adds the experimental egui developer menu (F1).
if (-not $env:RUST_MIN_STACK) { $env:RUST_MIN_STACK = '33554432' }
if (-not $SkipBuild) {
    Invoke-Native 'cargo' @('build', '--locked', '-j1', '--release', '--manifest-path', $Manifest,
        '-p', 'magicka-viewer', '-p', 'magicka-content', '--features', 'magicka-viewer/debug-ui')
}

# --- asset exports (steps and versions: tools/asset-exports.txt) -------------
$Exports = Join-Path $Cache 'exports'
function Get-LegacyVersion([string]$legacy) {
    if ($legacy.StartsWith('exists:')) {
        if (Test-Path -LiteralPath (Join-Path $Cache $legacy.Substring(7))) { return '1' }
    } elseif ($legacy.StartsWith('stamp:')) {
        $file = Join-Path $Cache $legacy.Substring(6)
        if (Test-Path -LiteralPath $file -PathType Leaf) { return ([IO.File]::ReadAllText($file)).TrimEnd("`r", "`n") }
    }
    ''
}
function Test-ManifestComplete([string]$manifest) {
    $lines = [IO.File]::ReadAllLines($manifest)
    for ($n = 1; $n -lt $lines.Count; $n++) {
        if (-not (Test-Path -LiteralPath $lines[$n])) { return $false }
    }
    $true
}
foreach ($line in [IO.File]::ReadAllLines((Join-Path $Rewrite 'tools/asset-exports.txt'))) {
    $step = @($line.Trim() -split '\s+' | Where-Object { $_ })
    if ($step.Count -eq 0 -or $step[0].StartsWith('#')) { continue }
    $name = $step[0]; $version = $step[1]
    $previous = Get-LegacyVersion $step[2]
    $program = Join-Path $Bin ($step[3] + $Exe)
    $stepArgs = @($step | Select-Object -Skip 4 | ForEach-Object { $_.Replace('{content}', $Content).Replace('{cache}', $Cache) })
    $manifest = Join-Path $Exports "$name.txt"
    if (Test-Path -LiteralPath $manifest -PathType Leaf) {
        $previous = @([IO.File]::ReadAllLines($manifest))[0]
        if ($previous -eq $version -and (Test-ManifestComplete $manifest)) { continue }
    }
    if ($previous -eq $version) {
        Write-Output "== exporting missing $name =="
        $stepArgs += '--missing'
    } else {
        Write-Output "== exporting $name =="
    }
    New-Item -ItemType Directory -Force -Path $Exports | Out-Null
    Invoke-Native $program ($stepArgs + @('--manifest', "$manifest.tmp"))
    [IO.File]::WriteAllText($manifest, "$version`n" + [IO.File]::ReadAllText("$manifest.tmp"), (New-Object Text.UTF8Encoding $false))
    Remove-Item -LiteralPath "$manifest.tmp"
}
# Custom scenery stays in the private cache; preserve hand-edited authored map data.
if ((Split-Path -Leaf $Level) -eq 'confluence.lvl' -and -not (Test-Path -LiteralPath (Join-Path $Cache 'gltf/Custom/Confluence/spruce.glb'))) {
    $python = Get-Command python3, python -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($python) { Invoke-Native $python.Source @((Join-Path $Rewrite 'tools/build-confluence.py'), '--assets-only') }
    else { Write-Note 'The Confluence scenery needs Python 3 (tools/build-confluence.py).' }
}

if ($ExportOnly) { Write-Output '== asset cache up to date =='; exit 0 }

$Viewer = Join-Path $Bin "magicka-viewer$Exe"
if ($Record) {
    $out = Join-Path $Cache 'demo-cast'
    if (Test-Path -LiteralPath $out) { Remove-Item -LiteralPath $out -Recurse -Force }
    Invoke-Native $Viewer @('--play', $Content, '--root', $Cache, '--level', 'Havindr.lvl', '--no-scripts', '--autopilot', $out, '--autopilot-script', 'cast')
    Write-Output "frames: $out/0*.png (06/07 = spraying)"
    exit 0
}
if ($StartMenu) {
    Write-Output '== main menu =='
    & $Viewer --menu $Content --root $Cache @ModArgs @Extra
    exit $LASTEXITCODE
}
switch ($Mode) {
    'host' { $Extra += @('--network-host', $Address) }
    'join' { $Extra += @('--network-join', $Address) }
}
Write-Output "== playing $Level =="
& $Viewer --play $Content --root $Cache --level $Level @ModArgs @Extra
exit $LASTEXITCODE
