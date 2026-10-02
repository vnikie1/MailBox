# Microsoft Store submission — Halcyon Mail 1.0.0

Everything Partner Center asks for, in the order it asks, ready to paste. `docs/07-distribution.md`
is the reasoning behind each answer; this file is the answers.

Partner Center: <https://partner.microsoft.com/dashboard> → **Apps and games** → **Halcyon Mail**
→ **Start submission**.

---

## Before you submit — four things only you can do

Each of these changes what the listing may truthfully say, so settle them first.

1. **Gmail — publish the Google sign-in application.** The Google Cloud project behind Halcyon's
   built-in Google client is still in **Testing**. Until it is published, only the accounts listed
   there as test users can add Gmail at all, and even theirs is signed out every seven days. For
   a Store user that is "Gmail does not work". Google Auth Platform → **Audience** → **Publish
   app** (`src-tauri/oauth/README.md`, route A). Published but unverified, sign-in shows Google's
   _"hasn't verified this app"_ screen and stops admitting new users after 100 over the project's
   lifetime; verification (route B, weeks) removes both. **If you do not publish, delete the Gmail
   lines from the description and features below.**

2. **Outlook.com — register a Microsoft sign-in application, or say nothing about Outlook.** This
   build carries no Microsoft client (`HALCYON_MICROSOFT_CLIENT_ID` is empty in
   `src-tauri/oauth/clients.env`), so the Outlook tile asks the user to bring their own. Registering
   one is free and takes about fifteen minutes — `src-tauri/oauth/README.md`, "Registering the
   Microsoft application" — then put the ID in `clients.env` and rebuild the package. The text below
   does not mention Outlook; add it only once that works.

3. **A test account for the reviewers.** The most common rejection for a mail client is a reviewer
   who cannot get past the welcome screen. Create a throwaway mailbox that signs in **with a
   password** — a new Yahoo Mail address with an _app password_ is the simplest — put a few messages
   in it, and fill it into **Notes for certification** below. Not a Gmail address: the reviewer
   would meet Google's warning screen, or Testing mode's wall.

4. **Your public name.** The listing shows the publisher as **Unikie1**, the account handle. To
   show something else, change it in Partner Center → **Account settings** before the first
   submission, then copy the new value into `<PublisherDisplayName>` in
   `src-tauri/msix/AppxManifest.xml` and rebuild — the two must match or the upload is refused.

**Deadline:** the name _Halcyon Mail_ was reserved on 2026-08-25, and a reservation lapses after
three months without a submission. Submit before **25 November 2026**, or re-reserve it.

---

## 1. Packages

| | |
| --- | --- |
| File | `src-tauri/target/msix/Halcyon_1.0.0.0_x64.msix` — **unsigned**, which is what the Store wants: Microsoft signs it on ingestion |
| Built by | `powershell -ExecutionPolicy Bypass -File tools/make-msix.ps1` (updater compiled out, checked in the binary) |
| Identity | `Unikie1.HalcyonMail` · `CN=AFB09E9D-38C1-4779-9510-AF7E1F2C78F4` · `Unikie1` · `1.0.0.0` · x64 |
| Minimum OS | Windows 11 (10.0.22000.0) |

Every later submission needs a higher version: bump `version` in `src-tauri/tauri.conf.json` and
rebuild. `make-msix.ps1` reads it from there.

## 2. Pricing and availability

- **Markets:** all markets.
- **Discoverability:** make this product available and discoverable in the Microsoft Store.
- **Pricing:** Free. No free trial, no sale.
- **Schedule:** release as soon as it passes certification (or "manually", to press the button
  yourself).

## 3. Properties

| Field | Answer |
| --- | --- |
| Category | **Productivity** (no subcategory) |
| Privacy policy URL | <https://vnikie1.github.io/halcyon-mail/privacy.html> |
| Website | <https://vnikie1.github.io/halcyon-mail/> |
| Support contact | vnikie1@gmail.com (already public on the site) |
| Does your product access, collect or transmit personal information? | **Yes** — it stores the user's mail and credentials on their PC and talks to their mail provider. Hence the privacy policy. |

