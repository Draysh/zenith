# Try the freshly built Zenith out on this Windows machine, the way a
# person's computer would run it, and write down everything that happened
# in out\smoke (uploaded by the build, to be read afterwards).
#
#   1. install the setup program silently, as an update does
#   1b. update over that install the way the app does it: a copy of the
#      recorder program, run from another folder, shows Zenith's own
#      window and runs the setup program unseen (`zenithd update`)
#   2. ask the recorder what this machine offers, and have it test itself
#   3. record and save a clip from a made-up picture (the buffer, saving
#      and the sound tracks, with nothing depending on this machine)
#   4. record and save a clip of the real screen
#   5. run the background recorder and ask it for a clip
#
# Steps 1b and 3 have to work everywhere; the build machines have no gaming
# graphics card and may have no screen or sound to capture. The others
# are reported, and fail nothing.
#
# Nothing here waits without a limit: a program that hangs is ended and
# said to have hung, so the build never sits on it.
$ErrorActionPreference = "Continue"
Set-Location (Join-Path $PSScriptRoot "..\..")
$out = Join-Path (Get-Location).Path "out\smoke"
New-Item -ItemType Directory -Force $out | Out-Null
$report = Join-Path $out "report.txt"
function Say($text) { $text | Tee-Object -FilePath $report -Append | Out-Host }

# Run a program for at most so many seconds; say what it printed and how
# it ended. Only the program itself is waited for, not what it starts.
$script:step = 0
function Run($file, $arguments, $seconds) {
    $script:step += 1
    $stdout = Join-Path $out "run-$($script:step)-out.txt"
    $stderr = Join-Path $out "run-$($script:step)-err.txt"
    Say "> $(Split-Path $file -Leaf) $arguments"
    $given = @{ FilePath = $file; PassThru = $true; WindowStyle = "Hidden"; RedirectStandardOutput = $stdout; RedirectStandardError = $stderr }
    if ($arguments) { $given.ArgumentList = $arguments }
    $run = Start-Process @given
    $handle = $run.Handle
    if (-not $run.WaitForExit($seconds * 1000)) {
        Say "  HUNG: still running after $seconds s; ending it"
        try { $run.Kill() } catch {}
        $code = -1
    } else {
        $code = $run.ExitCode
    }
    foreach ($file in $stdout, $stderr) {
        if (Test-Path $file) { Get-Content $file -ErrorAction SilentlyContinue | Select-Object -First 200 | ForEach-Object { Say "  $_" } }
    }
    Say "  exit code: $code"
    return $code
}

function Stop-Zenith {
    Get-Process zenith-clips, zenithd, zenith-rec, ffmpeg -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
}

# --- 1. install ------------------------------------------------------------
$setup = Get-ChildItem out\*-setup.exe | Select-Object -First 1
Say "== install $($setup.Name)"
$null = Run $setup.FullName "/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /LOG=`"$out\install.log`"" 300
$app = Join-Path $env:LOCALAPPDATA "Programs\Zenith"
Get-ChildItem $app -File -ErrorAction SilentlyContinue | Select-Object Name, Length | Format-Table -AutoSize | Out-String | Add-Content $report
# The installer opens the app and starts the recorder; say whether it did,
# then end them: the steps below start their own.
Start-Sleep -Seconds 5
Say "running after the install: $((Get-Process zenith-clips, zenithd, zenith-rec -ErrorAction SilentlyContinue | ForEach-Object { $_.ProcessName }) -join ', ')"
Stop-Zenith
if (-not (Test-Path "$app\zenith-rec.exe")) { Say "RESULT: the recorder was not installed"; exit 1 }

# --- 1b. an update, as the app runs one ------------------------------------
# The helper deletes the setup program it was given once it has worked, so
# it gets a copy; and it is itself a copy, in the folder the app keeps a
# downloaded update in, with nothing of Zenith's beside it.
Say "== update over the install"
$updateDir = Join-Path $env:LOCALAPPDATA "Zenith\update"
New-Item -ItemType Directory -Force $updateDir | Out-Null
$helper = Join-Path $updateDir "zenith-update.exe"
$again = Join-Path $updateDir $setup.Name
Copy-Item "$app\zenithd.exe" $helper -Force
Copy-Item $setup.FullName $again -Force
$version = if ($setup.Name -match "^Zenith-(.+?)-windows") { $Matches[1] } else { "" }
$code = Run $helper "update `"$again`" `"$version`" `"$app\zenith-clips.exe`"" 300
Start-Sleep -Seconds 5
$opened = @(Get-Process zenith-clips -ErrorAction SilentlyContinue).Count -gt 0
Say "the app is open after the update: $opened; the setup program was cleared away: $(-not (Test-Path $again))"
$updateLog = Join-Path $env:LOCALAPPDATA "Zenith\logs\update.log"
if (Test-Path $updateLog) { Get-Content $updateLog | ForEach-Object { Say "  $_" } } else { Say "  no update.log was written" }
$updated = ($code -eq 0) -and (Test-Path "$app\zenith-clips.exe") -and (Test-Path $updateLog)
Say "RESULT update behind Zenith's window: $updated"
Stop-Zenith

# --- 2. what this machine offers ------------------------------------------
foreach ($ask in "--info", "--list-capture-options", "--list-audio-devices") {
    Say "== zenith-rec $ask"
    $null = Run "$app\zenith-rec.exe" $ask 60
}
Say "== zenith-rec --diagnose"
$null = Run "$app\zenith-rec.exe" "--diagnose" 240
Stop-Zenith

# Record for a while, ask for a clip, and say what the clip holds.
function Record($name, $seconds, $sound) {
    Say "== record: $name"
    $clips = Join-Path $out "clips-$name"
    New-Item -ItemType Directory -Force $clips | Out-Null
    $start = New-Object System.Diagnostics.ProcessStartInfo
    $start.FileName = "$app\zenith-rec.exe"
    $start.Arguments = "-w screen -c mp4 -f 30 -k h264 -bm cbr -q 4000 -r 20 -replay-storage ram -keyint 1 -ac aac -cursor yes $sound -v no -o `"$clips`""
    $start.RedirectStandardInput = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    Say "> zenith-rec $($start.Arguments)"
    $rec = [System.Diagnostics.Process]::Start($start)
    $errors = $rec.StandardError.ReadToEndAsync()
    $ok = $false
    if ($rec.WaitForExit($seconds * 1000)) {
        Say "  the recorder stopped by itself, exit code $($rec.ExitCode)"
    } else {
        $rec.StandardInput.WriteLine("save")
        $rec.StandardInput.Flush()
        $line = $rec.StandardOutput.ReadLineAsync()
        $saved = if ($line.Wait(60000)) { $line.Result } else { "" }
        Say "  saved: $saved"
        try { $rec.StandardInput.WriteLine("stop"); $rec.StandardInput.Close() } catch {}
        if (-not $rec.WaitForExit(20000)) { Say "  HUNG: it did not stop when asked; ending it"; try { $rec.Kill() } catch {} }
        else { Say "  recorder exit code: $($rec.ExitCode)" }
        if ($saved -and (Test-Path $saved)) {
            $probe = & "$app\ffprobe.exe" -v error -show_entries "stream=index,codec_type,codec_name,width,height,channels:format=duration,size" -of default=noprint_wrappers=1 $saved 2>&1
            $probe | ForEach-Object { Say "    $_" }
            $video = @($probe | Select-String "codec_type=video").Count
            $audio = @($probe | Select-String "codec_type=audio").Count
            $wanted = @($sound -split " " | Where-Object { $_ -eq "-a" }).Count
            Say "  streams: $video video, $audio audio (wanted 1 and $wanted)"
            $ok = ($video -eq 1 -and $audio -eq $wanted)
        } else {
            Say "  no clip was written"
        }
    }
    if ($errors.Wait(5000)) { $errors.Result | Set-Content (Join-Path $out "rec-$name-stderr.txt") }
    Stop-Zenith
    return $ok
}

