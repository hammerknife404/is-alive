<#
.SYNOPSIS
  Times is-alive's Map on this machine: frame time, CPU and memory (#146).

.DESCRIPTION
  Runs is-alive's performance probe (IS_ALIVE_PERF) once with each renderer,
  OpenGL then Direct3D 12. Each run opens the Contoso example map at
  1280 x 720 and measures three phases of 15 seconds: the map left alone,
  packets flowing, and the 3D view spinning. About a minute per renderer.

  It runs a portable copy of is-alive in a temporary folder, so your
  settings, sites and notes aren't touched, and an is-alive already open
  can stay open. Nothing is sent anywhere: the example map is built in and
  no tool runs.

  Leave the window in front and don't touch the mouse or keyboard while it
  runs: the map stops its packets when the window loses focus, and the
  report flags a phase where that happened.

  Writes perf-report.md (and each run's JSON) to a folder on the Desktop,
  copies the report to the clipboard, and opens the folder. Paste the
  report into issue #146.

.PARAMETER Exe
  is-alive.exe to time. Default: the one beside this script, else the one
  in the current folder. Needs a build with the probe (0.36.0 has none:
  use the next release or a pull request's is-alive-windows-x64 artifact).

.PARAMETER Label
  A word for this run in the report and folder name, such as "battery".

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File perf.ps1
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File perf.ps1 -Exe C:\Tools\is-alive.exe -Label battery
#>
[CmdletBinding()]
param(
  [string]$Exe,
  [string]$Label = 'plugged-in',
  [string[]]$Renderers = @('glow', 'wgpu'),
  [string]$OutDir,
  # Only build the report from JSON files already in -OutDir.
  [switch]$ReportOnly
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2

function Fmt($v, [string]$unit = '') {
  if ($null -eq $v) { return '-' }
  return ('{0:0.#}{1}' -f [double]$v, $unit)
}

function Get-Field($obj, [string]$name) {
  if ($null -eq $obj) { return $null }
  $p = $obj.PSObject.Properties[$name]
  if ($p) { return $p.Value }
  return $null
}

function Get-Machine {
  $m = [ordered]@{}
  try {
    $os = Get-CimInstance Win32_OperatingSystem
    $m['Windows'] = "$($os.Caption) $($os.Version)"
    $cpu = @(Get-CimInstance Win32_Processor)[0]
    $m['CPU'] = "$($cpu.Name.Trim()) ($($cpu.NumberOfCores) cores, $($cpu.NumberOfLogicalProcessors) threads)"
    $cs = Get-CimInstance Win32_ComputerSystem
    $m['Memory'] = '{0:0} GB' -f ($cs.TotalPhysicalMemory / 1GB)
    $m['Model'] = "$($cs.Manufacturer) $($cs.Model)"
    $i = 0
    foreach ($g in Get-CimInstance Win32_VideoController) {
      $i++
      $mode = ''
      if ($g.CurrentHorizontalResolution) {
        $mode = ", $($g.CurrentHorizontalResolution) x $($g.CurrentVerticalResolution) at $($g.CurrentRefreshRate) Hz"
      }
      $m["GPU $i"] = "$($g.Name), driver $($g.DriverVersion)$mode"
    }
    $bat = @(Get-CimInstance Win32_Battery)
    if ($bat.Count -eq 0) {
      $m['Power'] = 'no battery'
    } elseif ($bat[0].BatteryStatus -eq 1) {
      $m['Power'] = "on battery ($($bat[0].EstimatedChargeRemaining)%)"
    } else {
      $m['Power'] = "plugged in ($($bat[0].EstimatedChargeRemaining)%)"
    }
    $plan = (powercfg /getactivescheme) -replace '^.*\((.*)\).*$', '$1'
    if ($plan) { $m['Power plan'] = $plan }
  } catch {
    $m['Note'] = "machine details incomplete: $($_.Exception.Message)"
  }
  return $m
}

function Write-Report([string]$dir, $machine, [string]$exePath) {
  $runs = @(Get-ChildItem -Path $dir -Filter 'perf-*.json' | Sort-Object Name)
  $lines = New-Object System.Collections.Generic.List[string]
  $lines.Add("## is-alive performance: $Label")
  $lines.Add('')
  $lines.Add("Measured $(Get-Date -Format 'yyyy-MM-dd HH:mm') with ``scripts/perf.ps1``.")
  $lines.Add('')
  $lines.Add('| | |')
  $lines.Add('|---|---|')
  foreach ($k in $machine.Keys) { $lines.Add("| $k | $($machine[$k]) |") }
  if ($exePath -and (Test-Path $exePath)) {
    $f = Get-Item $exePath
    $lines.Add(('| is-alive.exe | {0:0.00} MB |' -f ($f.Length / 1MB)))
  }
  $warn = New-Object System.Collections.Generic.List[string]
  foreach ($r in $runs) {
    $d = Get-Content $r.FullName -Raw | ConvertFrom-Json
    $lines.Add('')
    $lines.Add("### $(Get-Field $d 'renderer')")
    $lines.Add('')
    $win = Get-Field $d 'window_points'
    $mon = Get-Field $d 'monitor_points'
    $winText = '-'
    if ($win) { $winText = '{0:0} x {1:0}' -f $win[0], $win[1] }
    $monText = '-'
    if ($mon) { $monText = '{0:0} x {1:0}' -f $mon[0], $mon[1] }
    $lines.Add("is-alive $(Get-Field $d 'version'), window $winText points at $(Fmt (Get-Field $d 'pixels_per_point') 'x') scale (screen $monText points), $(Get-Field $d 'map') ($(Get-Field $d 'map_cards') cards), $(Get-Field $d 'measure_secs') s a phase. On battery: $(Get-Field $d 'on_battery').")
    $err = Get-Field $d 'error'
    if ($err) {
      $lines.Add('')
      $lines.Add("**Didn't finish:** $err")
      $warn.Add("$($r.Name): $err")
    }
    $lines.Add('')
    $lines.Add('| Phase | fps | Frame gap p50 / p95 / p99 / max ms | CPU a frame p50 / p95 / p99 / max ms | Hitches | Process CPU |')
    $lines.Add('|---|---|---|---|---|---|')
    foreach ($p in @(Get-Field $d 'phases')) {
      $g = Get-Field $p 'gap_ms'
      $c = Get-Field $p 'cpu_ms'
      $gap = '-'
      if ($g) { $gap = '{0} / {1} / {2} / {3}' -f (Fmt $g.p50), (Fmt $g.p95), (Fmt $g.p99), (Fmt $g.max) }
      $cpu = '-'
      if ($c) { $cpu = '{0} / {1} / {2} / {3}' -f (Fmt $c.p50), (Fmt $c.p95), (Fmt $c.p99), (Fmt $c.max) }
      $name = Get-Field $p 'name'
      $lines.Add("| $name | $(Fmt (Get-Field $p 'fps')) | $gap | $cpu | $(Get-Field $p 'hitches') | $(Fmt (Get-Field $p 'process_cpu_percent') '%') |")
      if ((Get-Field $p 'unfocused_frames') -gt 0) {
        $warn.Add("$($r.Name) ${name}: the window wasn't in front for $(Get-Field $p 'unfocused_frames') frames; packets stop then, so run it again")
      }
      if ((Get-Field $p 'touched_frames') -gt 0) {
        $warn.Add("$($r.Name) ${name}: mouse or keys used during $(Get-Field $p 'touched_frames') frames")
      }
    }
    $lines.Add('')
    $lines.Add('| Memory at | Working set | Peak working set | Private |')
    $lines.Add('|---|---|---|---|')
    foreach ($m in @(Get-Field $d 'memory')) {
      $mm = Get-Field $m 'memory'
      $lines.Add("| $(Get-Field $m 'at') | $(Fmt (Get-Field $mm 'resident_mb') ' MB') | $(Fmt (Get-Field $mm 'peak_resident_mb') ' MB') | $(Fmt (Get-Field $mm 'private_mb') ' MB') |")
    }
  }
  if ($runs.Count -eq 0) {
    $lines.Add('')
    $lines.Add('**No results.** Either this is-alive has no performance probe (releases up to 0.36.0 have none), or the run failed: see the `startup` logs in this folder.')
    $warn.Add('no results')
  }
  $lines.Add('')
  $lines.Add('Frame gap: time between frames on screen (16.7 ms is 60 Hz). CPU a frame: eframe''s CPU time for drawing it, without waiting for the screen. A hitch: a gap over twice the phase''s typical one. Process CPU: all of is-alive''s threads, 100% = one core. Graphics memory on a dedicated GPU isn''t in the memory figures.')
  if ($warn.Count -gt 0) {
    $lines.Add('')
    $lines.Add('**Check before trusting these numbers:**')
    foreach ($w in $warn) { $lines.Add("- $w") }
  }
  $report = Join-Path $dir 'perf-report.md'
  [IO.File]::WriteAllLines($report, $lines)
  return @{ Path = $report; Warnings = $warn }
}

# --- Where things are -------------------------------------------------------

$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
if (-not $OutDir) {
  $desktop = [Environment]::GetFolderPath('Desktop')
  if (-not $desktop) { $desktop = (Get-Location).Path }
  $OutDir = Join-Path $desktop "is-alive-perf-$Label-$stamp"
}
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
# Absolute: is-alive runs from its temporary folder, and would read a
# relative path from there.
$OutDir = (Resolve-Path $OutDir).Path

if (-not $Exe) {
  foreach ($c in @((Join-Path $PSScriptRoot 'is-alive.exe'), (Join-Path (Get-Location) 'is-alive.exe'))) {
    if (Test-Path $c) { $Exe = $c; break }
  }
}

$machine = Get-Machine

if (-not $ReportOnly) {
  if (-not $Exe -or -not (Test-Path $Exe)) {
    throw 'is-alive.exe not found: put this script beside it, or pass -Exe <path>.'
  }
  $Exe = (Resolve-Path $Exe).Path
  # A portable copy: the folder is-alive-data beside the .exe holds all it
  # keeps, so the user's own settings and sites stay as they are.
  $app = Join-Path ([IO.Path]::GetTempPath()) "is-alive-perf-$stamp"
  New-Item -ItemType Directory -Force -Path (Join-Path $app 'is-alive-data') | Out-Null
  Copy-Item $Exe (Join-Path $app 'is-alive.exe')
  $runExe = Join-Path $app 'is-alive.exe'

  Add-Type -Namespace IsAlivePerf -Name Win -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetForegroundWindow(System.IntPtr hWnd);
'@

  foreach ($r in $Renderers) {
    $json = Join-Path $OutDir "perf-$r.json"
    Remove-Item $json -ErrorAction SilentlyContinue
    Write-Host "Timing with --renderer=$r (about a minute; leave the window in front)..."
    $env:IS_ALIVE_PERF = $json
    try {
      $proc = Start-Process -FilePath $runExe -ArgumentList "--renderer=$r" -WorkingDirectory $app -PassThru
    } finally {
      Remove-Item Env:IS_ALIVE_PERF
    }
    # Bring the window in front once it's there (Windows may refuse; the
    # report says if it wasn't).
    for ($i = 0; $i -lt 40 -and -not $proc.HasExited; $i++) {
      $proc.Refresh()
      if ($proc.MainWindowHandle -ne [IntPtr]::Zero) {
        [IsAlivePerf.Win]::SetForegroundWindow($proc.MainWindowHandle) | Out-Null
        break
      }
      Start-Sleep -Milliseconds 250
    }
    if (-not $proc.WaitForExit(300000)) {
      Write-Warning "$r run didn't finish in 5 minutes; stopping it."
      $proc.Kill()
    }
    if (-not (Test-Path $json)) {
      $log = Join-Path $app 'is-alive-data\startup.log'
      Write-Warning "$r wrote no results (exit code $($proc.ExitCode))."
      if (Test-Path $log) { Copy-Item $log (Join-Path $OutDir "startup-$r.log") }
    }
  }
  Copy-Item (Join-Path $app 'is-alive-data\startup.log') (Join-Path $OutDir 'startup.log') -ErrorAction SilentlyContinue
  Remove-Item -Recurse -Force $app -ErrorAction SilentlyContinue
}

$result = Write-Report $OutDir $machine $Exe
Write-Host ''
Get-Content $result.Path | Write-Host
try { Get-Content $result.Path -Raw | Set-Clipboard; Write-Host "`nCopied to the clipboard: paste it into issue #146." } catch { }
Write-Host "Saved in $OutDir"
if ($result.Warnings.Count -gt 0) { Write-Warning 'Some numbers need a second look: see the end of the report.' }
if ($env:OS -eq 'Windows_NT' -and -not $env:CI) { Invoke-Item $OutDir }
