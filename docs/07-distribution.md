# Distribution — installers, signing, and the Microsoft Store

Phase 11 reference. Read §1 now; §2 has weeks of lead time and should start around Phase 9.

---

## 0. The three gates a Windows app must pass

Understand these before choosing a path — they are separate mechanisms with very different
consequences.

| Gate | What it does | User can override? |
|---|---|---|
| **Smart App Control** | Hard-blocks anything not signed by a trusted publisher or vouched for by Microsoft's reputation service | **No.** No dialog, no "run anyway" |
| **SmartScreen** | "Windows protected your PC" dialog on files carrying Mark of the Web | Yes — More info → Run anyway |
| **Defender** | Heuristic scan; unsigned Rust binaries occasionally false-positive | Only if it doesn't flag |

**Mark of the Web (MOTW)** is the alternate data stream Windows attaches to files that arrived
from the internet. It is what triggers SmartScreen:

| Delivery | MOTW | SmartScreen fires? |
|---|---|---|
| Downloaded from a website or GitHub Releases | Yes | Yes |
| Extracted from a downloaded ZIP | Usually | Yes |
| Emailed or messaged | Yes | Yes |
| Copied from USB or a LAN share | No | No |
| Built locally | No | No |

Check and clear it:

```powershell
Get-Item .\Halcyon_setup.exe -Stream Zone.Identifier -ErrorAction SilentlyContinue
Unblock-File .\Halcyon_setup.exe
```

`Unblock-File` clears SmartScreen. It does **nothing** for Smart App Control.

> **Developer note.** Smart App Control blocks locally built binaries, so it must be off on the
> development machine. It is a **one-way switch** — once disabled, the only way to re-enable it
> is a clean reinstall of Windows. Decide deliberately.

---

## 1. Path A — Standalone installer

Already configured in `src-tauri/tauri.conf.json`. Build:

```powershell
npm run tauri build
```

| Output | Purpose |
|---|---|
| `src-tauri/target/release/bundle/nsis/<Name>_<ver>_x64-setup.exe` | The installer you distribute |
| `src-tauri/target/release/bundle/msi/<Name>_<ver>_x64_en-US.msi` | Enterprise / GPO deployment (add `"msi"` to `bundle.targets`) |
| `src-tauri/target/release/<Name>.exe` | Raw binary — will not run standalone elsewhere |

`"installMode": "currentUser"` is correct for this app: no admin prompt, and `mailto:`
registration works from `HKCU`.

### Code signing

| Route | Cost | Requirement | SmartScreen outcome |
|---|---|---|---|
| **Azure Trusted Signing** | ~$10/mo | Org with 3+ years of verifiable history, **or** individual validation | Reputation accrues over downloads |
| OV certificate (Sectigo, DigiCert…) | ~$200–400/yr | Hardware token or cloud HSM — mandatory since the 2023 CA/B rule change | Reputation accrues |
| EV certificate | ~$400–700/yr | Hardware token, stricter vetting | **Immediate** reputation |
| Unsigned | free | — | Blocked by SAC; scary dialog everywhere else |

Individual developers should use **Azure Trusted Signing with individual validation**.
Identity verification takes **2–4 weeks** — start it at Phase 9.

Once issued, sign at build time:

```json
"bundle": {
  "windows": {
    "certificateThumbprint": "YOUR_THUMBPRINT",
    "digestAlgorithm": "sha256",
    "timestampUrl": "http://timestamp.digicert.com"
  }
}
```

Always timestamp. Without it, every signature expires when the certificate does.

### Auto-update

```powershell
npm run tauri signer generate -- -w $env:USERPROFILE\.tauri\halcyon.key
```

Add `tauri-plugin-updater`, put the public key in `tauri.conf.json`, host `latest.json` on
GitHub Releases or any static host. **Disable this entirely in Store builds** (§2.3).

---

## 2. Path B — Microsoft Store

**The decisive advantage: Microsoft signs your package.** No certificate purchase, no identity
validation wait, and both Smart App Control and SmartScreen stop being a problem. For a solo
developer this is the single strongest reason to ship through the Store.

**Cost:** one-time Partner Center registration — **$19 individual**, $99 company.
Free apps pay no revenue share. Paid non-game apps pay 15%.

---

### 2.1 — Register at Partner Center

1. Go to partner.microsoft.com, choose **Windows & Xbox** developer program.
2. Pick **Individual** unless you have a registered company. Individual accounts can publish
   free and paid apps.