# --- 3. a made-up picture --------------------------------------------------
$env:ZENITH_REC_TEST_SOURCE = "1"
$env:ZENITH_REC_TEST_TONE = "1"
$pipeline = Record "test-picture" 10 "-a default_output -a default_input"
Remove-Item Env:ZENITH_REC_TEST_SOURCE
Remove-Item Env:ZENITH_REC_TEST_TONE
Say "RESULT test picture: $pipeline"

# --- 4. the real screen, first without sound, then with -------------------
$screen = Record "screen" 12 ""
Say "RESULT real screen, no sound: $screen"
$screenSound = Record "screen-sound" 12 "-a default_output -a default_input"
Say "RESULT real screen with sound: $screenSound"

# --- 5. the background recorder -------------------------------------------
Say "== background recorder"
$config = Join-Path $env:APPDATA "Zenith"
New-Item -ItemType Directory -Force $config | Out-Null
"[capture]`nactivation = `"always`"`nbuffer_secs = 20`n`n[audio]`ngame = false`nchat = false`nmicrophone = false`n" | Set-Content (Join-Path $config "config.toml")
$daemon = Start-Process "$app\zenithd.exe" -WindowStyle Hidden -PassThru -RedirectStandardError (Join-Path $out "zenithd-stderr.txt") -RedirectStandardOutput (Join-Path $out "zenithd-stdout.txt")
Start-Sleep -Seconds 15
$null = Run "$app\zenithd.exe" "status" 20
$null = Run "$app\zenithd.exe" "doctor" 90
Say "-- save"
$daemonSaved = ((Run "$app\zenithd.exe" "save" 90) -eq 0)
$null = Run "$app\zenithd.exe" "status" 20
Get-ChildItem (Join-Path $env:USERPROFILE "Videos\Clips") -Recurse -File -ErrorAction SilentlyContinue | Select-Object FullName, Length | Format-Table -AutoSize | Out-String | Add-Content $report
Say "-- report"
$null = Run "$app\zenithd.exe" "report" 90
$null = Run "$app\zenithd.exe" "quit" 30
Start-Sleep -Seconds 3
if (-not $daemon.HasExited) { Say "the background recorder did not quit when asked; ending it" }
Stop-Zenith
Say "RESULT background recorder save: $daemonSaved"

# --- the logs --------------------------------------------------------------
$logs = Join-Path $env:LOCALAPPDATA "Zenith\logs"
if (Test-Path $logs) { Copy-Item $logs (Join-Path $out "logs") -Recurse -Force }

Say "== summary"
Say "update: $updated   test picture: $pipeline   real screen: $screen   real screen with sound: $screenSound   background recorder: $daemonSaved"
if (-not $pipeline -or -not $updated) { exit 1 }
exit 0
