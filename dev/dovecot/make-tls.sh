#!/bin/sh
# Creates a throwaway certificate authority and a server certificate for the
# test Dovecot servers (compose.yaml), valid for localhost and 127.0.0.1.
# The tests trust `tls/ca.crt` and nothing else, so they check certificates
# exactly as with a real server. Run by `make imap-up`. The directory is
# ignored by Git.
#
# The core checks certificates with macOS's own rules (rustls-platform-
# verifier), which refuse a server certificate valid for more than 825 days
# or without the "server authentication" usage, even from an authority the
# user trusts (error -67901, "not standards compliant"). The certificate
# follows them, and is made again when it does not, or expires within 30
# days.
set -eu
cd "$(dirname "$0")"
if [ -f tls/server.key ] && [ -f tls/server.crt ] \
  && openssl x509 -in tls/server.crt -noout -text 2>/dev/null | grep -q "TLS Web Server Authentication" \
  && openssl x509 -in tls/server.crt -noout -checkend 2592000 >/dev/null 2>&1; then
  exit 0
fi
echo "make-tls.sh: new test certificates in dev/dovecot/tls"
rm -rf tls
mkdir -p tls
openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -subj "/CN=NiceMailer test CA" \
  -keyout tls/ca.key -out tls/ca.crt \
  -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign" 2>/dev/null \
  || openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -subj "/CN=NiceMailer test CA" \
       -keyout tls/ca.key -out tls/ca.crt 2>/dev/null
openssl req -newkey rsa:2048 -nodes -subj "/CN=localhost" \
  -keyout tls/server.key -out tls/server.csr 2>/dev/null
printf 'subjectAltName=DNS:localhost,IP:127.0.0.1\nbasicConstraints=CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n' > tls/ext.cnf
openssl x509 -req -in tls/server.csr -CA tls/ca.crt -CAkey tls/ca.key -CAcreateserial \
  -days 825 -extfile tls/ext.cnf -out tls/server.crt 2>/dev/null
# Dovecot reads the key as the vmail user of the image.
chmod 644 tls/server.key
rm -f tls/server.csr tls/ext.cnf tls/ca.srl
