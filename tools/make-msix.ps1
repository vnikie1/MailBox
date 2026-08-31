<#
  Saved with a UTF-8 BOM on purpose. Windows PowerShell 5.1 reads an unmarked .ps1 as the
  system ANSI code page, so an em-dash in a string becomes three bytes of mojibake and the
  parser dies on "Unexpected token". PowerShell 7 does not care; 5.1 is what ships with Windows.
#>

<#
.SYNOPSIS
  Builds the Microsoft Store package. docs/07 §2.5.

.DESCRIPTION
  Builds Halcyon with the updater compiled out, stages the MSIX layout, and packs it with
  makeappx from the Windows SDK.

  The package is produced UNSIGNED, which is correct: the Store signs it. Signing is only needed
  to sideload it for testing, which -Test does with a self-signed certificate.

  ## What this refuses to do

  Ship a Store package with the self-updater in it. The Store installs its own updates, and two
  mechanisms fighting produces duplicate installs and fails certification (docs/07 §2.3). That is
  enforced three times over — a compile_error if both features are on, a check that the built
  binary does not contain the update endpoint, and the build flags below — because a mistake here
  is not caught until certification, days later.

.PARAMETER Test
  Also sign the package with a self-signed certificate and install it locally. For trying the
  thing before submitting it; never for release.

.PARAMETER SkipBuild
  Package whatever is already in target/release. For iterating on the manifest.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File tools/make-msix.ps1
  powershell -ExecutionPolicy Bypass -File tools/make-msix.ps1 -Test

  Windows PowerShell 5.1, which is what ships with Windows. pwsh works too where it exists.
#>

