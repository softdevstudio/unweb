#!/usr/bin/env bash
# After Unweb has exited, none of the folders it or its web engine could use
# may be left, not even empty. Prints one PASS or FAIL line; exits 1 on FAIL.
#
#   scripts/check-left-after-exit.sh
set -u
case "$(uname -s)" in
  Linux*) after=("$HOME/.local/share/org.unweb.app" "$HOME/.cache/org.unweb.app" "$HOME/.local/share/unweb" "$HOME/.cache/unweb") ;;
  Darwin*) after=("$HOME/Library/Application Support/org.unweb.app" "$HOME/Library/Caches/org.unweb.app" "$HOME/Library/WebKit/org.unweb.app" "$HOME/Library/HTTPStorages/org.unweb.app" "$HOME/Library/Caches/unweb" "$HOME/Library/WebKit/unweb" "$HOME/Library/HTTPStorages/unweb") ;;
  MINGW*|MSYS*|CYGWIN*) after=("${APPDATA:-}/org.unweb.app" "${LOCALAPPDATA:-}/org.unweb.app") ;;
  *) after=() ;;
esac
left=()
for d in "${after[@]}"; do
  [ -e "$d" ] && left+=("$d")
done
if [ "${#left[@]}" -eq 0 ]; then
  echo "PASS nothing is left after Unweb exits: none of ${#after[@]} folders exists"
else
  echo "FAIL nothing is left after Unweb exits: still there: ${left[*]}"
  exit 1
fi
