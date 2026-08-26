# Phase 5 — verification record (part one)

Roadmap gate (`docs/04-roadmap.md`): *against Dovecot-in-Docker **and** a real Gmail account —
cold sync of a 50k-message mailbox completes and stays correct; killing the network mid-sync
recovers without duplicates or loss; flags changed on another device appear within 5s;
`UIDVALIDITY` reset is handled; a 12-hour soak shows no leak and no connection storm.*

**Status: in progress. Not claimed as passed.** Five of the nine numbered items in
`docs/06` Phase 5 are built and one is partial; the exit gate cannot be run at all yet
because it needs IDLE and incremental sync, neither of which exists. §4 is the honest split.

What *is* established, and matters most, is that the approach works against a real provider:
a real Gmail account authenticates, lists 46 mailboxes and renders its Inbox with correct
senders, subjects and dates.

---

## 1. Verified

| Check | Command | Result |
|---|---|---|
| Formatting | `npm run format:check` | pass |
| Lint | `npm run lint` | 0 problems |
| Design-token rule | `npm run lint:css` | 0 problems |
| Types | `npm run typecheck` | pass |
| Frontend tests | `npm run test` | 94 / 94 |
| Rust tests | `npm run rust:test` | 225 / 225 (was 119) |
| Secret-leak gate | `cargo test --test secrets` | 6 / 6 |
| Rust lint | `npm run rust:clippy` | 0 warnings |
| End-to-end | `npx playwright test` | 44 / 44 |
| Live account | `cargo test --test live_gmail -- --ignored` | authenticates, lists 46 mailboxes |

`npm run verify` exits 0.

---

## 2. Against the real account

`tests/live_gmail.rs`, run against the configured Gmail account:

```
=== vnikie1@gmail.com (google) ===
  imap imap.gmail.com:993
  [   0.00s] access token obtained (refreshed: false)
  [   0.01s] tcp connected
  [   0.04s] tls established
  [   0.26s] greeting: Ok "Gimap ready for requests from ..."
  [   0.91s] AUTHENTICATED
  [   1.21s] capabilities: IDLE MOVE CONDSTORE ID ESEARCH IMAP4rev1 X-GM-EXT-1
             LIST-EXTENDED LIST-STATUS UIDPLUS QUOTA CHILDREN NAMESPACE
             COMPRESS=DEFLATE UNSELECT XLIST ENABLE UTF8=ACCEPT LITERAL-
             SPECIAL-USE APPENDLIMIT=35651584
  [   1.55s] LIST returned 46 mailboxes
```

Every extension the remaining Phase 5 work needs is present: `IDLE` for push, `CONDSTORE`
for incremental deltas, `X-GM-EXT-1` for thread ids, `MOVE` and `UIDPLUS` for the operation
drain. `QRESYNC` is absent, as expected — Gmail does not offer it, which is why the windowed
`FLAGS` fallback in `docs/03` §5 is not optional.

Then through the engine, in the running window: the account's Inbox rendered with real
senders, real subjects and correct Today / Yesterday / Previous 7 Days grouping.

**Not measured, and not claimed:** the 50k cold-sync time, the mid-sync network kill, the
five-second flag propagation, a forced `UIDVALIDITY` reset, and the 12-hour soak. Those are
the exit gate, and four of the five need code that is not written.

---

## 3. What was built

**`backoff`** — 1s → 300s with ±25% jitter. The jitter is not decoration: every account
fails at the same instant when a network drops, and without it they retry at the same
instant too. Tests assert that two accounts failing together do not retry together, and that
no delay is ever short enough to be a tight loop.

**`threading`** — JWZ, **tests written first**, as `docs/06` Phase 5 requires. The tests live
in their own file so the order is visible in the repository rather than merely asserted here.

Union-find rather than JWZ's container tree, because this app renders a flat conversation and
only needs the partition. Two properties come free: a bridging message merges two threads by
construction, and a reference cycle cannot loop because there is no traversal to get stuck
in — and real mail contains cycles.

**`session`** — connect, authenticate, capabilities. Read after authentication, because
servers advertise a different set once they know who is asking.

**`mailboxes`** — `LIST`, then RFC 6154 attributes, then name heuristics. Duplicate role
claims resolved deterministically.

**`envelope`** — RFC 2047 decoding including legacy character sets, and `INTERNALDATE`
parsing. The list orders by the server's clock, not the sender's: a machine with a wrong date
would otherwise sit permanently at the top of the mailbox.

**`fetch`** — the envelope fetch is a raw command so Gmail's `X-GM-THRID` and `X-GM-MSGID`
arrive with the standard attributes in one round trip; `async_imap::Fetch` has no accessor
for them. Everything uses `BODY.PEEK`, tested for, because the non-peek form would mark an
entire mailbox read on first sync — irreversibly, since the flags go to the server.