3. Pay the one-time fee. Verification is usually same-day for individuals.

> The **publisher display name** is what appears as the publisher on the Store listing. On this
> account it came out as `Unikie1`, taken from the account handle rather than a legal name. It
> can be changed in Partner Center → Account settings, subject to re-verification — worth doing
> before the first submission if a different name should be public. A company account is only
> needed to publish under a *business* name.

---

### 2.2 — Reserve the app name

Partner Center → **Create a new app** → enter the name.

Do this **early**. It is free, takes five minutes, and it is what generates the three identity
strings you cannot invent yourself. Find them under **Product → Product identity**:

| Value | Example | Where it goes |
|---|---|---|
| **Package/Identity/Name** | `Unikie1.HalcyonMail` | `<Identity Name="…">` |
| **Package/Identity/Publisher** | `CN=AFB09E9D-38C1-4779-9510-AF7E1F2C78F4` | `<Identity Publisher="…">` |
| **Package/Properties/PublisherDisplayName** | `Unikie1` | `<PublisherDisplayName>` |

Getting any of these wrong causes the upload to be rejected with an identity-mismatch error.

Name reservations expire after three months if you never submit — just re-reserve.

#### Reserved name and the naming split

**`Halcyon Mail` was reserved on 2026-08-25.** `Halcyon` alone was taken.

This produces a deliberate split, and both halves matter:

| Where | Value | Why |
|---|---|---|
| Store listing | **Halcyon Mail** | The reserved name. Puts "mail" in the Store search index, which bare "Halcyon" would not. |
| `<Properties><DisplayName>` in the manifest | **Halcyon Mail** | **Must match a reserved name**, or the package is rejected at upload. Not optional. |
| `<uap:VisualElements DisplayName>` | **Halcyon** | Start menu and app list. Short reads better on a tile. |
| `tauri.conf.json` `productName` | **Halcyon** | Binary name (`Halcyon.exe`) and NSIS installer name. |
| Window title, in-app branding, icon | **Halcyon** | The product is called Halcyon. "Mail" is a search affordance, not part of the name. |

No source rename is needed for this — the project already builds as `Halcyon`, and the longer
name exists only in the MSIX manifest and the Store listing.

#### Recorded identity — Partner Center, 2026-08-25

These are the live values. They are **identifiers, not secrets** — every one of them is public
in the shipped package, so version-controlling them is correct.

| Field | Value | Used by |
|---|---|---|
| Package/Identity/Name | `Unikie1.HalcyonMail` | `<Identity Name>` |
| Package/Identity/Publisher | `CN=AFB09E9D-38C1-4779-9510-AF7E1F2C78F4` | `<Identity Publisher>`, and the **subject of the self-signed test cert** (§2.6) |
| PublisherDisplayName | `Unikie1` | `<PublisherDisplayName>`, shown publicly on the listing |
| Package Family Name | `Unikie1.HalcyonMail_anw48tyhk74bp` | The MSIX data directory (§2.3) and the toast AUMID |
| Package SID | `S-1-15-2-729224163-1076728153-953244817-3612561831-405782540-1140748460-737838963` | Only needed for a loopback exemption or enterprise auth. Not used today. |

The **AUMID** for toast notifications is the PFN plus `!` plus the `Application Id` from the
manifest — here `Unikie1.HalcyonMail_anw48tyhk74bp!Halcyon`. Under MSIX this is supplied
automatically; Phase 10 needs it only for the unpackaged NSIS build.

---

### 2.3 — Prepare the app for packaging

Four changes before you build:

**1. Version format.** MSIX requires `Major.Minor.Build.Revision` with the **revision forced to
zero**. `0.0.0` will not submit.

```json
"version": "1.0.0"
```
→ becomes `1.0.0.0` in the manifest. Every submission must increment.

**2. Disable the Tauri updater in Store builds.** The Store handles updates; two mechanisms
fighting will produce duplicate installs and failed certification. Gate it behind a Cargo
feature:

```toml
[features]
default = ["self-update"]
store = []
```

Build the Store package with `--no-default-features --features store`.

**3. Verify data paths under virtualization.** MSIX redirects per-user writes:

```
%LOCALAPPDATA%\Halcyon\            →  %LOCALAPPDATA%\Packages\Unikie1.HalcyonMail_anw48tyhk74bp\LocalCache\Local\Halcyon\
HKCU\Software\Halcyon\             →  virtualized registry hive
```

