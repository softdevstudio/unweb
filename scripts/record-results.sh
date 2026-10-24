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
  # GitHub cuts long annotations short and keeps ten of each level per
  # step, so the lines go out five at a time, each line cut at 700
  # characters (the full results are in the uploaded files).
  local part=0 chunk
  local lines
  lines=$(tail -n 50 "$3" | cut -c1-700 | sed -e 's/%/%25/g')
  while [ -n "$lines" ] && [ "$part" -lt 10 ]; do
    part=$((part + 1))
    chunk=$(printf '%s\n' "$lines" | head -n 5 | awk 'BEGIN{ORS="%0A"} {print}')
    lines=$(printf '%s\n' "$lines" | tail -n +6)
    echo "::$1 title=$2 ($os) $part::$chunk"
  done
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
grep -E '^(PASS|FAIL|SKIP|selftest)' "$raw" | tee -a "$log" || true
results=$(grep -cE '^(PASS|FAIL|SKIP|selftest)' "$raw" || true)

# After Unweb has exited, none of the folders it or its engine could use may
# be left, not even empty.
sleep 5
line=$(bash scripts/check-left-after-exit.sh) || { [ "$status" -eq 0 ] && status=1; }
echo "$line" | tee -a "$log" >> "$raw"
echo "selftest exit code: $status" | tee -a "$log"
grep -E '^(PASS|FAIL|SKIP|selftest)' "$raw" > "$out/selftest.txt" || true
if [ "$status" -eq 0 ]; then
  annotate notice "selftest passed" "$out/selftest.txt"
else
  if [ "${results:-0}" -eq 0 ]; then
    # The self-test printed no result (a crash): show how it ended.
    { echo "no self-test result; exit code $status; raw output:"; tail -n 30 "$raw"; } >> "$out/selftest.txt"
  fi
  annotate error "selftest failed" "$out/selftest.txt"
fi
exit "$status"
