#!/bin/sh
# Creates a throwaway certificate authority and a server certificate for the
# test Dovecot servers (compose.yaml), valid for localhost and 127.0.0.1.
# The tests trust `tls/ca.crt` and nothing else, so they check certificates
# exactly as with a real server. Run by `make imap-up`; does nothing if the
# files already exist. The directory is ignored by Git.
set -eu
cd "$(dirname "$0")"
[ -f tls/server.key ] && exit 0
mkdir -p tls
openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -subj "/CN=NiceMailer test CA" \
  -keyout tls/ca.key -out tls/ca.crt 2>/dev/null
openssl req -newkey rsa:2048 -nodes -subj "/CN=localhost" \
  -keyout tls/server.key -out tls/server.csr 2>/dev/null
printf 'subjectAltName=DNS:localhost,IP:127.0.0.1\nbasicConstraints=CA:FALSE\n' > tls/ext.cnf
openssl x509 -req -in tls/server.csr -CA tls/ca.crt -CAkey tls/ca.key -CAcreateserial \
  -days 3650 -extfile tls/ext.cnf -out tls/server.crt 2>/dev/null
# Dovecot reads the key as the vmail user of the image.
chmod 644 tls/server.key
rm -f tls/server.csr tls/ext.cnf tls/ca.srl
