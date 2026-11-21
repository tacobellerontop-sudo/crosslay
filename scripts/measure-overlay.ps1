# Bakes a crosshair, starts the overlay, lets it sit idle, and reports what it
# costs the system.
# Usage: measure-overlay.ps1 -Overlay path\to\crosslay.exe -Editor path\to\crosslay-editor.exe
param(
    [Parameter(Mandatory)] [string] $Overlay,
    [Parameter(Mandatory)] [string] $Editor
)
$ErrorActionPreference = "Stop"

# A private data folder with every hotkey and the update check turned off, so
# the run doesn't depend on (or disturb) the desktop session or the network.
$dir = Join-Path $env:RUNNER_TEMP "crosslay-measure"
New-Item -ItemType Directory -Force -Path $dir | Out-Null
@"
auto_update = false

[hotkeys]
toggle = ""
editor = ""
next = ""
previous = ""
quit = ""
"@ | Set-Content -Encoding utf8 (Join-Path $dir "config.toml")
# The editor is a GUI-subsystem exe, so `&` wouldn't wait for it.
$bake = Start-Process -FilePath $Editor -ArgumentList "--dir", "`"$dir`"", "--bake" -NoNewWindow -Wait -PassThru
if ($bake.ExitCode -ne 0) { throw "baking failed with code $($bake.ExitCode)" }
if (-not (Test-Path (Join-Path $dir "overlay.bin"))) { throw "baking wrote no overlay.bin" }

$p = Start-Process -FilePath $Overlay -ArgumentList "--dir", "`"$dir`"" -PassThru
Start-Sleep -Seconds 5
$p.Refresh()
if ($p.HasExited) { throw "overlay exited early with code $($p.ExitCode)" }
$cpuBefore = $p.TotalProcessorTime
Start-Sleep -Seconds 20
$p.Refresh()
$cpuIdle = ($p.TotalProcessorTime - $cpuBefore).TotalMilliseconds

$modules = $p.Modules | ForEach-Object { $_.ModuleName.ToLower() } | Sort-Object
$result = [ordered]@{
    exe_size_kb      = [math]::Round((Get-Item $Overlay).Length / 1KB)
    working_set_kb   = [math]::Round($p.WorkingSet64 / 1KB)
    private_bytes_kb = [math]::Round($p.PrivateMemorySize64 / 1KB)
    peak_working_kb  = [math]::Round($p.PeakWorkingSet64 / 1KB)
    cpu_ms_in_20s    = [math]::Round($cpuIdle, 1)
    threads          = $p.Threads.Count
    handles          = $p.HandleCount
    module_count     = $modules.Count
    modules          = ($modules -join " ")
}
Stop-Process -Id $p.Id -Force
$result | ConvertTo-Json

# Run summary.
$md = "### Idle overlay`n`n| Metric | Value |`n|---|---|`n"
foreach ($k in $result.Keys | Where-Object { $_ -ne "modules" }) { $md += "| $k | $($result[$k]) |`n" }
$md += "`n**Loaded modules:** $($result.modules)`n"
if ($env:GITHUB_STEP_SUMMARY) { $md | Out-File -Append -Encoding utf8 $env:GITHUB_STEP_SUMMARY }
