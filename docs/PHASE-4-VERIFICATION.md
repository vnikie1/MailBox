# Phase 4 — verification record

Roadmap gate (`docs/04-roadmap.md`): *add a Gmail account by OAuth and an iCloud account by
app-specific password; both connect; credentials are in Credential Manager and nowhere else.*

Prompt gate (`docs/06-prompt-library.md` § Phase 4): *after a successful setup, grep the
database file, all logs and all config for the password or token and show it appears
nowhere.*

**Status: the security clause is passed and automated. The two live-account clauses are not
done, and cannot be from here** — they need a real Google OAuth client and a real Apple ID.
What is blocking each, and exactly what the user has to do, is in §5.

---

## 1. Verified

| Check | Command | Result |
|---|---|---|
| Formatting | `npm run format:check` | pass |
| Lint | `npm run lint` | 0 problems |
| Design-token rule | `npm run lint:css` | 0 problems |
| Types | `npm run typecheck` | pass |
| Frontend tests | `npm run test` | 94 / 94 (was 68) |
| Rust tests | `npm run rust:test` | 117 / 117 (was 40) |
| Secret-leak gate | `cargo test --test secrets` | 6 / 6 |
| Rust lint | `npm run rust:clippy` | 0 warnings |
| End-to-end | `npx playwright test` | 42 / 42 (was 30) |

`npm run verify` exits 0.

---

## 2. The exit gate: no secret on disk

`src-tauri/tests/secrets.rs` is the prompt's grep, written as a test so it runs on every
build rather than once by hand. It searches **raw bytes on disk**, not query results: SQLite
keeps freed pages until they are overwritten, so a value deleted from a row can sit in the
file long after `SELECT` stops returning it. The `-wal`, `-shm` and `-journal` sidecars are
searched too, because a write that was rolled back still went through the log.

| What it proves | How |
|---|---|
| A password account's password is not in the store | Creates a real account with a sentinel password on a real database file, checkpoints the WAL, searches every byte |
| Only the reference is | Asserts `halcyon:<email>` **is** present, and the password is not |
| An OAuth client secret is not in `setting` | Asserts the client *id* is present (it is not a secret — it is in the URL the browser is sent to) and the secret is not |
| Removing an account removes its secrets | `purge` then `exists` across all four kinds |
| Nothing formattable prints a secret | `Debug` on a struct *containing* a `Secret` — the realistic leak, since nobody logs a `Secret` directly |
| No error message can carry one | `CredentialError` and `OAuthError` rendered and searched |

**The control matters more than the rest.** `the_byte_search_finds_a_secret_that_really_is_in_the_file`
writes the sentinel into the database on purpose and asserts the search finds it. Without it,
every "not found" assertion above would pass just as happily against a search that could not
see anything — which is how this kind of test rots. `docs/PHASE-3-VERIFICATION.md` §6 records
a test that pinned nothing and passed; this is the answer to it.

The design is what makes the result cheap to hold. `credentials::Secret` has no `Display`, no
`Serialize` and a `Debug` that writes `Secret(redacted)`, so a secret cannot reach a log line
or the IPC boundary without someone writing `expose()` — a name chosen to be ugly and
greppable. Errors carry the credential *reference*, never the value. Standing rule 12 is a
property of the types rather than something to remember at each call site.

### The same grep, against the running app

The automated test proves the code paths. This was run afterwards against the real window,
with a real password typed into the real form and a real connection attempt made to
`imap.gmail.com:993` and `smtp.gmail.com:587` — both of which reached the server, completed
TLS, and rejected the sign-in.

```
READ OK  halcyon.db      117,690,368 bytes  matches=0
READ OK  halcyon.db-wal            0 bytes  matches=0
READ OK  halcyon.db-shm       32,768 bytes  matches=0
EBWebView (568 files)                       matches=0
app log                                     matches=0
repository working tree                     matches=0
```

**With the same control.** Searching the same 117MB database for `northgate` — a string that
is certainly in it — returns 200,011 hits. Without that, "0 matches" would be equally
consistent with a search that could not read the file.

The first attempt at this reported `exit=2` from `grep -r`, which is *error*, not *no match*:
some WebView2 files are locked while the app runs. Reporting that as a pass would have made
this whole section a lie, so each file is now read individually and says whether it was read.

**A real provider account has still not been through this.** A password reached the network
and left no trace, which is the property that matters — but no account was saved, because the
test correctly refused to save one that could not sign in. §5.

---

## 3. What was built

