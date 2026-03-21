#!/bin/zsh

set -euo pipefail

IDENTITY_NAME="${ETSU_CODESIGN_IDENTITY:-ETSU Development}"
KEYCHAIN_PATH="${ETSU_CODESIGN_KEYCHAIN:-$HOME/Library/Keychains/login.keychain-db}"
OPENSSL_BIN="${OPENSSL_BIN:-$(command -v openssl || true)}"
TMP_DIR="$(mktemp -d)"
CERT_PEM="$TMP_DIR/etsu-dev-cert.pem"
KEY_PEM="$TMP_DIR/etsu-dev-key.pem"
P12_PATH="$TMP_DIR/etsu-dev-codesign.p12"
P12_PASS="$(LC_ALL=C tr -dc 'A-Za-z0-9' </dev/urandom | head -c 32 || true)"

cleanup() {
  rm -rf "$TMP_DIR"
}

trap cleanup EXIT

note() {
  printf '%s\n' "$*"
}

warn() {
  printf '%s\n' "$*" >&2
}

cert_and_key_exist() {
  security find-certificate -c "$IDENTITY_NAME" "$KEYCHAIN_PATH" >/dev/null 2>&1 \
    && security find-key -l "$IDENTITY_NAME" "$KEYCHAIN_PATH" >/dev/null 2>&1
}

trust_existing_identity() {
  security find-certificate -c "$IDENTITY_NAME" -p "$KEYCHAIN_PATH" > "$CERT_PEM"
  security add-trusted-cert \
    -p codeSign \
    -r trustRoot \
    -k "$KEYCHAIN_PATH" \
    "$CERT_PEM"
}

if [[ -z "$OPENSSL_BIN" ]]; then
  warn "openssl is required but was not found."
  exit 1
fi

if security find-identity -v -p codesigning 2>/dev/null | grep -Fq "\"$IDENTITY_NAME\""; then
  note "Codesigning identity already exists: $IDENTITY_NAME"
  exit 0
fi

if cert_and_key_exist; then
  note "Existing certificate/key found for $IDENTITY_NAME; updating trust settings..."
  trust_existing_identity

  if security find-identity -v -p codesigning "$KEYCHAIN_PATH" | grep -Fq "\"$IDENTITY_NAME\""; then
    note "Created codesigning identity: $IDENTITY_NAME"
    security find-identity -v -p codesigning "$KEYCHAIN_PATH" | grep -F "\"$IDENTITY_NAME\""
    exit 0
  fi
fi

note "Creating self-signed codesigning identity: $IDENTITY_NAME"

"$OPENSSL_BIN" req -newkey rsa:2048 -nodes \
  -keyout "$KEY_PEM" \
  -x509 \
  -sha256 \
  -days 3650 \
  -out "$CERT_PEM" \
  -subj "/CN=$IDENTITY_NAME/OU=ETSU/O=ETSU/C=US" \
  -addext "basicConstraints=critical,CA:FALSE" \
  -addext "keyUsage=critical,digitalSignature" \
  -addext "extendedKeyUsage=codeSigning" \
  -addext "subjectKeyIdentifier=hash"

"$OPENSSL_BIN" pkcs12 -export \
  -inkey "$KEY_PEM" \
  -in "$CERT_PEM" \
  -name "$IDENTITY_NAME" \
  -out "$P12_PATH" \
  -passout "pass:$P12_PASS" \
  -legacy

security import "$P12_PATH" \
  -k "$KEYCHAIN_PATH" \
  -P "$P12_PASS" \
  -T /usr/bin/codesign \
  -T /usr/bin/security

trust_existing_identity

if [[ -n "${ETSU_KEYCHAIN_PASSWORD:-}" ]]; then
  security set-key-partition-list \
    -S apple-tool:,apple:,codesign: \
    -s \
    -k "$ETSU_KEYCHAIN_PASSWORD" \
    "$KEYCHAIN_PATH"
else
  warn "ETSU_KEYCHAIN_PASSWORD was not set."
  warn "codesign may prompt the first time it accesses the imported private key."
fi

note "Created codesigning identity: $IDENTITY_NAME"
security find-identity -v -p codesigning "$KEYCHAIN_PATH" | grep -F "\"$IDENTITY_NAME\""