[CmdletBinding()]
param(
  [switch]$Test,
  [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$tauri = Join-Path $root 'src-tauri'
$msix = Join-Path $tauri 'msix'
$staging = Join-Path $tauri 'target\msix-staging'
$outDir = Join-Path $tauri 'target\msix'

# ---------------------------------------------------------------- the SDK tools

# Newest SDK first: an old makeappx cannot read a manifest that uses a newer namespace, and the
# error it gives ("unexpected element") points at the manifest rather than at itself.
$sdkRoot = "${env:ProgramFiles(x86)}\Windows Kits\10\bin"
$sdk = Get-ChildItem $sdkRoot -Directory -ErrorAction SilentlyContinue |
  Where-Object { $_.Name -match '^10\.' } |
  Sort-Object Name -Descending |
  Select-Object -First 1

if (-not $sdk) {
  throw "No Windows SDK found under $sdkRoot. Install the Windows 10/11 SDK; makeappx and signtool come with it."
}

$makeappx = Join-Path $sdk.FullName 'x64\makeappx.exe'
$signtool = Join-Path $sdk.FullName 'x64\signtool.exe'

if (-not (Test-Path $makeappx)) { throw "makeappx.exe not found at $makeappx" }
"using SDK $($sdk.Name)"

# ---------------------------------------------------------------- version

# Read from tauri.conf.json rather than from the manifest, so there is one place a version is
# changed. MSIX wants four parts with the revision forced to zero; the Store rejects a `0` major
# and rejects any submission that does not increment.
$conf = Get-Content (Join-Path $tauri 'tauri.conf.json') -Raw | ConvertFrom-Json
$version = $conf.version
if ($version -notmatch '^\d+\.\d+\.\d+$') { throw "tauri.conf.json version '$version' is not Major.Minor.Patch" }
if ($version -like '0.*') { throw "version '$version' has a zero major; the Store will not accept it (docs/07 2.3)" }
$msixVersion = "$version.0"
"packaging version $msixVersion"

# ---------------------------------------------------------------- build

if (-not $SkipBuild) {
  Push-Location $root
  try {
    # --no-default-features drops `self-update`, which drops the updater plugin entirely.
    "building with the updater compiled out"
    & npm run tauri -- build --no-bundle -- --no-default-features --features store
    if ($LASTEXITCODE -ne 0) { throw "the release build failed" }
  } finally {
    Pop-Location
  }
}

$exe = Join-Path $tauri 'target\release\Halcyon.exe'
if (-not (Test-Path $exe)) { throw "Halcyon.exe not found at $exe" }

# ---------------------------------------------------------------- the store check

# The binary itself, not the build flags. A flag can be got wrong; this asks the artefact.
#
# The needles are the updater plugin's *command routing* strings, which exist as data in the
# binary only when the plugin is linked.
#
# The first version searched for the update endpoint URL instead, and was wrong in the
# expensive direction: it failed a perfectly good Store build. Tauri embeds the whole of
# tauri.conf.json into every binary, so the endpoint is present whether or not the plugin is —
# it is configuration, not code. Verified by building both ways and comparing: the endpoint is
# in both, 'plugin:updater|check' is in neither the Store build nor anything else.
$needles = @('plugin:updater|check', 'plugin:updater|download_and_install')
$bytes = [System.IO.File]::ReadAllBytes($exe)
$text = [System.Text.Encoding]::ASCII.GetString($bytes)
$found = $needles | Where-Object { $text.Contains($_) }

if ($found) {
  throw @"
This binary still contains the updater plugin ($($found -join ', ')).

A Store package must not update itself: the Store does that, and two mechanisms fighting
produces duplicate installs and fails certification (docs/07 2.3).

Build with:  cargo build --release --no-default-features --features store
"@
}
"the self-updater is not in this binary"

# ---------------------------------------------------------------- stage

if (Test-Path $staging) { Remove-Item $staging -Recurse -Force }
New-Item -ItemType Directory -Path $staging -Force | Out-Null
New-Item -ItemType Directory -Path $outDir -Force | Out-Null

Copy-Item $exe (Join-Path $staging 'Halcyon.exe')
Copy-Item (Join-Path $msix 'Assets') (Join-Path $staging 'Assets') -Recurse

# The manifest, with the version substituted. Written into staging rather than edited in place,
# so the committed manifest keeps one canonical version and the build never dirties the tree.
$manifest = Get-Content (Join-Path $msix 'AppxManifest.xml') -Raw
$manifest = $manifest -replace 'Version="\d+\.\d+\.\d+\.\d+"', "Version=`"$msixVersion`""
Set-Content -Path (Join-Path $staging 'AppxManifest.xml') -Value $manifest -Encoding UTF8

# The base names the manifest refers to. Windows resolves `.scale-100` and friends by
# convention, but makeappx wants the unqualified file to exist as well or it warns on every one.
foreach ($base in 'Square44x44Logo', 'Square71x71Logo', 'Square150x150Logo', 'Square310x310Logo', 'Wide310x150Logo', 'StoreLogo') {
  $from = Join-Path $staging "Assets\$base.scale-100.png"
  if (Test-Path $from) { Copy-Item $from (Join-Path $staging "Assets\$base.png") }
}

# ---------------------------------------------------------------- pack

$package = Join-Path $outDir "Halcyon_$($msixVersion)_x64.msix"
if (Test-Path $package) { Remove-Item $package -Force }

& $makeappx pack /d $staging /p $package /o
if ($LASTEXITCODE -ne 0) { throw "makeappx failed" }

"packed $package"

# ---------------------------------------------------------------- sideload, optionally

if ($Test) {
  # The certificate subject must match <Identity Publisher> exactly, or Windows refuses the
  # package with a signature error that says nothing about the subject.
  $subject = 'CN=AFB09E9D-38C1-4779-9510-AF7E1F2C78F4'
  $pfx = Join-Path $outDir 'HalcyonTest.pfx'
  $password = ConvertTo-SecureString -String 'halcyon-test' -Force -AsPlainText

  $cert = Get-ChildItem Cert:\CurrentUser\My | Where-Object { $_.Subject -eq $subject } | Select-Object -First 1
  if (-not $cert) {
    "creating a self-signed test certificate"
    $cert = New-SelfSignedCertificate -Type Custom -Subject $subject `
      -KeyUsage DigitalSignature -FriendlyName 'Halcyon Test' `
      -CertStoreLocation 'Cert:\CurrentUser\My' `
      -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3', '2.5.29.19={text}')
  }

  Export-PfxCertificate -Cert $cert -FilePath $pfx -Password $password | Out-Null

  & $signtool sign /fd SHA256 /a /f $pfx /p 'halcyon-test' $package
  if ($LASTEXITCODE -ne 0) { throw "signtool failed" }

  ""
  "Signed for sideloading. To trust and install:"
  "  Import-PfxCertificate -FilePath '$pfx' -CertStoreLocation Cert:\LocalMachine\TrustedPeople -Password (ConvertTo-SecureString 'halcyon-test' -AsPlainText -Force)   # needs elevation"
  "  Add-AppxPackage '$package'"
  ""
  "Then check the things MSIX changes, per docs/07 2.6:"
  "  - the database lands in the redirected path and survives a restart"
  "  - OAuth tokens still resolve from Credential Manager"
  "  - toasts fire with the package AUMID Unikie1.HalcyonMail_anw48tyhk74bp!Halcyon"
  "  - a mailto: link opens the app"
  "  - Settings > Apps > Startup lists Halcyon, off"
  ""
  "And run the Windows App Certification Kit before submitting. A WACK failure is a"
  "guaranteed rejection."
} else {
  ""
  "Unsigned, which is what the Store wants — Microsoft signs it on ingestion."
  "Upload $package in Partner Center under Packages."
  ""
  "Before submitting, docs/07 2.7 lists the two that sink email clients:"
  "  - Notes for Certification MUST carry working test account credentials,"
  "    or the reviewer cannot get past the welcome screen and fails you for"
  "    incomplete functionality. This is the most common rejection."
  "  - runFullTrust needs a written justification. Be specific: IMAP/SMTP over"
  "    sockets, local SQLite storage, Credential Manager for OAuth tokens."
}
