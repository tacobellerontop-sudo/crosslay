# Starts the overlay exe, lets it settle, and reports what it costs the system.
# Usage: measure-overlay.ps1 -Exe path\to\crosslay.exe -Label "single exe"
param([Parameter(Mandatory)] [string] $Exe, [string] $Label = $Exe)
$ErrorActionPreference = "Stop"

# All hotkeys disabled so the run doesn't depend on the desktop session.
$cfg = Join-Path $env:RUNNER_TEMP "measure-config.toml"
@"
[hotkeys]
toggle = ""
menu = ""
next = ""
previous = ""
reload = ""
quit = ""
"@ | Set-Content -Encoding utf8 $cfg

$p = Start-Process -FilePath $Exe -ArgumentList "--config", "`"$cfg`"" -PassThru
Start-Sleep -Seconds 5
$p.Refresh()
if ($p.HasExited) { throw "$Label exited early with code $($p.ExitCode)" }
$cpuBefore = $p.TotalProcessorTime
Start-Sleep -Seconds 20
$p.Refresh()
$cpuIdle = ($p.TotalProcessorTime - $cpuBefore).TotalMilliseconds

$modules = $p.Modules | ForEach-Object { $_.ModuleName.ToLower() } | Sort-Object
$result = [ordered]@{
    label             = $Label
    exe_size_kb       = [math]::Round((Get-Item $Exe).Length / 1KB)
    working_set_kb    = [math]::Round($p.WorkingSet64 / 1KB)
    private_bytes_kb  = [math]::Round($p.PrivateMemorySize64 / 1KB)
    peak_working_kb   = [math]::Round($p.PeakWorkingSet64 / 1KB)
    cpu_ms_in_20s     = [math]::Round($cpuIdle, 1)
    threads           = $p.Threads.Count
    handles           = $p.HandleCount
    module_count      = $modules.Count
    modules           = ($modules -join " ")
}
Stop-Process -Id $p.Id -Force
Start-Sleep -Seconds 1
$result | ConvertTo-Json | Tee-Object -FilePath (Join-Path $env:RUNNER_TEMP "measure-$($Label -replace '\W','_').json")
