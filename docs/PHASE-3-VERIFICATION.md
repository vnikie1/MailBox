# Phase 3 — verification record

Roadmap gate (`docs/04-roadmap.md`): *100k-message DB; mailbox switch < 80 ms; list scroll
still 60 fps; FTS query < 120 ms; migrations run forward cleanly on a fresh and an existing
DB.*

Prompt gate (`docs/06-prompt-library.md` § Phase 3): *against the 100k seed — mailbox switch
< 80ms, list scroll still 60fps, FTS query < 120ms, cold start < 800ms. Show the
measurements.*

**Status: passed on every measured clause, with one clause measurable only in part** — cold
start, where the core's half is instrumented and the WebView's paint is not. See §2.

---

## 1. Verified

| Check | Command | Result |
|---|---|---|
| Formatting | `npm run format:check` | pass |
| Lint | `npm run lint` | 0 problems |
| Design-token rule | `npm run lint:css` | 0 problems |
| Types | `npm run typecheck` | pass |
| Frontend tests | `npm run test` | 68 / 68 |
| Rust tests | `npm run rust:test` | 40 / 40 |
| Rust lint | `npm run rust:clippy` | 0 warnings |
| End-to-end | `npx playwright test` | 30 / 30, across four display scales |
| App against the real store | `npm run app:dev` | opens the 100k database, lists, reads, no errors |

`npm run verify` exits 0.

---

## 2. The measurements

All against the seeded store: **100,000 messages, 42 mailboxes, 3 accounts**, printed by
`cargo run --bin seed`, which re-prints them on every run.

```
messages_page (first)            0.8 ms   (100 rows)
messages_page (20k deep)         0.6 ms   (100 rows)
mailboxes_tree (sidebar)         0.0 ms   (42 rows)
recount (maintenance only)      94.1 ms   (1 rows)
set_flags (50, incl. counts)    13.8 ms   (50 rows)
search                          26.6 ms   (50 rows)
```

| Budget (docs/03 §5) | Target | Measured |
|---|---|---|
| Mailbox switch | < 80 ms | **0.8 ms** — the list's first page; the sidebar reads cached counts and costs nothing |
| Search, 100k messages | < 120 ms | **26.6 ms** |
| Scroll | 60 fps | **median 16.8 ms/frame** (the Phase 2 measurement, still in the suite and still passing) |
| Idle RAM, 100k messages | < 300 MB | **57 MB** working set, measured on the running window |
| Cold start to painted UI | < 800 ms | **545 ms core-side** — see below |

**Keyset pagination, proven rather than asserted.** A page 20,000 rows deep costs 0.6 ms
against the first page's 0.8. That is the whole point of the cursor: `OFFSET 20000` would
re-walk twenty thousand rows to answer the same question.

**Cold start is measured in part, and the missing part is named.** 545 ms covers process
start to the window being shown, including opening and migrating the store — instrumented in
`lib.rs` and logged on every launch. It does *not* include the WebView's own load and first
paint, which cannot be timed from the core, and it is a **debug** build against a dev server.
An honest end-to-end figure needs a release build with bundled assets, which is Phase 11's
measurement. Recorded as partial rather than claimed as passed.

### Query plans

`EXPLAIN QUERY PLAN` for the three queries the prompt names, printed by the seed tool and
asserted in `db::tests_queries::the_hot_queries_use_their_indexes`:

```
messages_page:
  SEARCH message USING COVERING INDEX ix_msg_list (mailbox_id=? AND date_received<?)

unread count:
  SEARCH message USING COVERING INDEX ix_msg_unread (mailbox_id=?)

search:
  SCAN message_fts VIRTUAL TABLE INDEX 0:M5
  SEARCH message USING INTEGER PRIMARY KEY (rowid=?)
  USE TEMP B-TREE FOR ORDER BY
```

Both list queries are answered from a **covering** index with no temp b-tree, so the
`ORDER BY` is free. The temp b-tree in the search plan is the `bm25()` relevance sort, which
FTS5 cannot produce in order; it sorts only the matched rows, and 26 ms says so.

