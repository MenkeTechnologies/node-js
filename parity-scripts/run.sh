#!/bin/bash
# Differential byte-parity harness: run every parity-scripts/**/*.js through the
# reference `node` (oracle) and the freshly-built `node-js`, and assert their
# stdout is byte-identical (and success/failure agrees). Dev tool — needs `node`
# on PATH. Prints the byte-parity rate and every divergence (with a short diff).
#
#   Usage: bash parity-scripts/run.sh [-v]     (-v shows the diff for each miss)
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
NODEJS="$ROOT/target/debug/node"
CORPUS="$ROOT/parity-scripts"
ORACLE="${NODE_JS_PARITY_NODE:-node}"
VERBOSE="${1:-}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

command -v "$ORACLE" >/dev/null || { echo "parity: no reference '$ORACLE' on PATH"; exit 2; }
[ -x "$NODEJS" ] || { echo "parity: $NODEJS not built (cargo build)"; exit 2; }
# Compiler/parser changes are not invalidated by the source-keyed rkyv cache.
rm -f "$HOME/.node-js/scripts.rkyv"

pass=0; fail=0
declare -a misses
while IFS= read -r f; do
  rel="${f#"$CORPUS"/}"
  timeout 15 "$ORACLE" "$f" >"$TMP/n.out" 2>/dev/null; nrc=$?
  timeout 15 "$NODEJS" "$f" >"$TMP/j.out" 2>/dev/null; jrc=$?
  # success-agreement: both exit 0, or both non-zero
  ok_rc=0; { [ $nrc -eq 0 ] && [ $jrc -eq 0 ]; } || { [ $nrc -ne 0 ] && [ $jrc -ne 0 ]; } || ok_rc=1
  if cmp -s "$TMP/n.out" "$TMP/j.out" && [ $ok_rc -eq 0 ]; then
    pass=$((pass+1))
  else
    fail=$((fail+1)); misses+=("$rel|$nrc|$jrc")
    if [ "$VERBOSE" = "-v" ]; then
      echo "=== DIFF $rel  (node rc=$nrc, node-js rc=$jrc) ==="
      diff <(cat "$TMP/n.out") <(cat "$TMP/j.out") | head -20
    fi
  fi
done < <(find "$CORPUS" -name '*.js' | sort)

total=$((pass+fail))
echo ""
echo "════════════════════════════════════════════"
echo "BYTE PARITY: $pass / $total match  (oracle: $ORACLE $($ORACLE --version))"
echo "════════════════════════════════════════════"
if [ $fail -gt 0 ]; then
  echo "Divergences:"
  for m in "${misses[@]}"; do
    IFS='|' read -r rel nrc jrc <<<"$m"
    echo "  DIFF  $rel  (node rc=$nrc, node-js rc=$jrc)"
  done
fi
[ $fail -eq 0 ]