Product declarations:

- _This app allows users to make purchases, but does not use the Microsoft Store commerce system_ —
  **no**.
- _This app has been tested to meet accessibility guidelines_ — **leave unticked** until the Narrator
  walkthrough Phase 10 still wants has been recorded. Ticking it is a claim.
- _Customers can install this app to alternate drives or removable storage_ — leave ticked.
- _Windows can include this app's data in automatic backups to OneDrive_ — leave ticked.
- _This app depends on non-Microsoft drivers or NT services_ — **no**.

System requirements: nothing is required beyond Windows 11; a keyboard is "recommended" if you want
to tick something.

## 4. Age ratings (IARC questionnaire)

Category: **Communication**. Answer honestly — misdeclaring here is a certification failure and can
get the app pulled later (docs/07 §2.7):

| Question | Answer |
| --- | --- |
| Do users interact or exchange content with other users? | **Yes** — it is email |
| Is user-generated content moderated? | No |
| Does it share the user's location? | No |
| Digital purchases? | No |
| Unrestricted internet access, like a web browser? | No — links open in the user's own browser |
| Violence, sexual content, gambling, drugs, crude humour | None in the app itself |

Expect the "Users Interact" notice on the rating.

## 5. Store listing — English (United Kingdom)

The package declares `en-gb`, so that is the listing language Partner Center offers.

### Product name

```text
Halcyon Mail
```

### Description

```text
Halcyon is an email app for Windows 11, made to feel like the Mail app on a Mac: three panes, clear type, and nothing asking to be noticed.

Your mail lives on your PC. There is no Halcyon account to create, no server of ours between you and your email provider, no adverts and no analytics of any kind. Halcyon connects only to your own mail servers.

WORKS WITH YOUR ACCOUNTS
Add any account that offers IMAP. iCloud and Yahoo Mail are set up for you; anything else needs only its server name. Gmail signs in through your own browser, never through a window this app drew.

READ AND SEARCH, EVEN OFFLINE
Mail is downloaded and indexed on your PC, so reading and searching work with no connection at all. Search every account at once, with from:, subject:, has:attachment and date ranges, and get answers immediately even in a mailbox of a hundred thousand messages.

STAY ORGANISED
Rules, smart mailboxes, VIPs, coloured flags and junk filtering: the organising tools Mail has, running on your PC rather than on somebody's server.

UNDO ALMOST ANYTHING
Archived, moved or deleted the wrong thing? Press Ctrl+Z. Sending waits a moment first, so you can take a message back.

BRING YOUR MAIL WITH YOU, AND TAKE IT AWAY
Import from Thunderbird, mbox files and Outlook .pst files. Export to mbox or to a folder of .eml files whenever you like. Your mail is yours.

MADE FOR WINDOWS 11
Light and dark themes, Mica, Snap Layouts, notifications, a taskbar badge and a jump list. Halcyon can open mailto: links and .eml files, works entirely from the keyboard, and can be driven by a screen reader.

PRIVATE BY DESIGN
Passwords and sign-in tokens are kept in Windows Credential Manager, never in a file. Messages are shown in a sandbox that cannot run code, and you can stop remote images from loading. The privacy policy says exactly what leaves your PC, which is very nearly nothing: https://vnikie1.github.io/halcyon-mail/privacy.html
```

If Gmail is not working for everyone yet (step 1 above), delete the sentence beginning "Gmail
signs in".

### What's new in this version

```text
First release.
```

### Product features

One per box, in this order.

```text
A three-pane layout modelled on the Mail app on a Mac
Works with any IMAP account; iCloud and Yahoo Mail are set up for you
Gmail signs in through your own browser
Mail is stored and indexed on your PC, so reading and search work offline
Search every account at once, with from:, subject:, has:attachment and date ranges
Conversations grouped into threads
Rules, smart mailboxes, VIPs and coloured flags
Junk filtering that learns from you, on your PC
Undo with Ctrl+Z, and a moment to take back a sent message
Import from Thunderbird, mbox and Outlook .pst; export to mbox or .eml
Light and dark themes, Mica and Snap Layouts
Notifications, a taskbar badge and a jump list
Opens mailto: links and .eml files
Full keyboard control and screen-reader support
No account, no adverts, no telemetry
```