**`accounts::credentials`** — the only module that touches a secret. Four kinds (password,
refresh token, access token, client secret) as separate Credential Manager entries under the
service name `Halcyon Mail`, so revoking a token does not disturb a password and a user
auditing their credentials sees something they recognise. The reference is derived from the
email rather than the row id, so deleting and re-adding an account reuses the entry instead
of orphaning one — Credential Manager has no garbage collection.

**`accounts::provider`** — what is known before the user types anything. Google, Microsoft,
iCloud, **Yahoo** and Other, each with its servers, its auth kind, its scopes and the
sentence a user needs when signing in requires a step outside the app. Yahoo is capped at two
connections (docs/05 §5) because it throttles, and being throttled looks exactly like the app
being broken.

**`accounts::oauth`** — OAuth 2.0 with PKCE, hand-rolled, in the **system browser**. Three
things are not negotiable and each carries its reasoning in the source: the system browser
(docs/05 §2 — Google blocks embedded user agents, and it is the only arrangement where the
user can see the address bar); PKCE always (a desktop client cannot keep a secret, so PKCE is
what stops an intercepted code being redeemable); and a checked `state` parameter (without it
the loopback listener accepts a code from any page on the machine that can reach localhost).

The loopback listener binds `127.0.0.1:0` — a port the OS picks, so nothing can squat on a
fixed one — answers only `/callback`, ignores the browser's unprompted `/favicon.ico`, and
serves a small self-contained page so the last thing the user sees is not a blank tab reading
"ok". Google gets `access_type=offline` and `prompt=consent`, without which no refresh token
is issued and the account stops working in an hour with no explanation.

**`accounts::autodiscover`** — Mozilla's ISPDB, then the domain's own autoconfig, then SRV
records, then port probing, in that order because that is the order of decreasing confidence.
Every result says where it came from, and only a probed one asks the user to check it. The
autoconfig XML is hand-parsed rather than handed to an XML crate: the input is untrusted, and
a parser that only ever looks for four specific tags cannot be talked into expanding an
entity or fetching a DTD.

**`accounts::verify`** — the connection test, and the reason this phase is more than plumbing.
docs/04 asks for *a readable diagnostic report, not "authentication failed"*. Every mail
client on Windows can say authentication failed; almost none can say **which** of six things
that meant. The test runs as named steps — connect, secure, sign in, open Inbox — each
reporting pass, fail or **skipped**, because a report that stops at the failure looks like
the app gave up, and which stage was never reached is half the diagnosis.

The mapping from a server's refusal to a remedy is by provider *and* by response text. The
one that earns the module:

> `535 5.7.139 … SmtpClientAuthentication is disabled for the Tenant`
> → "Your organisation has turned off SMTP authentication for this mailbox. An administrator
> has to enable SMTP AUTH for it in the Microsoft 365 admin centre — **your password is not
> the problem**."

That is an administrator setting, per mailbox, and docs/05 §3 calls it out. Every other
client reports it as a failed sign-in, and the user changes their password over and over.

**`accounts::store`** — account rows, with the invariant that a row never holds a secret
enforced by the API rather than by discipline: `insert` takes no password. Removal reads the
credential reference *before* deleting the row, for the reason
`docs/PHASE-3-VERIFICATION.md` §6 records about permanent delete — after the row is gone
there is nothing left to resolve it from, and the secrets would stay in Credential Manager
forever with nothing to associate them with. Messages are deleted row by row rather than by
cascade so the FTS triggers fire; a cascade would leave a removed account's subjects in the
search index with no message behind them.

**The command surface** — thirteen commands, none of which returns a secret. There is
deliberately no `credential_get`.

**The UI** — an account assistant modelled on Mail's, with the flow as a reducer in
`model.ts` rather than as component state, because the interesting part is which step follows
which and that is worth testing directly. Provider first, then the address, then servers
**only if they are not already known**, then the test, then the report. Plus a settings pane
with reordering, per-account colour, a re-authenticate indicator, remove-with-purge, and the
bring-your-own-OAuth-client fields.

**Nothing is saved until the connection test passes.** An account row that cannot connect is
worse than no row: it appears in the sidebar, fails quietly, and working out why becomes the
user's problem.

---

## 4. Deviations, with reasons

- **Nothing is compiled in as an OAuth client, so Google and Microsoft are unusable until the
  user registers one.** docs/05 §2 offers "bring your own OAuth client" as a mitigation; here
  it is the only path. Embedding a client id and secret in a desktop binary means shipping a
  credential that anyone can extract, and it makes every user's mail access contingent on one
  registration surviving Google's review. The provider tile says
  "Needs setting up in Settings first" and Continue is disabled, rather than opening a browser
  onto a Google error page that reads as the app being broken.
