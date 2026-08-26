#!/bin/sh
# Generates a throwaway CA and a server certificate for the test Dovecot.
#
# A CA rather than a bare self-signed certificate, because Halcyon validates against the
# system trust store with no bypass (docs/05 §6). Trusting one short-lived CA on the
# development machine is a smaller and more reversible change than adding a code path that
# skips validation — and a validation bypass that exists "only for tests" is exactly the kind
# of thing that ships.
set -eu

DIR="$(cd "$(dirname "$0")" && pwd)/certs"
HOST="${1:-halcyon-test.local}"
mkdir -p "$DIR"

openssl req -x509 -newkey rsa:2048 -nodes -days 30 \
  -keyout "$DIR/ca.key" -out "$DIR/ca.crt" \
  -subj "/CN=Halcyon Test CA" >/dev/null 2>&1

openssl req -newkey rsa:2048 -nodes \
  -keyout "$DIR/server.key" -out "$DIR/server.csr" \
  -subj "/CN=$HOST" >/dev/null 2>&1

# The SAN is what actually gets validated; a CN alone has not been accepted for years.
cat > "$DIR/ext.cnf" <<EOF
subjectAltName = DNS:$HOST, DNS:localhost, IP:127.0.0.1
extendedKeyUsage = serverAuth
EOF

openssl x509 -req -in "$DIR/server.csr" -CA "$DIR/ca.crt" -CAkey "$DIR/ca.key" \
  -CAcreateserial -out "$DIR/server.crt" -days 30 -extfile "$DIR/ext.cnf" >/dev/null 2>&1

chmod 644 "$DIR/server.crt" "$DIR/ca.crt"
chmod 600 "$DIR/server.key" "$DIR/ca.key"
rm -f "$DIR/server.csr" "$DIR/ext.cnf"

echo "CA:     $DIR/ca.crt   (trust this on the machine running Halcyon)"
echo "server: $DIR/server.crt for $HOST"
