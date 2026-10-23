#!/usr/bin/env bash
# Sets up what the self-test's HTTPS checks (HSTS) need, on a test machine:
#
#   1. a throwaway certificate authority and a certificate for
#      hsts.unweb.test, made with openssl in <dir>;
#   2. the authority trusted by the system, so the web engine accepts the
#      certificate (an engine ignores HSTS from a site whose certificate it
#      doesn't trust);
#   3. hsts.unweb.test pointed at 127.0.0.1 in the hosts file.
#
# Then run the self-test with UNWEB_TEST_TLS=<dir>. This changes the
# machine's trust store and hosts file, so use it on CI runners and test
# machines only. Needs sudo on Linux and macOS, an administrator shell on
# Windows (Git Bash).
#
#   scripts/test-ca.sh <dir>
set -euo pipefail
dir="${1:?usage: scripts/test-ca.sh <dir>}"
host=hsts.unweb.test
mkdir -p "$dir"
cd "$dir"

# Git Bash on Windows rewrites arguments that start with "/"; openssl's
# -subj needs them as they are.
export MSYS_NO_PATHCONV=1

openssl req -x509 -newkey rsa:2048 -nodes -days 30 \
  -keyout ca.key -out ca.pem -subj "/CN=Unweb self-test CA" \
  -addext "basicConstraints=critical,CA:TRUE" \
  -addext "keyUsage=critical,keyCertSign,cRLSign" 2>/dev/null

openssl req -newkey rsa:2048 -nodes -keyout leaf.key -out leaf.csr \
  -subj "/CN=$host" 2>/dev/null
cat > leaf.ext <<EOF
basicConstraints=CA:FALSE
keyUsage=critical,digitalSignature,keyEncipherment
extendedKeyUsage=serverAuth
subjectAltName=DNS:$host
EOF
openssl x509 -req -in leaf.csr -CA ca.pem -CAkey ca.key -CAcreateserial \
  -days 30 -out leaf.pem -extfile leaf.ext 2>/dev/null
# The server sends the leaf and the authority that signed it.
cat leaf.pem ca.pem > chain.pem

case "$(uname -s)" in
  Linux*)
    sudo cp ca.pem /usr/local/share/ca-certificates/unweb-selftest.crt
    sudo update-ca-certificates >/dev/null
    grep -q " $host\$" /etc/hosts || echo "127.0.0.1 $host" | sudo tee -a /etc/hosts >/dev/null
    ;;
  Darwin*)
    sudo security authorizationdb write com.apple.trust-settings.admin allow >/dev/null 2>&1 || true
    sudo security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain ca.pem
    grep -q " $host\$" /etc/hosts || echo "127.0.0.1 $host" | sudo tee -a /etc/hosts >/dev/null
    ;;
  MINGW*|MSYS*|CYGWIN*)
    certutil -addstore -f Root ca.pem >/dev/null
    hosts=/c/Windows/System32/drivers/etc/hosts
    grep -q " $host" "$hosts" || printf '\r\n127.0.0.1 %s\r\n' "$host" >> "$hosts"
    ;;
esac
rm -f ca.key leaf.csr leaf.ext ca.srl
echo "test certificate for $host ready in $dir"
