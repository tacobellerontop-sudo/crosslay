# Bakes a crosshair, starts the overlay, and reports what it costs the system:
#
#   1. idle, with a right-click crosshair set up (so mouse input is registered)
#   2. while the mouse moves (synthetic input), with the right-click crosshair
#   3. while the mouse moves, without any right-click crosshair (the overlay
#      shouldn't be listening to the mouse at all)
#
# It also checks that holding the right button really swaps the crosshair and
# releasing it swaps back.
#
# Usage: measure-overlay.ps1 -Overlay path\to\crosslay.exe -Editor path\to\crosslay-editor.exe
param(
    [Parameter(Mandatory)] [string] $Overlay,
    [Parameter(Mandatory)] [string] $Editor,
    [int] $MouseEvents = 20000
)
$ErrorActionPreference = "Stop"

Add-Type -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public static class Desk {
    [StructLayout(LayoutKind.Sequential)]
    struct MOUSEINPUT { public int dx, dy; public uint mouseData, dwFlags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Sequential)]
    struct INPUT { public uint type; public MOUSEINPUT mi; }

    [DllImport("user32.dll")] static extern uint SendInput(uint n, INPUT[] inputs, int size);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);

    const uint MOVE = 0x1, RIGHTDOWN = 0x8, RIGHTUP = 0x10;

    static uint Send(uint flags, int dx, int count) {
        var batch = new INPUT[count];
        for (int i = 0; i < count; i++) {
            batch[i].mi.dwFlags = flags;
            batch[i].mi.dx = (i % 2 == 0) ? dx : -dx; // wiggle, don't drift
        }
        return SendInput((uint)count, batch, Marshal.SizeOf(typeof(INPUT)));
    }

    // About `total` relative mouse moves in batches, roughly like a mouse
    // polled at 1000 Hz. Returns how many Windows accepted.
    public static long Moves(int total, int perBatch) {
        long sent = 0;
        for (int done = 0; done < total; done += perBatch) {
            sent += Send(MOVE, 1, Math.Min(perBatch, total - done));
            Thread.Sleep(1);
        }
        return sent;
    }

    public static uint RightDown() { return Send(RIGHTDOWN, 0, 1); }
    public static uint RightUp() { return Send(RIGHTUP, 0, 1); }

    // The overlay's windows that are visible right now.
    public static IntPtr[] VisibleOverlays(int pid) {
        var found = new List<IntPtr>();
        EnumWindows((h, l) => {
            uint p; GetWindowThreadProcessId(h, out p);
            var name = new StringBuilder(64);
            GetClassName(h, name, 64);
            if (p == pid && name.ToString() == "crosslay_overlay" && IsWindowVisible(h)) found.Add(h);
            return true;
        }, IntPtr.Zero);
        return found.ToArray();
    }
}
"@

# A private data folder with every hotkey and the update check turned off, so
# the run doesn't depend on (or disturb) the desktop session or the network.
$dir = Join-Path $env:RUNNER_TEMP "crosslay-measure"
New-Item -ItemType Directory -Force -Path $dir | Out-Null
$base = @"
auto_update = false
right_click = "hold"

[hotkeys]
toggle = ""
editor = ""
next = ""
previous = ""
quit = ""

[[crosshair]]
name = "Measured"
"@
$rightClick = @"

[crosshair.right_click]
center_dot = true
center_dot_thickness = 3

[crosshair.right_click.inner_lines]
show = false

[crosshair.right_click.outer_lines]
show = false
"@