- **STARTTLS on IMAP is reported as unsupported rather than implemented.** The code detects it
  and refuses with a sentence naming port 993. Reaching that branch means a hand-entered
  plaintext IMAP port, which docs/05 §6 does not permit against a public host anyway, and none
  of the five providers needs it. Implemented rather than silently falling back to an
  unencrypted connection, which is the failure that would matter.
- **The connection test speaks IMAP and SMTP directly rather than through `async-imap`.** A
  diagnostic wants the raw response line — that is the evidence — and a session-oriented
  client is built to hide it. `async-imap` is in `Cargo.toml` for Phase 5's sync engine, where
  IDLE, FETCH and CONDSTORE make it earn its place; it is unused by Phase 4.
- **The `select` for encryption is a native element.** The design system has no dropdown
  primitive, and building one inside a feature is what docs/02 §6 forbids. Styled to the field
  tokens so it does not look pasted in; a real `Select` primitive belongs in `src/ui`.
- **`socketType=plain` in an autoconfig document is read as the secure form for the port.**
  Honouring it would open an unencrypted connection to a public host silently, which docs/05
  §6 rules out. The connection test then says so if the port is wrong.
- **SRV lookup uses the system resolver only.** Falling back to a hard-coded public resolver
  would send the user's mail domain to a DNS server they did not choose — standing rule 16.
  If the machine's own configuration cannot be read, the step is skipped and probing runs.
- **`accounts_list` (Phase 3) and `accounts_detail` (Phase 4) both exist.** The first returns
  what the sidebar needs and is on the hot path; the second returns server settings and
  credential status for the settings pane. Merging them would put a Credential Manager read
  behind every sidebar render.

---

## 5. Not done, and what it needs

**The two live-account clauses of the roadmap gate are not met.** Both need something only
the user can supply, and neither can be faked without the test becoming a lie.

1. **Add a Gmail account by OAuth.** Needs a Google Cloud OAuth client of type *Desktop app*
   with the `https://mail.google.com/` scope, pasted into Settings → Accounts → sign-in
   applications. Until then the Google tile is correctly disabled.
2. **Add an iCloud account by app-specific password.** Needs an Apple ID with two-factor
   turned on and an app-specific password generated at appleid.apple.com. The assistant links
   straight to that page.