The SQLite DB, `.eml` store, and attachment cache all land in the redirected location. This
works, but **test it explicitly** — a hardcoded absolute path will silently write somewhere
you don't expect.

**4. Confirm what is *not* virtualized:**

| Subsystem | Behaviour under MSIX |
|---|---|
| **Windows Credential Manager** | Works normally. Not virtualized. OAuth tokens are safe. |
| **WebView2** | Works. System component, no declaration needed. |
| **Toast notifications** | **Better** — package identity supplies the AUMID for free. No COM activator registration needed. This *simplifies* Phase 10. |
| **`mailto:` handler** | Declared in the manifest, not the registry (§2.4). |
| **Run at login** | Use the `windows.startupTask` extension, not `HKCU\...\Run`. |
| **Taskbar badge / jump list** | Work normally. |

---

### 2.4 — Author the manifest

MSIX needs an `AppxManifest.xml`. Full-trust Win32 apps use the Desktop Bridge shape:

```xml
<?xml version="1.0" encoding="utf-8"?>
<Package
  xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
  xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10"
  xmlns:uap5="http://schemas.microsoft.com/appx/manifest/uap/windows10/5"
  xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities"
  IgnorableNamespaces="uap uap5 rescap">

  <!-- All three values come from Partner Center. Do not invent them. -->
  <Identity
    Name="Unikie1.HalcyonMail"
    Publisher="CN=AFB09E9D-38C1-4779-9510-AF7E1F2C78F4"
    Version="1.0.0.0"
    ProcessorArchitecture="x64" />

  <Properties>
    <DisplayName>Halcyon Mail</DisplayName>
    <PublisherDisplayName>Unikie1</PublisherDisplayName>
    <Logo>Assets\StoreLogo.png</Logo>
  </Properties>

  <Dependencies>
    <TargetDeviceFamily Name="Windows.Desktop"
                        MinVersion="10.0.19041.0"
                        MaxVersionTested="10.0.26100.0" />
  </Dependencies>

  <Resources>
    <Resource Language="en-us" />
  </Resources>

  <Capabilities>
    <!-- Required for any Win32 app. Needs justification at submission. -->
    <rescap:Capability Name="runFullTrust" />
  </Capabilities>

  <Applications>
    <Application Id="Halcyon"
                 Executable="Halcyon.exe"
                 EntryPoint="Windows.FullTrustApplication">

      <uap:VisualElements
        DisplayName="Halcyon"
        Description="A local-first email client for Windows 11."
        BackgroundColor="transparent"
        Square150x150Logo="Assets\Square150x150Logo.png"
        Square44x44Logo="Assets\Square44x44Logo.png">
        <uap:DefaultTile Wide310x150Logo="Assets\Wide310x150Logo.png"
                         Square310x310Logo="Assets\Square310x310Logo.png"
                         Square71x71Logo="Assets\Square71x71Logo.png" />
      </uap:VisualElements>

      <Extensions>
        <!-- mailto: default-handler registration -->
        <uap:Extension Category="windows.protocol">
          <uap:Protocol Name="mailto">
            <uap:DisplayName>Halcyon</uap:DisplayName>
          </uap:Protocol>
        </uap:Extension>

        <!-- .eml file association -->
        <uap:Extension Category="windows.fileTypeAssociation">
          <uap:FileTypeAssociation Name="eml">
            <uap:DisplayName>Email Message</uap:DisplayName>
            <uap:SupportedFileTypes>
              <uap:FileType>.eml</uap:FileType>
            </uap:SupportedFileTypes>
          </uap:FileTypeAssociation>
        </uap:Extension>

        <!-- Run at login, user-toggleable in Windows Settings -->
        <uap5:Extension Category="windows.startupTask"
                        Executable="Halcyon.exe"
                        EntryPoint="Windows.FullTrustApplication">
          <uap5:StartupTask TaskId="HalcyonStartup"
                            Enabled="false"
                            DisplayName="Halcyon" />
        </uap5:Extension>
      </Extensions>
    </Application>
  </Applications>
</Package>
```

#### Required image assets