The test asserts the plan, not just the result. A plan that degrades to a table scan still
returns correct rows — nothing else in the suite would notice, and it would simply miss the
budget on a real mailbox.

---

## 3. What was built

**Schema and migrations.** `migrations/0001_initial.sql`, the whole of `docs/03` §3, applied
by a forward-only runner that embeds each file with `include_str!` and records it in
`schema_migration`. No `down`: a rollback that correctly un-migrates data is a fiction, and
the honest recovery is a fix-forward migration plus a restore.

**The `Db` handle**, deliberately asymmetric: writes go through a single actor on its own OS
thread, each job wrapped in a transaction; reads use an r2d2 pool on blocking threads. WAL
lets both run at once. Serialising the writer turns `SQLITE_BUSY` from an error every caller
must handle into a queue nobody has to think about.

**FTS5, external-content**, with triggers keeping it in step with `message`.

**Keyset pagination** on `(date_received, id)`, never `OFFSET`. The id in the cursor is not
decoration: timestamps collide constantly in mail, and a cursor on the date alone repeats or
skips the whole colliding run. Both implementations are tested for exactly that.

**The command surface** — `accounts_list`, `mailboxes_tree`, `messages_page`, `message_get`,
`thread_get`, `search`, `msg_set_flags`, `msg_move`, `msg_delete` — with `mailbox:changed`
and `messages:updated` events. The UI subscribes and invalidates query keys; nothing polls.

**Typed bindings.** Eleven types generated from the Rust structs into `src/lib/generated/`
by `cargo test`. A field renamed on one side and not the other is now a TypeScript error.

**The seed tool**, and **the UI swapped off fixtures onto the IPC contract**: TanStack Query
over the commands, an infinite query paging by cursor, and `src/store/mail.ts` reduced to
selection state, which is all it ever should have held.

---

## 4. Deviations, with reasons

- **FTS5 is external-content, not `content=''`.** `docs/03` §3 specifies contentless and
  `docs/06` Phase 3 specifies external-content; the two contradict each other. External
  content wins on a concrete point: a contentless table cannot be updated in place, and
  deleting from one means re-supplying every original column value in the DELETE trigger.
  Attachment names live in another table and are not available in `OLD`, so that trigger
  could not be written correctly. Three denormalised columns on `message` make the
  external-content form exact.
- **`ix_msg_list` is three columns, not two.** §3 gives
  `(mailbox_id, date_received DESC)`. Keyset pagination compares the *pair*
  `(date_received, id)`, and without `id` in the index that comparison cannot be answered
  from the index alone — the plan above would not say COVERING.
- **`i64` is declared to TypeScript as `number`, not `bigint`.** ts-rs defaults to `bigint`
  because 64-bit integers are not generally safe as JS numbers, but Tauri's IPC is JSON and
  `JSON.parse` produces `number` whatever the type says. Declaring `bigint` would describe a
  value that never arrives. Safe here because the only i64 fields are row ids and epoch
  seconds, both far inside `Number.MAX_SAFE_INTEGER`.
- **Only the commands with behaviour behind them exist.** `compose_*`, `rule_*`,
  `smartbox_*` and the rest of §4 arrive with their phases. Standing rule 18 forbids a
  command that returns a plausible shape and does nothing, and a `compose_send` with no SMTP
  is exactly that.
- **Conversation grouping is deferred, and the control was removed rather than left inert.**
  Phase 2 had an "Organise by Conversation" toggle over in-memory fixtures. Server-side
  grouping needs a thread-per-mailbox projection to stay inside the budget, and threading
  itself is the sync engine's job in Phase 5. A toggle that does nothing is worse than no
  toggle, so it is gone until there is something to toggle.
