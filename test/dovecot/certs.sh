#!/bin/sh
# Generates a CA and a server certificate for the test Dovecot, then deletes the CA's key.
#
#   ./certs.sh <host> [extra name or address]...
#
# A CA rather than a bare self-signed certificate, because Halcyon validates against the
# system trust store with no bypass (docs/05 §6). Trusting one CA on the development machine
# is a smaller and more reversible change than adding a code path that skips validation — and
# a validation bypass that exists "only for tests" is exactly the kind of thing that ships.
#
# What keeps that trust small is not a short life. The first version of this script made a CA
# that lived 30 days and kept its key: a key on disk that the development machine would accept
# for *any* site, and a rig that stopped working every month. So now:
#
# - The CA signs one certificate and its key is deleted straight after. Nothing else can ever
#   be issued under it, so trusting it vouches for this server and nothing more.
# - It carries critical name constraints listing only the names and addresses below, so even a
#   copy of the key taken before the deletion could not issue a certificate for anything else.
#   SChannel enforces them; test/dovecot/README.md says how that was checked.
# - The server certificate lasts 397 days, the longest a server certificate is generally
#   accepted for, and the CA a day longer.
#
# Renewing is running this again with the same arguments, then trusting the new CA and
# removing the old one. README.md has the commands. The running server keeps the certificate
# it loaded until it is reloaded, so nothing changes for it until then.
set -eu

DIR="$(cd "$(dirname "$0")" && pwd)/certs"
HOST="${1:-halcyon-test.local}"
# Remaining arguments are extra names or addresses for the SAN list below.
[ $# -gt 0 ] && shift
DAYS=397

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

run() {
  if ! "$@" >>"$WORK/log" 2>&1; then
    cat "$WORK/log" >&2
    echo "certs.sh: '$1 $2' failed; nothing in $DIR was changed." >&2
    exit 1
  fi
}

# The SAN is what actually gets validated; a CN alone has not been accepted for years.
#
# The IP matters as much as the name here. The development machine cannot resolve the Mac's
# mDNS name, so Halcyon connects to it by address — and a certificate with only a DNS SAN
# fails validation against an IP, which would look like a broken server rather than a missing
# SAN. Every address the client might use has to be listed, and the CA's constraints have to
# permit each one.
SANS="DNS:$HOST, DNS:localhost, IP:127.0.0.1"
PERMITTED="permitted;DNS:$HOST, permitted;DNS:localhost, permitted;IP:127.0.0.1/255.255.255.255"
for extra in "$@"; do
  case "$extra" in
    # Crude but sufficient: anything that is only digits and dots is an address.
    *[!0-9.]*)
      SANS="$SANS, DNS:$extra"
      PERMITTED="$PERMITTED, permitted;DNS:$extra"
      ;;
    *)
      SANS="$SANS, IP:$extra"
      PERMITTED="$PERMITTED, permitted;IP:$extra/255.255.255.255"
      ;;
  esac
done

# The subjects are in the configuration rather than given with `-subj`, because Git Bash
# rewrites an argument starting with a slash into a Windows path. The date is in the CA's name
# so the old and new ones can be told apart in the certificate store during a renewal.
cat >"$WORK/ca.cnf" <<EOF
[req]
distinguished_name = subject
prompt = no

[subject]
CN = Halcyon Test CA $(date -u +%Y-%m-%d)

[extensions]
basicConstraints = critical, CA:TRUE, pathlen:0
keyUsage = critical, keyCertSign, cRLSign
subjectKeyIdentifier = hash
nameConstraints = critical, $PERMITTED
EOF

cat >"$WORK/server.cnf" <<EOF
[req]
distinguished_name = subject
prompt = no

[subject]
CN = $HOST

[extensions]
basicConstraints = critical, CA:FALSE
keyUsage = critical, digitalSignature, keyEncipherment
extendedKeyUsage = serverAuth
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid
subjectAltName = $SANS
EOF

run openssl req -x509 -config "$WORK/ca.cnf" -extensions extensions \
  -newkey rsa:2048 -nodes -sha256 -days $((DAYS + 1)) \
  -keyout "$WORK/ca.key" -out "$WORK/ca.crt"

run openssl req -new -config "$WORK/server.cnf" \
  -newkey rsa:2048 -nodes -sha256 \
  -keyout "$WORK/server.key" -out "$WORK/server.csr"

# A random serial rather than `-CAcreateserial`: the CA signs once, so a serial file would be
# one more thing left behind.
run openssl x509 -req -in "$WORK/server.csr" \
  -CA "$WORK/ca.crt" -CAkey "$WORK/ca.key" -set_serial "0x$(openssl rand -hex 16)" \
  -sha256 -days "$DAYS" -extfile "$WORK/server.cnf" -extensions extensions \
  -out "$WORK/server.crt"

# Checked here, where a mistake costs nothing, rather than by a failed handshake later.
run openssl verify -CAfile "$WORK/ca.crt" "$WORK/server.crt"

rm -f "$WORK/ca.key"

chmod 644 "$WORK/server.crt" "$WORK/ca.crt"
chmod 600 "$WORK/server.key"
mkdir -p "$DIR"
for file in ca.crt server.crt server.key; do
  mv -f "$WORK/$file" "$DIR/$file"
done
# What an earlier run of this script left: the old CA's key and its serial file.
rm -f "$DIR/ca.key" "$DIR/ca.srl"

echo "CA:     $DIR/ca.crt — trust this on the machine running Halcyon"
openssl x509 -in "$DIR/ca.crt" -noout -subject -enddate -fingerprint -sha1 | sed 's/^/        /'
echo "server: $DIR/server.crt for $SANS"
openssl x509 -in "$DIR/server.crt" -noout -enddate | sed 's/^/        /'
echo
echo "Then reload the server and replace the trusted CA: README.md, \"The certificate\"."
