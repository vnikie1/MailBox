# Privacy policy

**Halcyon, version 1.2. Last updated 3 October 2026.**

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

| Connection                                        | When                                                                                                         | What it carries                                                 |
| ------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------- |
| Your email provider's IMAP server                 | While the app is running                                                                                     | Your username, your password or access token, and your mail     |
| Your email provider's SMTP server                 | When you send                                                                                                | The message and its recipients                                  |
| Your provider's sign-in page, in your own browser | When you add an OAuth account                                                                                | Whatever your provider's sign-in requires                       |
| `autoconfig` records for your email domain        | Once, when adding an account, to find server settings                                                        | Your email domain — for example `example.com`, not your address |
| `github.com`                                      | When you open **Settings → General**, or press **Check for updates**. Never from the Microsoft Store version | Nothing about you. A request for a small public file            |
| Servers named in the messages you read            | Only if you allow images to load                                                                             | See **Remote images** below                                     |

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

## Google accounts

Adding a Google account asks Google for one permission, which Google's own screen describes as
_"Read, compose, send and permanently delete all your email from Gmail"_ — the scope
`https://mail.google.com/`. Halcyon asks for that one and no other, because it is the only one
Google's IMAP and SMTP servers accept.

Halcyon uses it for what a mail client does, at your direction: downloading your mail to your
computer so that you can read and search it, sending what you write, and moving, flagging and
deleting messages when you ask. What it receives from Google is stored only on your computer, in
the places listed above, with the sign-in token in Windows Credential Manager.

Halcyon does not send Google user data to us or to anyone else — none of it reaches us, so no one
here can read it — does not use it for advertising, and does not use it to develop, improve or
train artificial-intelligence or machine-learning models. You can withdraw Halcyon's access at
any time at <https://myaccount.google.com/permissions>. To delete what it has stored, delete the
folders listed above; the uninstaller offers to do it for you.

Halcyon's use and transfer to any other app of information received from Google APIs will adhere
to the
[Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy),
including the Limited Use requirements.

## Remote images

Many messages contain images loaded from the sender's server. Requesting one tells that sender
you opened the message, roughly when, and the network address you opened it from. This is the
read receipt nobody agreed to, and it is how commercial mail tracks you.

Halcyon loads remote images **automatically by default**, because a mail client that shows
broken pictures is one people stop using. This is the one default in the application chosen
against the security advice, and it is a setting rather than a decision made for you:
**Settings → Privacy → "Show images in messages automatically."** Turning it off shows a banner
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

Opening **Settings → General**, where the Updates section is, asks GitHub once whether there is
a newer version, and so does pressing **Check for updates**. Each is one request for one small
public file. Your address is visible to GitHub, as it is to any web server you contact; nothing
identifying you, your accounts or your mail is sent, and nothing checks at any other time —
not at start-up, and not on a timer. The Microsoft Store version does not do this at all: the
updater is not in it, and the Store handles its own updates.

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
