# Measures cold start: process creation to a painted, addressable window.
#
#   powershell -ExecutionPolicy Bypass -File tools/cold-start.ps1 -Runs 5
#
# ## What is being measured, and why the number in the log is not it
#
# docs/06 Phase 3 sets a budget of 800ms for cold start. The app logs its own figure --
# "window shown (core side of cold start; excludes WebView paint)" -- and that line says exactly
# what is wrong with quoting it: the window exists, and there is nothing in it yet. The person
# waiting is not waiting for a window handle.
#
# So this waits for the UI to be *there*: the first moment UI Automation can find the toolbar
# inside the WebView. That is later than first paint and earlier than fully synced, and it is the
# closest honest proxy for "the app is up" that can be measured from outside the process.
#
# Each run is cold in the sense that matters for the budget -- a fresh process -- but not in the
# sense of a cold file cache. The first run after a build is the only genuinely cold one, and it
# is reported separately rather than averaged in, because averaging it hides both numbers.

param(
    [int]$Runs = 5,
    [string]$Exe = 'src-tauri\target\release\halcyon.exe',
    [string]$DataDir = ''
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes

if ($DataDir -ne '') { $env:LOCALAPPDATA = $DataDir }

$A = [System.Windows.Automation.AutomationElement]
$T = [System.Windows.Automation.TreeScope]
$true_ = [System.Windows.Automation.Condition]::TrueCondition

function PaintedWindow {
    $windows = $A::RootElement.FindAll($T::Children, $true_)
    foreach ($w in $windows) {
        if ($w.Current.Name -ne 'Halcyon') { continue }
        # A window with no descendants is a frame with an unpainted WebView in it.
        $buttons = $w.FindAll($T::Descendants, $true_) |
            Where-Object { $_.Current.ControlType.ProgrammaticName -like '*Button*' }
        if ($buttons.Count -gt 0) { return $true }
    }
    return $false
}

$results = @()

for ($run = 1; $run -le $Runs; $run++) {
    Get-Process halcyon -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Seconds 3

    $clock = [System.Diagnostics.Stopwatch]::StartNew()
    $p = Start-Process $Exe -PassThru

    $painted = $false
    while ($clock.ElapsedMilliseconds -lt 30000) {
        if (PaintedWindow) { $painted = $true; break }
        Start-Sleep -Milliseconds 25
    }

    $clock.Stop()

    if ($painted) {
        $ms = $clock.ElapsedMilliseconds
        $results += $ms
        Write-Output ("  run {0}: {1} ms" -f $run, $ms)
    } else {
        Write-Output ("  run {0}: never painted within 30s" -f $run)
    }

    Get-Process halcyon -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Seconds 2
}

if ($results.Count -gt 0) {
    $sorted = $results | Sort-Object
    $median = $sorted[[int][Math]::Floor($sorted.Count / 2)]
    Write-Output ''
    Write-Output ("runs      : {0}" -f $results.Count)
    Write-Output ("first     : {0} ms   (the only genuinely cold one)" -f $results[0])
    Write-Output ("median    : {0} ms" -f $median)
    Write-Output ("best      : {0} ms" -f $sorted[0])
    Write-Output ("worst     : {0} ms" -f $sorted[-1])
    Write-Output ("budget    : 800 ms   docs/06 Phase 3")
    Write-Output ("verdict   : {0}" -f $(if ($median -le 800) { 'WITHIN BUDGET' } else { 'OVER BUDGET' }))
}
