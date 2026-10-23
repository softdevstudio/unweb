#!/usr/bin/env bash
# Runs every check and records the results under test/results/latest/<os>/.
#
#   rustfmt, clippy with warnings denied, all tests, then the app's
#   end-to-end self-test on this OS's web engine.
#
# git history keeps earlier runs. Every number on the site comes from
# these files.
set -euo pipefail
cd "$(dirname "$0")/.."

case "$(uname -s)" in
  Linux*) os=linux ;;
  Darwin*) os=macos ;;
  MINGW*|MSYS*|CYGWIN*) os=windows ;;
  *) os=other ;;
esac
out="test/results/latest/$os"
rm -rf "$out"
mkdir -p "$out"
log="$out/checks.log"

{
  echo "date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "commit: $(git rev-parse --short HEAD 2>/dev/null || echo none)"
  echo "os: $os ($(uname -srm))"
  rustc --version
  cargo --version
  echo
} > "$log"

# In GitHub Actions, results are also posted as annotations, which can be
# read through the API without downloading logs or artifacts.
annotate() { # level title file
  [ -n "${GITHUB_ACTIONS:-}" ] || return 0
  local body
  body=$(sed -e 's/%/%25/g' "$3" | tail -n 40 | awk 'BEGIN{ORS="%0A"} {print}')
  echo "::$1 title=$2 ($os)::$body"
}

step() {
  echo "== $*" | tee -a "$log"
  local tmp
  tmp=$(mktemp)
  set +e
  "$@" > "$tmp" 2>&1
  local rc=$?
  set -e
  tee -a "$log" < "$tmp"
  if [ "$rc" -ne 0 ]; then
    annotate error "$1 $2 failed" "$tmp"
    exit "$rc"
  fi
}

step cargo fmt --all -- --check
step cargo clippy --locked --all-targets -- -D warnings
step cargo test --locked
step cargo build --locked -p unweb

bin=target/debug/unweb
[ "$os" = windows ] && bin=target/debug/unweb.exe

echo "== selftest" | tee -a "$log"
raw="$out/selftest-output.log"
set +e
if [ "$os" = linux ] && [ -z "${DISPLAY:-}" ]; then
  timeout 300 xvfb-run -a -s "-screen 0 1600x1000x24" "$bin" --selftest "$out" > "$raw" 2>&1
  status=$?
else
  "$bin" --selftest "$out" > "$raw" 2>&1 &
  pid=$!
  # The self-test ends itself within about two minutes; give it five.
  ( sleep 300 && kill "$pid" 2>/dev/null ) &
  watchdog=$!
  wait "$pid"
  status=$?
  kill "$watchdog" 2>/dev/null
fi
set -e
grep -E '^(PASS|FAIL|selftest)' "$raw" | tee -a "$log" || true
echo "selftest exit code: $status" | tee -a "$log"
grep -E '^(PASS|FAIL|selftest)' "$raw" > "$out/selftest.txt" || true
if [ "$status" -eq 0 ]; then
  annotate notice "selftest passed" "$out/selftest.txt"
else
  if [ ! -s "$out/selftest.txt" ]; then
    { echo "no self-test result; exit code $status; raw output:"; tail -n 30 "$raw"; } > "$out/selftest.txt"
  fi
  annotate error "selftest failed" "$out/selftest.txt"
fi
exit "$status"
