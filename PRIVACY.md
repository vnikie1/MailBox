# Privacy policy

**Halcyon, version 1.0. Last updated 31 August 2026.**

> Published at <https://vnikie1.github.io/halcyon-mail/privacy.html>, which is the URL given to
> the Microsoft Store. This file is the source it is generated from; the two must not drift.

Halcyon is a mail client that runs on your computer. It has no server, no account, and no
business model that involves knowing anything about you.

This document says what leaves your machine and what does not. Every claim in it is checkable
against the source code, which is published for that reason.

---

## The short version

**Nothing about you or your mail is sent to the makers of this application, ever.** There is
nowhere for it to go. No analytics, no usage statistics, no crash reporting service, no
telemetry of any description.

Your mail goes to and from your own email provider, over an encrypted connection, exactly as it
would with any other mail client.

---

## What Halcyon connects to, and why

| Connection                                        | When                                                  | What it carries                                                 |
| ------------------------------------------------- | ----------------------------------------------------- | --------------------------------------------------------------- |
| Your email provider's IMAP server                 | While the app is running                              | Your username, your password or access token, and your mail     |
| Your email provider's SMTP server                 | When you send                                         | The message and its recipients                                  |
| Your provider's sign-in page, in your own browser | When you add an OAuth account                         | Whatever your provider's sign-in requires                       |
| `autoconfig` records for your email domain        | Once, when adding an account, to find server settings | Your email domain — for example `example.com`, not your address |
| `github.com`                                      | Only when you press **Check for updates**             | Nothing about you. A request for a small public file            |
| Servers named in the messages you read            | Only if you allow images to load                      | See **Remote images** below                                     |

There are no other connections. Nothing runs on a schedule except your own mail sync.

## What is stored, and where

All of it is on your computer. None of it is uploaded.

|                                                            |                                                               |
| ---------------------------------------------------------- | ------------------------------------------------------------- |
| Your mail, its attachments, and the search index           | `%LOCALAPPDATA%\com.uniki.halcyon`                            |
| Passwords and OAuth tokens                                 | Windows Credential Manager, protected by your Windows account |
| Logs, and crash reports if the app ever stops unexpectedly | `%LOCALAPPDATA%\com.uniki.halcyon\diagnostics`                |
| Window size, theme, and your settings                      | `%APPDATA%\com.uniki.halcyon`                                 |

**Passwords are never written to the database, to a configuration file, to a log, or into an
error message.** They are handed to Windows Credential Manager and referenced by a name that is
not itself a secret. There is an automated test that fails the build if any error type in the
application is capable of printing one.

**The mail database is not encrypted.** Anything running as you can read it, which is equally
true of every desktop mail client. BitLocker — Windows' full-disk encryption — is what protects
it if your computer is lost or stolen, and turning it on is worthwhile.

## Remote images

Many messages contain images loaded from the sender's server. Requesting one tells that sender
you opened the message, roughly when, and the network address you opened it from. This is the
read receipt nobody agreed to, and it is how commercial mail tracks you.

Halcyon loads remote images **automatically by default**, because a mail client that shows
broken pictures is one people stop using. This is the one default in the application chosen
against the security advice, and it is a setting rather than a decision made for you:
**Settings → Privacy → "Load images in messages automatically."** Turning it off shows a banner
on each message instead, and images load only when you ask.

Whatever the setting, message content is stripped of scripts and displayed in a sandbox that
cannot run code or reach your files.

## Crash reports

If Halcyon stops unexpectedly it writes a file describing what it was doing — the error and the
sequence of functions that led to it. That file stays on your computer.

**It is never uploaded.** There is no server to upload it to. You can read the reports under
**Settings → Advanced**, open the folder, delete them, or ignore them, in which case old ones
are eventually discarded automatically.

If you choose to send one to us to help with a problem, you do so yourself, deliberately, by
attaching the file — and you can read exactly what is in it first.

## Updates

Pressing **Check for updates** requests one small public file from GitHub. Your address is
visible to GitHub, as it is to any web server you contact; nothing identifying you, your
accounts or your mail is sent, and no request is made unless you press the button. The
Microsoft Store version does not do this at all — the Store handles its own updates.

For Store installs, Microsoft gives us aggregate install counts, ratings and crash figures. That
is Microsoft measuring their own platform rather than this application reporting on you, and it
cannot be turned off from here — but you should not have to discover it, so it is written down.

## Children

Halcyon is not directed at children and collects nothing from anyone, of any age.

## Your rights

Because we hold no data about you, there is nothing for us to disclose, correct, export or
delete. Deleting your data means deleting the folders listed above, and the uninstaller offers
to do it for you.

Mail held by your email provider is governed by that provider's own privacy policy, not this
one.

## Changes to this policy

Any change appears in this file and in the application's changelog, both of which are public and
have a full history. The version and date at the top say which one you are reading.

## Contact

Questions about this policy: **vnikie1@gmail.com**

To report a security problem, please follow [SECURITY.md](SECURITY.md) instead.
