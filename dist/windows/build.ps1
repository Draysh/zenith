# Build Zenith for Windows and its setup program:
#   out\Zenith-<version>-windows-x86_64-setup.exe
#
# Needs, besides Rust and Qt 6 (qmake and windeployqt on the PATH):
#   MPV_DIR     a libmpv development folder: include\mpv\*.h, mpv.lib, libmpv-2.dll
#   FFMPEG_DIR  an ffmpeg build: bin\ffmpeg.exe, bin\ffprobe.exe and their DLLs
#   ISCC        Inno Setup's compiler (default: its usual place)
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..\..")

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.*)"').Matches[0].Groups[1].Value
$root = (Get-Location).Path
$out = Join-Path $root "out"
$stage = Join-Path $out "stage"
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force $stage | Out-Null

cargo build --release --locked -p zenith -p zenith-daemon -p zenith-rec --features zenith/bundled-sqlite
if ($LASTEXITCODE -ne 0) { throw "the build failed" }
# The window, the background recorder, and the program that does the recording.
Copy-Item target\release\zenith-clips.exe, target\release\zenithd.exe, target\release\zenith-rec.exe $stage

# Qt's own files: its libraries, plugins and the QML modules the screens use.
windeployqt --release --no-translations --qmldir app\qml (Join-Path $stage "zenith-clips.exe")
if ($LASTEXITCODE -ne 0) { throw "windeployqt failed" }

# The video player's library, and ffmpeg for thumbnails, export and probing.
Copy-Item (Join-Path $env:MPV_DIR "libmpv-2.dll") $stage
Copy-Item (Join-Path $env:FFMPEG_DIR "bin\ffmpeg.exe"), (Join-Path $env:FFMPEG_DIR "bin\ffprobe.exe") $stage
Copy-Item (Join-Path $env:FFMPEG_DIR "bin\*.dll") $stage

# The recorder captures through these two; an ffmpeg without them would
# install and then record by the slowest way, or not the game's window.
$filters = & (Join-Path $stage "ffmpeg.exe") -hide_banner -filters 2>$null | Out-String
foreach ($filter in "ddagrab", "gfxcapture") {
    if ($filters -notmatch "\s$filter\s") { throw "this ffmpeg has no $filter filter: the recorder needs it" }
}

$iscc = if ($env:ISCC) { $env:ISCC } else { "C:\Program Files (x86)\Inno Setup 6\ISCC.exe" }
& $iscc "/DAppVersion=$version" "/DStage=$stage" "/DOutDir=$out" dist\windows\zenith.iss
if ($LASTEXITCODE -ne 0) { throw "the setup program could not be made" }
Write-Output (Join-Path $out "Zenith-$version-windows-x86_64-setup.exe")
