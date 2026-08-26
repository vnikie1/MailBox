# The Phase 5 exit gate rig

docs/04 §Phase 5 asks for the sync engine to be proved **against Dovecot-in-Docker _and_ a
real Gmail account**. Gmail has covered most of the ground already; this is for the parts it
cannot.

Three things need a server we control:

- **QRESYNC.** Gmail advertises CONDSTORE and not QRESYNC, so that whole path has never run.
- **A `UIDVALIDITY` reset.** There is no way to make Gmail renumber a mailbox on request, and
  "drop and re-sync" is the recovery most likely to be wrong and least likely to be noticed.
- **Rudeness.** Killing the network mid-sync, refusing connections, and holding an IDLE open
  for twelve hours are all things to do to a server you own.

## Running it

On the machine hosting Docker:

```sh
./certs.sh halcyon-test.local     # a throwaway CA and a server certificate
docker compose up -d
./seed.sh 50000                   # the mailbox the gate asks for
```

Then, on the machine running Halcyon, **trust the CA** (`certs/ca.crt`).

That step is deliberate and worth being explicit about. Halcyon validates certificates
against the system trust store with no user-visible bypass (docs/05 §6), so a test server has
to present a certificate that genuinely validates. The alternative — a code path that skips
validation "only in tests" — is exactly the kind of thing that ships, and a mail client that
can be talked out of checking a certificate is not one worth writing. Trusting one 30-day CA
on one development machine is smaller, visible in the certificate store, and reversible.

## The account

| | |
|---|---|
| Address | `tester@halcyon.test` |
| Password | `halcyon-test-only` |
| IMAP | the Docker host, port **9993**, TLS |

The password is in `users/passwd` in plaintext and is meant to be. It guards a disposable
container of generated mail on a LAN, and pretending otherwise by hashing it would suggest it
protects something.

## What the seed data is for

Fifty thousand identical messages would prove nothing. `seed.sh` produces threads of twenty
with real `In-Reply-To`/`References` chains, a spread of dates so keyset pagination is
exercised over a real range, HTML messages with quoted replies and tracking numbers, and a
proportion with the malformed headers and missing charsets that real mail is full of.

## Resetting

`docker compose down -v` destroys the volume, which is how the `UIDVALIDITY` test starts from
nothing. The volume is named for that reason.
