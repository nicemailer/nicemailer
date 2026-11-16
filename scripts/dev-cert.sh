#!/bin/bash
# Creates, once, the self-signed code signing certificate "NiceMailer Dev",
# the fallback of `make core` (scripts/sign-core.sh) on a Mac without an
# Apple Development certificate.
#
# Limit: in the login keychain, an item saved by a program signed without
# an Apple Team ID gets the partition "cdhash:<hash of that binary>", so
# the next build is refused even with this certificate. Only throwaway
# keychains (security create-keychain) have no partition. Prefer an Apple
# Development certificate, see docs/configuration.md, "Passwords".
#
# Why: the core keeps the passwords of the accounts in the Keychain, which
# lets an item be read without a dialog only by the application that saved
# it, recognized by its code signature. A Rust binary is signed by the
# linker with a signature of its own content, so every rebuild would be a
# new application, refused (the core never shows Keychain dialogs). Signed
# with this certificate and the identifier eu.nicemailer.core, every build
# is the same application. See docs/configuration.md, "Passwords".
#
# The certificate is self-signed and not marked as trusted (that would need
# an administrator password): codesign uses it anyway, and the Keychain
# only compares it with itself. It goes into the login keychain, or into
# $KEYCHAIN when set (the tests of this script use a throwaway one).
#
# Usage: scripts/dev-cert.sh        (does nothing if the certificate exists)
set -euo pipefail

NAME="NiceMailer Dev"
KEYCHAIN=${KEYCHAIN:-$HOME/Library/Keychains/login.keychain-db}

if security find-identity -p codesigning "$KEYCHAIN" 2>/dev/null | grep -q "\"$NAME\""; then
  echo "dev-cert: \"$NAME\" already in $KEYCHAIN"
  exit 0
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
cat > "$TMP/cert.cnf" <<CNF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $NAME
[ext]
basicConstraints = critical,CA:false
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
CNF
openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -config "$TMP/cert.cnf" \
  -keyout "$TMP/key.pem" -out "$TMP/cert.pem" 2>/dev/null
# The Keychain reads only the older PKCS#12 encryption: OpenSSL 3 needs
# -legacy for it, LibreSSL (macOS's openssl) does not know the option and
# uses it by default.
PASS=$(openssl rand -hex 16)
openssl pkcs12 -export -legacy -inkey "$TMP/key.pem" -in "$TMP/cert.pem" -out "$TMP/id.p12" -passout "pass:$PASS" 2>/dev/null \
  || openssl pkcs12 -export -inkey "$TMP/key.pem" -in "$TMP/cert.pem" -out "$TMP/id.p12" -passout "pass:$PASS"
# -T: codesign may use the private key without asking.
security import "$TMP/id.p12" -k "$KEYCHAIN" -P "$PASS" -T /usr/bin/codesign >/dev/null
echo "dev-cert: \"$NAME\" created in $KEYCHAIN"
