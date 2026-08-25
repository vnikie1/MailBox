# Risks, blockers and legal notes

Read this **before** Phase 4. Several items here have long lead times and will block shipping
if you discover them late.

---

## 1. Cloning Apple's design

Building a Windows app that _works and feels_ like macOS Mail is fine. Copying Apple's actual
assets is not. Concretely:

| Do                                                                        | Don't                                                                     |
| ------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| Reproduce layout, spacing, interaction patterns, information architecture | Ship SF Pro, SF Mono, or SF Symbols — licensed for Apple platforms only   |
| Use Apple's published system colour _values_ (facts, not creative works)  | Copy Apple's app icon, stamp/envelope artwork, or any bundled image       |
| Say "inspired by the macOS Mail experience"                               | Call it "Apple Mail for Windows" or use Apple trademarks in the name/icon |
| Use Inter + Lucide as substitutes                                         | Trace SF Symbols glyphs into your own SVGs                                |

For personal use none of this matters. For public distribution, all of it does. The
substitutions in `01-macos-mail-analysis.md` §10 and §13 are chosen to make this a non-issue
without losing fidelity.

**Naming:** pick something neutral. The codename used throughout these docs is a placeholder.

---

## 2. Gmail OAuth — the biggest scheduling risk

To read mail via IMAP you need the `https://mail.google.com/` scope, which Google classifies
as **restricted**. That means:

- Up to **100 test users** with no review — fine for personal use and early development.
- Beyond that: OAuth verification **plus a CASA (Cloud Application Security Assessment)** Tier 2
  or 3 review, performed by an approved third-party assessor, renewed annually. Budget
  **6–12 weeks and a four-figure cost** for a public launch.

**Mitigations:**

- Start the verification process at Phase 4, not Phase 11.
- Ship a "bring your own OAuth client" option in Settings for advanced users — sidesteps the
  problem entirely for self-hosters and for you during development.
- Support Google **app passwords** as a fallback where the account allows them.

Also: Google requires OAuth to happen in the **system browser**, not an embedded WebView.
Using a WebView will get your client blocked with `disallowed_useragent`.

---

## 3. Microsoft / Outlook

- Basic authentication for IMAP/SMTP is **disabled** on Microsoft 365 and consumer Outlook —
  OAuth is mandatory. Register an app in Entra ID with `IMAP.AccessAsUser.All`,
  `SMTP.Send`, `offline_access`, `User.Read`.
- Consumer accounts require the `consumers` or `common` tenant.
- Corporate tenants may block third-party IMAP entirely. For those, the only real path is
  **Microsoft Graph**, which is a different API surface — treat it as post-v1 and set
  expectations accordingly.
- SMTP AUTH is disabled by default per-mailbox on many tenants; surface a specific error
  message rather than a generic auth failure, because the user's admin has to fix it.

---

## 4. iCloud

- No third-party OAuth. Users must generate an **app-specific password** at appleid.apple.com,
  which requires 2FA on the account.
- Build a guided flow with the exact steps and a deep link. This is the single most common
  support issue for every third-party mail client.
- Hosts: `imap.mail.me.com:993` (TLS), `smtp.mail.me.com:587` (STARTTLS).

---

## 5. IMAP reality checks

| Trap                                            | Handling                                                                              |
| ----------------------------------------------- | ------------------------------------------------------------------------------------- |
| Servers lie about `UIDNEXT`                     | Always verify with a `UID SEARCH` after a suspicious delta                            |
| `UIDVALIDITY` changes without warning           | Full re-sync of that mailbox; never attempt a merge                                   |
| Gmail's `All Mail` duplicates everything        | Key on `X-GM-MSGID`; exclude `All Mail` from unified counts                           |
| Yahoo/AOL throttle aggressively                 | Cap to 2 connections, add jitter, honour `[THROTTLED]`                                |
| Some servers cap connections at 3–5 per account | Make the pool size configurable per account, default 3                                |
| Broken MIME is the norm, not the exception      | `mail-parser` is chosen specifically because it is lenient; never `unwrap()` on parse |
| Missing or duplicate `Message-ID`               | Synthesise a stable one from hash(from + date + subject + size)                       |
| Non-UTF8, mislabelled charsets                  | Detect with `chardetng`, fall back to windows-1252                                    |
| IDLE dies silently behind NAT                   | Re-issue every 25 min and add an application-level heartbeat                          |
| Huge mailboxes (500k+)                          | Never `SELECT` and fetch all; always window by UID                                    |

---

## 6. Security obligations

You are building software that holds someone's entire private correspondence and the
credentials to their identity provider. Non-negotiables:

- Credentials in Windows Credential Manager only. Never in the DB, config, logs or telemetry.
- Message bodies rendered in a sandboxed iframe with no scripting. Ever.
- Remote content blocked by default; proxied when loaded.
- Certificate validation **on** with no user-visible "ignore this" for public hosts. (A
  per-account "accept this self-signed cert" flow with a fingerprint pin is acceptable for
  self-hosted servers.)
- Consider encrypting the SQLite DB at rest (SQLCipher) behind an optional app password — the
  DB is otherwise readable by anything running as that user.
- Have a plan for responsible disclosure before you publish.

---

## 7. Code signing and distribution

- Unsigned installers get SmartScreen-blocked and effectively cannot be distributed.
- OV certificates now require hardware key storage (eToken or cloud HSM). **Azure Trusted
  Signing** is the cheapest practical route (~$10/month) but requires an org with 3+ years of
  verifiable history, or an individual validation path.
- EV certificates give instant SmartScreen reputation but cost significantly more.
- Budget 1–3 weeks of lead time for identity validation. Start this during Phase 9.

---

## 8. Scope traps

Things that look small and are not. Time-box or defer each of these deliberately:

1. **Rich-text editing.** Contenteditable is a swamp. Use Lexical, restrict the feature set to
   exactly what Mail's format bar offers, and do not accept "just add tables" requests.
2. **Rendering other people's HTML email.** Every newsletter is a 2003-era table layout with
   inline CSS. Expect a long tail of rendering bugs.
3. **Threading.** JWZ looks simple and has many edge cases. Write the tests first.
4. **Calendar invites.** Parsing `.ics` and doing RSVP correctly is a project in itself.
5. **Exchange.** If corporate mail is a requirement, it is a second sync backend, not a flag.
6. **Search relevance.** FTS5 gives you matching; ranking that feels like Top Hits is tuning
   work you should budget a week for on its own.

---

## 9. Decisions to make before Phase 0

| Question                                   | Default if you don't decide                                                                                                        |
| ------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------- |
| App name and icon                          | Placeholder codename; blocks nothing until Phase 11                                                                                |
| Personal use only, or public distribution? | Assume public — it changes §1, §2 and §7                                                                                           |
| Which providers must work at v1?           | Gmail + generic IMAP; Outlook and iCloud next                                                                                      |
| Encrypt the local DB?                      | No, with an opt-in setting later                                                                                                   |
| Open source?                               | Changes the OAuth client-secret story materially — a distributed secret is not a secret, so an OSS build must ship BYO-credentials |
| Paid or free?                              | Free/local-first is the strongest differentiator against Mailbird and eM Client                                                    |