(Third line: the same condition as the description.)

### Screenshots

In `store/screenshots/`, 3200 × 1800 each. All of the mail in them is invented, written by
`src-tauri/src/bin/storedemo.rs`; none of it is anybody's. Upload in this order — the first is the
one search results show:

| Order | File | Caption |
| --- | --- | --- |
| 1 | `02-conversation-light.png` | Your accounts, your messages and the whole conversation, side by side. |
| 2 | `01-conversation-dark.png` | Dark mode, following Windows or set by you. |
| 3 | `04-compose-dark.png` | Write in a separate window, with the message you are answering below. |
| 4 | `05-search-light.png` | Search every account at once, even offline. |
| 5 | `03-newsletter-dark.png` | Newsletters and receipts look as their senders designed them, in a sandbox that cannot run code. |

To take them again — after a visible change, or for a new version — close Halcyon and run
`node tools/store-screenshots.cjs`. It writes the invented store, starts the Store build against
it with every path redirected away from your own mail, and puts back your window position after.

### Store logos

In `store/logos/`:

| Slot | File |
| --- | --- |
| 1:1 Box art (1080 × 1080) | `BoxArt-1080x1080.png` — the designer's mark on the brand red, at the 300 px icon's proportions |
| 1:1 App tile icon (300 × 300) | `AppTileIcon-300x300.png` — the designer's own Store icon |

Optional for a Windows desktop app, but without box art the Store falls back to the package's
small logo, which reads poorly at listing sizes.

### Search terms

Seven at most. Other products' names are left out on purpose: keyword-stuffing with someone else's
brand is a listing violation.

```text
email
mail
email client
IMAP
inbox
private email
offline email
```

### Copyright and trademark info

```text
© 2026 Vishal Singh
```

### Developed by

```text
Vishal Singh
```

(Shown publicly. Use the publisher name instead if you would rather not.)

### Additional system requirements

```text
Windows 11, 64-bit. The WebView2 runtime, which Windows 11 already includes.
```

## 6. Submission options

### Restricted capabilities — why `runFullTrust`

```text
Halcyon is a desktop (Win32) email client packaged with the Desktop Bridge. It needs runFullTrust to connect to the user's own IMAP and SMTP servers over TLS sockets, to keep the user's mail in a local SQLite database under %LOCALAPPDATA%, to store passwords and OAuth tokens in Windows Credential Manager, and to open the user's default browser for OAuth sign-in, which Google and Microsoft require instead of an embedded web view. It requests no other restricted capability, runs no background services, and has no server of its own.
```

### Notes for certification

Replace the bracketed parts with the test account from step 3 above.

```text
Halcyon is an email client, so testing it needs a mailbox. Please use this account, created for certification:

  Provider:  [Yahoo Mail]
  Email:     [address]
  Password:  [app password]

To test:
1. Start Halcyon. With no account yet, the account assistant opens.
2. Choose [Yahoo Mail], enter the address and password above, and continue. Halcyon tests the connection and adds the account.
3. The Inbox appears within a few seconds. Select a message to read it.
4. Press Ctrl+N, or the pencil button, to write a message. Send one to the same address to watch it arrive.
5. Type in the search field at the top right to search.

Gmail accounts sign in through the system browser (OAuth); the account above needs no browser.

Halcyon has no server and collects no data. Everything it stores is on the PC, under %LOCALAPPDATA%\com.uniki.halcyon, and passwords go to Windows Credential Manager. No purchase, sign-up or account other than the mailbox above is needed.
```

---

## After it is live

- **Updates:** bump the version, run `tools/make-msix.ps1`, upload, submit. Windows updates
  installed copies by itself.
- **Staged rollout and package flights** are in Partner Center for every later version — use the
  rollout percentage for anything risky.
- **Partner Center's analytics** are Microsoft's aggregate install and crash figures, not anything
  the app sends; `PRIVACY.md` already says so.