function Bake([string] $text) {
    $text | Set-Content -Encoding utf8 (Join-Path $dir "config.toml")
    # The editor is a GUI-subsystem exe, so `&` wouldn't wait for it.
    $bake = Start-Process -FilePath $Editor -ArgumentList "--dir", "`"$dir`"", "--bake" -NoNewWindow -Wait -PassThru
    if ($bake.ExitCode -ne 0) { throw "baking failed with code $($bake.ExitCode)" }
    if (-not (Test-Path (Join-Path $dir "overlay.bin"))) { throw "baking wrote no overlay.bin" }
}

function CpuMs($p) { $p.Refresh(); $p.TotalProcessorTime.TotalMilliseconds }

# CPU time the overlay uses while `MouseEvents` mouse moves go by.
function MeasureMoves($p) {
    $before = CpuMs $p
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $sent = [Desk]::Moves($MouseEvents, 8)
    Start-Sleep -Milliseconds 500 # let the overlay drain its queue
    $cpu = (CpuMs $p) - $before
    [ordered]@{
        sent    = $sent
        seconds = [math]::Round($watch.Elapsed.TotalSeconds, 1)
        cpu_ms  = [math]::Round($cpu, 1)
        us_each = if ($sent) { [math]::Round($cpu * 1000 / $sent, 2) } else { $null }
    }
}

Bake ($base + $rightClick)
$p = Start-Process -FilePath $Overlay -ArgumentList "--dir", "`"$dir`"" -PassThru
Start-Sleep -Seconds 5
$p.Refresh()
if ($p.HasExited) { throw "overlay exited early with code $($p.ExitCode)" }

# 1. Idle.
$cpuBefore = CpuMs $p
Start-Sleep -Seconds 20
$cpuIdle = (CpuMs $p) - $cpuBefore
$p.Refresh()
$idle = [ordered]@{
    working_set_kb   = [math]::Round($p.WorkingSet64 / 1KB)
    private_bytes_kb = [math]::Round($p.PrivateMemorySize64 / 1KB)
    peak_working_kb  = [math]::Round($p.PeakWorkingSet64 / 1KB)
    threads          = $p.Threads.Count
    handles          = $p.HandleCount
}

# Right button: the shown window must change, then change back.
$normal = [Desk]::VisibleOverlays($p.Id)
if ([Desk]::RightDown() -eq 0) { throw "SendInput was refused; no interactive desktop?" }
Start-Sleep -Milliseconds 300
$held = [Desk]::VisibleOverlays($p.Id)
[void][Desk]::RightUp()
Start-Sleep -Milliseconds 300
$released = [Desk]::VisibleOverlays($p.Id)
$swapOk = $normal.Count -eq 1 -and $held.Count -eq 1 -and $released.Count -eq 1 -and
    $held[0] -ne $normal[0] -and $released[0] -eq $normal[0]
Write-Host "visible overlay window: normal $normal, right held $held, released $released"

# 2. Mouse moving, right-click crosshair set up.
$withRc = MeasureMoves $p

# 3. Mouse moving, no right-click crosshair: the overlay reloads the file and
#    stops listening to the mouse.
Bake $base
Start-Sleep -Seconds 2
$withoutRc = MeasureMoves $p

$p.Refresh()
$modules = $p.Modules | ForEach-Object { $_.ModuleName.ToLower() } | Sort-Object
$result = [ordered]@{
    exe_size_kb                 = [math]::Round((Get-Item $Overlay).Length / 1KB)
    working_set_kb              = $idle.working_set_kb
    private_bytes_kb            = $idle.private_bytes_kb
    peak_working_kb             = $idle.peak_working_kb
    cpu_ms_in_20s_idle          = [math]::Round($cpuIdle, 1)
    right_click_swap_works      = $swapOk
    mouse_moves_sent            = $withRc.sent
    cpu_ms_moves_right_click    = $withRc.cpu_ms
    us_per_move_right_click     = $withRc.us_each
    core_pct_at_1000hz          = if ($null -ne $withRc.us_each) { [math]::Round($withRc.us_each / 10, 3) } else { $null }
    cpu_ms_moves_no_right_click = $withoutRc.cpu_ms
    threads                     = $idle.threads
    handles                     = $idle.handles
    module_count                = $modules.Count
    modules                     = ($modules -join " ")
}
Stop-Process -Id $p.Id -Force
$result | ConvertTo-Json

# Run summary.
$md = "### Overlay cost`n`n| Metric | Value |`n|---|---|`n"
foreach ($k in $result.Keys | Where-Object { $_ -ne "modules" }) { $md += "| $k | $($result[$k]) |`n" }
$md += "`nMouse moves: $($withRc.sent) in $($withRc.seconds) s with a right-click crosshair, " +
    "$($withoutRc.sent) in $($withoutRc.seconds) s without. core_pct_at_1000hz is the share of one CPU core " +
    "a 1000 Hz mouse would cost while it moves.`n"
$md += "`n**Loaded modules:** $($result.modules)`n"
if ($env:GITHUB_STEP_SUMMARY) { $md | Out-File -Append -Encoding utf8 $env:GITHUB_STEP_SUMMARY }

if (-not $swapOk) { throw "holding the right button didn't swap to the right-click crosshair and back" }
