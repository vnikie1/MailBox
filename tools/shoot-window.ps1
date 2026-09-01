# Captures the Halcyon window to a PNG.
#
#   powershell -ExecutionPolicy Bypass -File tools/shoot-window.ps1 -Out shot.png
#
# Used for the Phase 6 newsletter check, where the evidence has to be something a person can
# look at. A description of a rendering is not a rendering.

param(
    [string]$Out = 'window.png',
    [string]$Title = 'Halcyon'
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms

$sig = @'
using System;
using System.Runtime.InteropServices;
public class W {
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string c, string n);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int attr, out RECT r, int size);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
'@
Add-Type -TypeDefinition $sig

# Found through UI Automation rather than FindWindow. A Tauri window's caption is set by the
# webview after it loads, and FindWindow matches the caption the shell knows about, which is not
# reliably the same string -- it returned nothing for a window plainly titled "Halcyon".
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$A = [System.Windows.Automation.AutomationElement]
$T = [System.Windows.Automation.TreeScope]
$element = $A::RootElement.FindAll($T::Children, [System.Windows.Automation.Condition]::TrueCondition) |
    Where-Object { $_.Current.Name -eq $Title } | Select-Object -First 1

if (-not $element) { Write-Output "no window titled '$Title'"; exit 1 }
$handle = [IntPtr]$element.Current.NativeWindowHandle
if ($handle -eq [IntPtr]::Zero) { Write-Output "'$Title' has no native handle"; exit 1 }

[void][W]::BringWindowToTop($handle)
[void][W]::SetForegroundWindow($handle)
Start-Sleep -Milliseconds 700

# DWM's extended frame bounds, not GetWindowRect: on Windows 11 the latter includes an invisible
# resize border, which puts a strip of desktop down each side of every capture.
$rect = New-Object W+RECT
$null = [W]::DwmGetWindowAttribute($handle, 9, [ref]$rect, [System.Runtime.InteropServices.Marshal]::SizeOf($rect))

$width = $rect.Right - $rect.Left
$height = $rect.Bottom - $rect.Top
if ($width -le 0 -or $height -le 0) { Write-Output 'the window has no size'; exit 1 }

$bitmap = New-Object System.Drawing.Bitmap $width, $height
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
$bitmap.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$graphics.Dispose()
$bitmap.Dispose()

Write-Output "$Out  ${width}x${height}"
