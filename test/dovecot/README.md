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
./certs.sh mac-studio.local 192.168.1.15   # a server certificate, and a CA that can sign nothing else
docker compose up -d
./seed.sh 50000                   # the mailbox the gate asks for
```

Then, on the machine running Halcyon, **trust the CA** (`certs/ca.crt`):

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File testdovecot	rust-ca.ps1 -Path <copy of ca.crt>
```

That step is deliberate and worth being explicit about. Halcyon validates certificates
against the system trust store with no user-visible bypass (docs/05 §6), so a test server has
to present a certificate that genuinely validates. The alternative — a code path that skips
validation "only in tests" — is exactly the kind of thing that ships, and a mail client that
can be talked out of checking a certificate is not one worth writing. Trusting one CA that can
vouch for this server and nothing else, on one development machine, is smaller, visible in the
certificate store, and reversible.

## The certificate

`certs.sh` makes a CA, signs one server certificate with it, and deletes the CA's key. The CA
also carries critical name constraints listing only the names and addresses it was given, so
even a copy of its key could not issue a certificate for another site. The server certificate
lasts 397 days and the CA one day longer.

The current pair was made on 2026-09-17 and **expires on 2027-10-19**. After that every rig
test fails in the TLS handshake (`SEC_E_CERT_EXPIRED`, "The received certificate has
expired"). To renew:

```sh
# on the Docker host
./certs.sh mac-studio.local 192.168.1.15
docker exec halcyon-dovecot doveadm reload
```

```powershell
# on the Windows machine, with certs/ca.crt copied across; Windows asks to confirm each change
powershell -NoProfile -ExecutionPolicy Bypass -File testdovecot	rust-ca.ps1 -Path .ca.crt
```

`trust-ca.ps1` trusts the new CA for the current user and removes every other
"Halcyon Test CA" that user trusts. It refuses a CA without name constraints.

Why it is no longer a 30-day CA: the first `certs.sh` made one that lived a month and kept
its key, which was both a key on disk that the development machine would accept for any site
and a rig that stopped working every month. The constraints were checked against Windows'
own chain engine, with the new CA as its only root, before it was trusted: the rig's four
names validate, any other name fails with `CERT_E_CN_NO_MATCH`, and a certificate naming a host
outside the constraints fails with `CERT_TRUST_HAS_NOT_PERMITTED_NAME_CONSTRAINT`.

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