Once either is done, the manual half of the gate is: add the account, confirm the four
diagnostic steps pass, then run `cargo test --test secrets` and additionally grep
`%LOCALAPPDATA%\com.uniki.halcyon\` and the app's log output for the actual password or
token. The automated test proves the code paths carry no secret to disk; only a real account
proves that a specific provider's flow adds nothing.

Also outstanding:

- **Token refresh has never run against a live provider.** The five-minute margin, the
  `invalid_grant` → re-authenticate mapping and the "keep the stored refresh token when the
  provider omits one" rule are all unit-tested against constructed values. The last of those
  is the one that would silently log an account out hours later if it were wrong.
- **`account_test` for an OAuth account requires the account to have been added first**, since
  it tests with the stored token. The wizard never reaches that state — OAuth accounts are
  signed in and tested in one step — so it is only reachable from the settings pane, where
  re-testing an existing account is what it is for.
- **No Microsoft work-or-school account has been tried.** The two failures docs/05 §3 names —
  a tenant blocking IMAP, and SMTP AUTH disabled per mailbox — are mapped to remedies and
  unit-tested against their literal response strings, but the strings came from the spec, not
  from a server.
- **Removing an account does not stop an in-flight sync**, because there is no sync engine yet.
  Worth revisiting in Phase 5.
- **No `Select` primitive.** See §4.

---

## 6. Incidents

- **A tooltip-wrapped icon button did nothing when clicked, and had since Phase 1.**
  `withTriggerProps` called `getReferenceProps()` **without** passing the trigger's own props,
  then `cloneElement`'d the result over the trigger. Floating UI merges what it is given with
  what it generates, calling both handlers; `cloneElement` does not merge, it replaces. So the
  moment a primitive generated a handler of the same name — which `useDismiss({ referencePress: true })`
  does — the trigger's own `onClick` was silently dropped.

  Every `Tooltip`-wrapped `IconButton` in the app was affected, including the sidebar collapse
  toggle, which has been inert since Phase 2. It was invisible because no test clicked one: the
  Phase 1 primitive tests drive `Menu`, whose triggers have no `onClick` of their own, and the
  Phase 2 shell tests click plain `Button`s.

  Found by adding a Settings button to the sidebar header and watching an end-to-end test fail
  to open the sheet. Diagnosis took three probes — a synthetic listener proved the DOM click
  arrived, which ruled out an overlay and pointed at the React prop rather than at the event.

  Fixed by passing the trigger's own props *through* Floating UI's merger, with the ref applied
  after the merge — a ref passed through the merger does not reliably survive, and a trigger
  with no ref is one the focus manager cannot return focus to. That distinction cost one
  failing menu test before it was noticed.

- **The visual baselines cannot see a control this size.** `maxDiffPixelRatio: 0.002` allows
  about 2,500 differing pixels on a 1400×900 frame. A 28px icon button is under 800. Adding the
  Settings button to the sidebar header failed **no** baseline, in either theme, at either
  width.

  Worse, `npx playwright test --update-snapshots` rewrote nothing: in Playwright 1.62 that flag
  defaults to `changed`, and nothing had changed as far as the comparison was concerned.
  `--update-snapshots=all` was needed, and the result was confirmed by opening the PNG — which
  is the same thing `docs/PHASE-1-VERIFICATION.md` records about a blank baseline.

  The tolerance is not obviously wrong; it exists for font antialiasing. The conclusion is that
  **chrome that matters gets an assertion, not a screenshot**, which is what
  `tests/e2e/accounts.spec.ts` is for.

- **A credential test failed once and never again.** `stores_loads_and_purges` asserted that a
  purged entry was gone and found it present, in one run out of a dozen. It did not reproduce
  in isolation (3 runs) or in the full suite (3 runs), and `cmdkey /list` showed no leftover
  `halcyon` entries at all, so `purge` demonstrably works and left no residue.

  The cause was not identified. The one cross-run interference that was plausible has been
  removed: scratch references were keyed on the process id alone, and Windows reuses process
  ids, so a run that panicked before its `purge` would leave an entry a later run with the same
  id would find. They now carry a nanosecond timestamp as well. Recorded as unexplained rather
  than as fixed.

- **A new end-to-end test was flaky in my own hands, twice in three runs.** The reorder test
  read the account order with `allTextContents()`, which is a one-shot read with no retry,
  while the reorder round-trips through the store and a query invalidation. Replaced with
  `expect(locator).toHaveText([...])`, which retries — and which asserts the whole order rather
  than just the first row. Three consecutive clean runs.

  Noted because the first instinct was to blame parallelism; running with `--workers=1` failed
  too, which is what pointed at the test rather than at the suite. `docs/PHASE-1-VERIFICATION.md`
  has the same lesson in different clothes: waiting longer for the wrong thing never works.

- **Two Rust test failures from writing SQL against a remembered schema.** A test insert used
  `size_bytes` and `snippet`; the columns are `size` and `preview`, and `date_sent` is NOT NULL.
  Cheap to fix and worth recording, because the same guess in production code would have been a
  runtime error on a path no test covered.

- **Three bugs were visible only in the running window. Again.** All three passed 92 frontend
  tests, 117 Rust tests and 42 end-to-end tests without a murmur:

  1. **The account description field collapsed to its intrinsic width**, rendering "Northgate"
     as "North" in a box narrower than the word. `.identity` stretched; the field inside it
     did not.
  2. **"Add Your Other Mail Account Account".** `Add Your ${displayName} Account` against the
     one provider whose name already ends in the word. Now special-cased, with a test.
  3. **Choosing a provider moved the tiles under the cursor.** The setup note appeared below
     the list, the sheet grew, and because a sheet is vertically centred the tiles shifted
     up — so a second click could land on a different provider. Standing rule 6. The note's
     space is now reserved whether or not there is a note.

  This is the third phase running that the real window has found what the suite could not. It
  is no longer a lesson; it is a step in the gate.

- **Windows Firewall prompts on first run.** Opening the assistant raised
  "Do you want to allow public and private networks to access this app?". It was **declined**
  rather than allowed, because nothing Halcyon does needs inbound access: IMAP and SMTP are
  outbound, and the OAuth redirect listener binds `127.0.0.1`, which Windows Firewall does not
  filter. Declining had no effect on the connection test, which then reached Gmail and
  completed TLS on both ports.

  Worth carrying into `docs/07`: a signed, installed build should not surprise a user with a
  firewall prompt on first launch, and if one is unavoidable it should not be answered "Allow"
  by reflex for a permission the app does not need.

- **A clippy failure that improved the design.** `account_add_password` took nine parameters.
  Grouping them into an `AccountInput` struct is better than the original — the secret is now
  visibly a *separate* parameter from the struct, which is exactly the distinction the module
  is built around, and it cannot accidentally acquire a `Serialize` alongside the fields that
  have one.
