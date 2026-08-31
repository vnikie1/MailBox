<#
  Saved with a UTF-8 BOM. Windows PowerShell 5.1 reads an unmarked .ps1 as the system ANSI code
  page, and any non-ASCII character in a string then breaks the parser.
#>

<#
.SYNOPSIS
  Runs the Windows App Certification Kit against the installed package. docs/07 §2.6.

.DESCRIPTION
  WACK runs the same checks Store certification runs. A WACK failure is a guaranteed rejection,
  so this is not optional before submitting.

  **This must be run from an elevated prompt.** appcert.exe refuses to start otherwise, with
  "The requested operation requires elevation" and nothing else. That is the only reason this is
  a separate script from make-msix.ps1, which needs no elevation at all.

  Expect it to take 10-20 minutes. It launches and closes the app repeatedly, drives it, and
  watches for crashes — so leave the machine alone while it runs, and close Halcyon first.

.PARAMETER Report
  Where to write the report. Defaults to a timestamped file beside the package.

.EXAMPLE
  # An elevated prompt opens in C:\WINDOWS\system32, not in the repository, so a relative path
  # will not resolve. Use the full one, quoted -- there is a space in it.
  powershell -ExecutionPolicy Bypass -File "C:\path\to\MacMail-for-Windows\tools\run-wack.ps1"

.EXAMPLE
  # Or change directory first, which is less to get wrong:
  cd "C:\path\to\MacMail-for-Windows"
  powershell -ExecutionPolicy Bypass -File tools\run-wack.ps1
#>

[CmdletBinding()]
param(
  [string]$Report
)

$ErrorActionPreference = 'Stop'

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$elevated = (New-Object Security.Principal.WindowsPrincipal $identity).IsInRole(
  [Security.Principal.WindowsBuiltInRole]::Administrator)

if (-not $elevated) {
  # `$PSCommandPath` is the absolute path to this file. Printing the *relative* one here was a
  # small cruelty: an elevated prompt starts in C:\WINDOWS\system32, so pasting it fails with
  # "the argument to the -File parameter does not exist" — which reads as though the script is
  # missing, rather than as though the working directory is somewhere else.
  throw @"
The App Certification Kit requires an elevated prompt.

Right-click Windows PowerShell, choose "Run as administrator", then paste this exactly. The
quotes matter — there is a space in the path — and an elevated prompt does not start in the
repository, so the full path is not optional.

  powershell -ExecutionPolicy Bypass -File "$PSCommandPath"

Or open an elevated prompt straight from here, which skips the pasting:

  Start-Process powershell -Verb RunAs -ArgumentList '-NoExit -ExecutionPolicy Bypass -File "$PSCommandPath"'
"@
}

$appcert = "${env:ProgramFiles(x86)}\Windows Kits\10\App Certification Kit\appcert.exe"
if (-not (Test-Path $appcert)) {
  $appcert = "${env:ProgramFiles}\Windows Kits\10\App Certification Kit\appcert.exe"
}
if (-not (Test-Path $appcert)) {
  throw "appcert.exe not found. It ships with the Windows SDK; install the App Certification Kit component."
}

$package = Get-AppxPackage Unikie1.HalcyonMail
if (-not $package) {
  throw @"
Halcyon is not installed, so there is nothing to certify.

Package it and register it first:

  powershell -ExecutionPolicy Bypass -File tools\make-msix.ps1
  Add-AppxPackage -Register src-tauri\target\msix-staging\AppxManifest.xml
"@
}

if (-not $Report) {
  $root = Split-Path -Parent $PSScriptRoot
  $dir = Join-Path $root 'src-tauri\target\msix'
  New-Item -ItemType Directory -Path $dir -Force | Out-Null
  $Report = Join-Path $dir ("wack-{0:yyyyMMdd-HHmmss}.xml" -f (Get-Date))
}

# Halcyon holds a mail database and an IMAP connection. WACK will start and stop the app itself,
# and a copy already running confuses both it and the single-instance plugin.
$running = Get-Process Halcyon -ErrorAction SilentlyContinue
if ($running) {
  "closing the running copy first"
  $running | Stop-Process -Force
  Start-Sleep -Seconds 3
}

"package : $($package.PackageFullName)"
"report  : $Report"
""
"This takes 10-20 minutes and drives the app. Leave the machine alone."
""

& $appcert reset
& $appcert test -apptype windowsstoreapp -packagefullname $package.PackageFullName -reportoutputpath $Report

if ($LASTEXITCODE -ne 0) {
  "appcert exited with $LASTEXITCODE"
}

if (-not (Test-Path $Report)) {
  throw "no report was written to $Report"
}

# The XML is long and the answer is one word. Pull the overall result and every failure out of it
# rather than leaving somebody to read a thousand lines of it.
[xml]$xml = Get-Content $Report
$overall = $xml.REPORT.OVERALL_RESULT

""
"=============================================="
"  OVERALL: $overall"
"=============================================="
""

$failures = $xml.SelectNodes('//*[@RESULT="FAIL"]')
if ($failures.Count -eq 0) {
  "No failures. Every failure here is a guaranteed Store rejection, so none is what you want."
} else {
  "$($failures.Count) failure(s) — each one is a guaranteed rejection:"
  ""
  foreach ($failure in $failures) {
    $name = $failure.GetAttribute('NAME')
    if (-not $name) { $name = $failure.LocalName }
    "  - $name"
    $message = $failure.SelectSingleNode('.//MESSAGE')
    if ($message) { "      $($message.InnerText.Trim())" }
  }
}

""
"Full report: $Report"
"Open it with:  Start-Process '$Report'"