**`persist`** — idempotent on `(mailbox_id, uid)`. Threading and counts are recomputed rather
than incremented, so a replayed batch cannot inflate them.

**`engine`** — connect, discover, newest 500 of the Inbox, then backfill in 500s.

---

## 4. Where Phase 5 actually stands

`docs/06` Phase 5 numbers nine items. Honestly:

| # | Item | State |
|---|---|---|
| 1 | Mailbox discovery, role inference | **done** |
| 2 | Initial envelope sync + backfill | **done** |
| 3 | Lazy body fetch, prefetch 3, `.eml` cache | **not done** — `fetch::body` exists and is unused; nothing caches, nothing prefetches. The reader shows a subject and no body. |
| 4 | IDLE on a dedicated connection | **not done** |
| 5 | CONDSTORE/QRESYNC incremental; windowed fallback | **not done** — capabilities are detected and `MODSEQ` is fetched and stored, but nothing reads it back. Every sync is currently a full pass. |
| 6 | `UIDVALIDITY` → drop and re-sync | **built, not exercised** — detected, and the drop path is unit-tested; no live reset has been forced. |
| 7 | JWZ threading + Gmail `X-GM-THRID` | **done** |
| 8 | `pending_op` drain with backoff | **not done** — Phase 3 enqueues ops; nothing drains them, so local flag changes never reach the server. |
| 9 | Per-account error surfacing with a retry-at time | **partial** — the core emits `account:error` with `retryInSeconds` and `needsReauth`, and `useSync` collects it. No banner renders it yet. |

Also outstanding:

- **The connection pool does not exist.** docs/03 §5 specifies 2–4 pooled connections per
  account plus one parked on IDLE. Today there is exactly one connection, used serially.
- **No in-process IMAP test server.** There is no Docker on this machine, so the exit gate's
  Dovecot half cannot be run as written. An in-process server is the intended substitute —
  and a better one for CI, since it can force `UIDVALIDITY` resets and mid-sync disconnects
  on demand. Until it exists, the deterministic half of the gate has nothing to run against.
- **Gmail's `All Mail` is skipped entirely.** Correct for now — every message appears there
  as well as in its labels, and syncing both would double the mailbox — but it means archived
  mail with no other label is currently invisible. Labels-as-mailboxes is the real answer.
- **The body column is never populated**, so the message list shows no preview text. Envelope
  sync cannot produce one; it arrives with item 3.

---

## 5. Incidents

- **Every OAuth sync hung for exactly sixty seconds, and the cause was an unread greeting.**
  `async_imap::Client::new` does not consume the server's opening `* OK ... ready`, and
  nothing in the crate's API suggests it must. `authenticate` then reads the greeting as the
  answer to the command it just sent, and waits for a continuation the server will never
  send; the server waits for a client that has stopped talking. TLS established in 40ms, then
  silence until a timeout.

  What made this expensive was the diagnosis, not the fix. The engine logged "sync starting"
  and then a sixty-second timeout — a boundary around the *whole* handshake, which is four
  round trips wide. `tests/live_gmail.rs` puts a boundary around each step and named the
  failing one on the first run. The general lesson: **an error boundary that spans several
  operations tells you almost nothing; narrowing it is the diagnosis.**

- **The log line that would have explained it was after the call that hung.** "sync started"
  was logged once `connect()` returned, so an account stuck in the handshake produced no line
  at all and looked as though it had never been attempted. The first stretch of diagnosis
  went looking for why account 4 was being skipped. It was not being skipped.

  **A log line after the risky call only tells you about the runs that succeeded.**

- **Two more real bugs were found by reading the dependency's source rather than guessing.**
  `async_imap` base64-encodes the authenticator's return value itself, so encoding it here
  sent base64 of base64. And a failed XOAUTH2 exchange needs an empty reply to Google's error
  continuation or both ends wait. Neither would have been found by staring at our own code;
  both were plain in forty lines of `client.rs`.

- **One broken account starved three working ones.** Configuration errors were treated as
  retryable and the engine held a single global lock, so three demo accounts with no IMAP
  host each backed off through five attempts before the real account was reached. Ninety
  seconds of a cold start spent on accounts that could never succeed. Fixed twice over:
  configuration errors are non-retryable, and the lock is per account.

- **The mail arrived and the counts said zero.** The first successful sync wrote every
  message correctly, and the sidebar badge and the list header both showed nothing, because
  `write_batch` never refreshed the cached `unread_count` / `total_count`. Visible instantly
  in the running window; invisible to 219 passing tests. Fourth phase running that this has
  been true.

- **`tauri dev` restarted the app mid-diagnosis** when a source file was saved, which is
  correct behaviour and briefly very confusing: a database growing by 3MB with no apparent
  cause was the running app picking up the greeting fix and syncing for real.