Place under `Assets\`. Missing assets fail certification.

| Asset | Base size |
|---|---|
| `Square44x44Logo.png` | 44×44 — taskbar and app list |
| `Square71x71Logo.png` | 71×71 — small tile |
| `Square150x150Logo.png` | 150×150 — medium tile |
| `Square310x310Logo.png` | 310×310 — large tile |
| `Wide310x150Logo.png` | 310×150 — wide tile |
| `StoreLogo.png` | 50×50 — Store listing |

Generate scale variants at **100/125/150/200/400 %** (`Square150x150Logo.scale-200.png` etc.)
and target-size variants of the 44×44 at **16/24/32/48/256** px
(`Square44x44Logo.targetsize-32.png`). The **Visual Studio Asset Generator** or
`Microsoft.Windows.SDK.BuildTools` will produce the whole set from one 1024×1024 source — do
not hand-produce 40 files.

---

### 2.5 — Build the MSIX

⚠️ **Tauri 2 does not emit MSIX natively.** Three options:

| Method | Effort | When |
|---|---|---|
| **MSIX Packaging Tool** (free, from the Store) | Lowest | Wraps your existing NSIS installer by recording an install. Fine for a first submission. |
| **`makeappx.exe`** (Windows SDK) | Medium | Full control, scriptable, CI-friendly. **Recommended.** |
| Windows Application Packaging Project (Visual Studio) | Medium | If you already use VS |

Using `makeappx`, stage a folder then pack:

```
msix-staging/
├─ AppxManifest.xml
├─ Halcyon.exe              ← from src-tauri/target/release/
├─ (any sidecar DLLs)
└─ Assets/                  ← all logo variants
```

```powershell
& "${env:ProgramFiles(x86)}\Windows Kits\10\bin\10.0.26100.0\x64\makeappx.exe" pack /d .\msix-staging /p .\Halcyon.msix /o
```

For Store submission the package may be uploaded **unsigned** — Microsoft signs it. For local
sideload testing you must sign with a certificate your machine trusts (§2.6).

---

### 2.6 — Test locally before submitting

**1. Create a self-signed test certificate.** Its subject must match `<Identity Publisher>`
exactly:

```powershell
New-SelfSignedCertificate -Type Custom -Subject "CN=AFB09E9D-38C1-4779-9510-AF7E1F2C78F4" -KeyUsage DigitalSignature -FriendlyName "Halcyon Test" -CertStoreLocation "Cert:\CurrentUser\My" -TextExtension @("2.5.29.37={text}1.3.6.1.5.5.7.3.3", "2.5.29.19={text}")
```

Export it, trust it in `LocalMachine\TrustedPeople`, then sign:

```powershell
& "${env:ProgramFiles(x86)}\Windows Kits\10\bin\10.0.26100.0\x64\signtool.exe" sign /fd SHA256 /a /f .\HalcyonTest.pfx /p <password> .\Halcyon.msix
```

**2. Install and test:**

```powershell
Add-AppxPackage .\Halcyon.msix
```

Verify specifically: the DB lands where you expect and survives a restart; OAuth tokens
persist in Credential Manager; toasts fire with the package AUMID; `mailto:` opens the app;
the startup task appears in Settings → Apps → Startup.

> **Corrected 2026-08-31, by installing it.** This section previously said the database lands in
> "the redirected path". It does not. A `runFullTrust` package writes **straight through** to the
> real `%LOCALAPPDATA%\com.uniki.halcyon` — filesystem redirection into
> `%LOCALAPPDATA%\Packages\<PFN>` applies to sandboxed UWP apps, not to full-trust desktop ones.
>
> Measured on a registered package: the Store build opened the same `halcyon.db` the NSIS build
> uses, synced 46 mailboxes into it, and rendered a message from it.
>
> This is the better outcome and worth keeping deliberately: somebody who installs the Store
> version over the downloaded one keeps their mail, their accounts and their settings, with no
> migration step. The cost is that the two builds cannot run at once against the same database —
> which they could not anyway, because of the single-instance plugin.

**3. Run the Windows App Certification Kit** (ships with the Windows SDK). It runs the same
checks certification will. Fix everything it flags — a WACK failure is a guaranteed rejection.

**4. Uninstall cleanly:**

```powershell
Remove-AppxPackage -Package (Get-AppxPackage *Halcyon*).PackageFullName
```

---

### 2.7 — Submit

Partner Center → your app → **Start submission**. Six sections:

#### Pricing and availability
Free. Select markets (all, unless a provider restriction applies). Choose immediate publish or
manual.

#### Properties
- **Category:** Productivity
- **Privacy policy URL — MANDATORY.** Your app handles personal data and credentials. This must
  be a live, publicly reachable URL before you submit. No policy, no submission.
- Support contact, website.
- **Declare** that the app accesses personal information.

#### Age ratings
IARC questionnaire. A mail client is straightforward, but **answer honestly that the app allows
user-to-user communication and shares user-provided content** — misdeclaring here is a
certification failure and can get an app pulled later.

#### Packages
Upload the `.msix`. Partner Center validates identity, version and manifest immediately —
most errors surface here, not in certification.

#### Store listing
- Description (up to 10,000 characters)
- **Screenshots: at least one**, 1366×768 or larger. Practically, use 4–8 showing the three-pane
  layout in light and dark, compose, and search.
- Feature list, "what's new", search terms.

#### Submission options — **the part people skip and get rejected for**

- **Restricted capability justification.** `runFullTrust` needs one. Write something concrete:
  *"Desktop application requiring full-trust for IMAP/SMTP network protocol access, local SQLite
  storage, and Windows Credential Manager integration for OAuth token storage."*
- **Notes for certification — critical for a mail client.** The reviewer must be able to log in
  and exercise the app. **Provide working test account credentials** (a throwaway IMAP account),
  plus a short walkthrough of how to add an account and read mail. Without this the reviewer
  hits your onboarding screen, cannot proceed, and fails the submission as "incomplete
  functionality." This is the most common rejection for email clients.

---

### 2.8 — Certification

Typically **24–72 hours**; a first submission can take longer. If it fails you get a report
naming the failed policy, and resubmission is free and unlimited.

Common rejections for an app of this type:

| Reason | Prevention |
|---|---|
| Reviewer couldn't log in | Supply test credentials in Notes for Certification |
| Privacy policy URL dead or missing | Verify the link resolves before submitting |
| Crash during automated testing | Run WACK; test on a clean VM |
| Restricted capability unjustified | Write a specific, technical justification |
| Metadata mismatch | Screenshots and description must match what the app does |
| Incomplete functionality | Every visible feature must work; no dead ends (Phase 10 gate) |

---

### 2.9 — After publishing

- **Updates:** increment the version, upload a new package, submit. Windows updates installed
  apps automatically.
- **Staged rollout:** roll a new version to a percentage of users first. Use it.
- **Package flights:** private test packages for a named group — the Store's beta channel.
- **Analytics:** Partner Center gives installs, ratings, crash data. It is aggregate telemetry
  from Microsoft, not from your app — it does not violate the no-telemetry promise, but say so
  plainly in the privacy policy.

---

## 3. Ship both

Not mutually exclusive, and worth doing:

| Channel | For |
|---|---|
| **Microsoft Store (MSIX)** | Mainstream users. Free signing, automatic updates, both gates bypassed. |
| **Direct NSIS `.exe`** | Users who avoid the Store, beta testers, enterprises. Needs a real certificate. |

Same codebase, two bundle outputs, one build matrix in CI.

---

## 4. Timeline

| When | Do |
|---|---|
| **Now** | Settle the product name. Reserve it in Partner Center ($19). Fix `version` to `1.0.0`. |
| **Phase 9** | Begin Azure Trusted Signing identity validation (2–4 weeks). Write the privacy policy and host it. |
| **Phase 10** | Generate the icon asset set. Author `AppxManifest.xml`. Build and sideload-test an MSIX. Create the throwaway IMAP test account for reviewers. |
| **Phase 11** | Run WACK. Capture screenshots. Submit to Store. Sign and publish the NSIS build. |

---

## 5. Pre-submission checklist

**Identity**
- [ ] Product name settled and reserved in Partner Center
- [ ] `Identity Name`, `Publisher`, `PublisherDisplayName` copied exactly from Product identity
- [ ] Version is `Major.Minor.Build.0` and higher than any prior submission

**Package**
- [ ] Tauri updater disabled in the Store build
- [ ] Data paths verified under MSIX virtualization
- [ ] Credential Manager, toasts, `mailto:`, `.eml`, startup task all verified in a sideloaded install
- [ ] Full icon asset set present at every required scale and target size
- [ ] WACK passes with no failures
- [ ] Clean install and clean uninstall verified on a fresh Windows 11 VM

**Submission**
- [ ] Privacy policy live at a public URL
- [ ] `runFullTrust` justification written
- [ ] **Test account credentials in Notes for Certification**
- [ ] Screenshots: light and dark, three-pane, compose, search
- [ ] Age rating declares user-to-user communication
- [ ] Personal-information access declared in Properties