- **Sorting by anything but date is client-side, over the pages already loaded.** The store
  returns rows newest-first because that is the ordering `ix_msg_list` supports and the only
  one the keyset cursor can page. Sorting a hundred thousand messages by sender needs
  another index and another cursor —
  `docs/PHASE-0-VERIFICATION.md` §4 already flags this. Stated in `sort.ts` so nobody
  discovers it by surprise.

---

## 5. Not done

- **End-to-end cold start**, as above: needs a release build with bundled assets.
- **`thread_id` is never populated.** The seed leaves it NULL and `thread_get` falls back to
  the single message. Threading is Phase 5's, and inventing a one-message thread row per
  message would be structure that means nothing — the foreign key refused it when the seed
  tried, correctly.
- **The event bus is partial.** `mailbox:changed` and `messages:updated` are emitted and
  consumed; `sync:progress`, `messages:added/removed`, `outbox:changed` and `account:error`
  have no producer until Phases 5 and 7.
- **No integrity check yet.** Counts are maintained incrementally (§6), and the safeguard
  against drift is a test comparing them against a full recount. A background verifier that
  repairs drift in a real store is worth having before release.

---

## 6. Incidents

- **A full recount ran after every mutation — 84 ms per "mark as read".** `refresh_counts`
  recomputed `COUNT(*)` over the whole mailbox to update the cached badge, on every flag
  change, move and delete. Measured against a 70,000-message inbox it cost 84 ms, blocking
  the single writer for the duration. Standing rule 10 wants the local write instant.

  Replaced by snapshots of only the affected rows, before and after, with the difference
  applied to the cached counts. Marking 50 messages read now costs **13.8 ms including the
  count maintenance**. The full recount is kept for the seed tool and a future integrity
  check, and a test asserts the incremental path agrees with it — drift is the one risk this
  trade introduces.

  Found by measuring rather than by review: the timing report put `mailbox_counts` next to
  `messages_page` and the difference was three orders of magnitude.

- **A permanent delete never told the server to expunge.** `delete()` enqueued its
  `pending_op` *after* removing the rows, so the lookup that resolves which account the
  messages belonged to found nothing and silently wrote no op at all. The message would have
  been deleted locally and reappeared on the next sync.

  Deleted mail coming back is close to the worst thing this project can ship, and it was
  invisible until a test asked for the op — the delete itself worked perfectly. Accounts are
  now resolved before the rows go.

- **The schema caught two seed bugs, which is the schema working.** `UNIQUE(mailbox_id, uid)`
  rejected a seed that numbered UIDs from the batch counter rather than per mailbox; the
  foreign key on `thread_id` rejected an attempt to point every message at a thread row that
  did not exist. Both would have produced a plausible-looking database with wrong data.

- **Eleven generated files were written outside the repository.** `#[ts(export_to = ...)]`
  resolves relative to the *source file*, not the crate root, so a `../../../` path that
  looked correct from `src/db/` put them in the parent of the project directory. Removed, and
  the destination is now set once in `.cargo/config.toml` via `TS_RS_EXPORT_DIR` — at the
  repository root, because cargo reads that file from the invocation directory upward and
  every cargo command here runs from the root.

- **The app rendered nothing after the swap, from a stale Vite cache.** `@tanstack/react-query`
  had been a dependency since Phase 0 but was never imported until now, so Vite's pre-bundle
  cache held a version linked against a different React instance — presenting as "Invalid
  hook call" and an empty page. `rm -rf node_modules/.vite` fixed it. Worth knowing before
  spending an hour on a hook-rules audit, as nearly happened here.

- **Adding a second binary broke `tauri dev`.** With `halcyon` and `seed` both present,
  `cargo run` could not choose. Fixed with `default-run = "halcyon"`.

- **Two bugs were visible only in the running app, again.** The reader showed
  "No Message Selected" beside a selected row — the fallback above — and every message in the
  list carried the same timestamp, because the seed's integer date arithmetic collapsed every
  small roll to the same instant. Neither was caught by 138 passing tests. Phase 2's lesson
  holds: running the real window is a distinct verification activity.
