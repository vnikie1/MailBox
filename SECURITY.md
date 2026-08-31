# Reporting a security problem

> Published at <https://vnikie1.github.io/halcyon-mail/security.html>.

Halcyon holds people's entire private correspondence and their mail passwords. A vulnerability
here is serious, and a report about one is welcome — including one that shows something claimed
in [PRIVACY.md](PRIVACY.md) is not true.

## How to report

**Email vnikie1@gmail.com** with `SECURITY` in the subject line.

If you prefer, open a
[GitHub security advisory](https://github.com/vnikie1/MailBox/security/advisories/new), which is
private until published.

**Please do not open a public issue for a security problem**, and please do not post a working
exploit publicly before there is a fix. Everything else — partial findings, uncertain findings,
"this looks wrong but I could not exploit it" — is worth sending.

Useful in a report, though none of it is required:

- What an attacker could do, and what they would need to start with.
- The version, from **Settings → Advanced**, and your Windows version.
- Steps to reproduce, or a message file that triggers it.
- Whether you would like to be credited, and under what name.

## What happens next

This is a one-person project, so these are honest commitments rather than an enterprise SLA:

|                                             |                                                                             |
| ------------------------------------------- | --------------------------------------------------------------------------- |
| Acknowledgement                             | Within 3 days                                                               |
| An assessment, and whether it will be fixed | Within 14 days                                                              |
| Fix for a serious issue                     | As fast as it can be built, tested and signed                               |
| Public disclosure                           | After a fix ships, or 90 days, whichever comes first — sooner if you prefer |

You will be credited by name unless you would rather not be. There is no bounty programme; there
is no money in this project to fund one.

## What is in scope

Anything in this repository, and anything an installed copy of Halcyon does. The parts most
worth your attention:

- **Message rendering.** Bodies are hostile input. They are sanitised in the Rust core and shown
  in a sandboxed frame that should not be able to run script, reach the filesystem, call the
  application's own commands, or make a network request before the user allows images. A way
  around any of that is a real finding.
- **Credential handling.** Passwords and OAuth tokens should exist only in Windows Credential
  Manager. Finding one in the database, a log, a crash report, an error message or a temporary
  file is a real finding.
- **The IPC surface.** The web layer can only call the commands listed in
  `src-tauri/capabilities/default.json`. A way to reach anything beyond that list — or to make a
  command act on a file or account it was not given — is a real finding.
- **Transport.** Certificate validation must be on for every public host, with no way for a user
  or a server to turn it off. Anything that downgrades or bypasses TLS is a real finding.
- **Import and update paths.** A crafted mbox file, `.eml` file or update manifest that reads or
  writes outside the directories it was given, or that gets code to run, is a real finding.

## What is out of scope

- Anything requiring an attacker who already has your Windows account. Halcyon does not defend
  against that and does not claim to; see the note about disk encryption in PRIVACY.md.
- The mail database being unencrypted at rest. Known, documented, and true of every desktop mail
  client.
- Vulnerabilities in Windows, WebView2, or a dependency — report those upstream. If a dependency
  advisory affects Halcyon specifically, do tell us.
- Spam, phishing or malware that arrives _in_ mail. That is your provider's filtering and your
  own judgement; Halcyon renders it safely, it does not adjudicate it.
- Reports produced by an automated scanner with no demonstration that anything is actually
  reachable.

## Supported versions

The current release only. Halcyon updates in place, so "upgrade" is the fix for everything.
