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
# Pin the locale and timezone rather than inheriting the developer's. Reference
# `node` is not invariant under either — `(1234.5).toLocaleString()` is
# `1.234,5` under de_DE and `new Date(0).getHours()` is 9 under Asia/Tokyo — so
# a corpus case touching them would pass on one machine and fail on another with
# nothing in the report to say why. node-js reads no LANG/LC_ALL/TZ anywhere, so
# the pin costs it nothing. UTC specifically because this runtime hardwires
# `Date` to UTC: pinning any other zone would turn every local-time getter into
# a standing divergence the harness cannot act on (a documented missing feature,
# not something to rediscover per run).
export TZ=UTC LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8
VERBOSE="${1:-}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

command -v "$ORACLE" >/dev/null || { echo "parity: no reference '$ORACLE' on PATH"; exit 2; }
[ -x "$NODEJS" ] || { echo "parity: $NODEJS not built (cargo build)"; exit 2; }
# Compiler/parser changes are not invalidated by the source-keyed rkyv cache.
rm -f "$HOME/.node-js/scripts.rkyv"

pass=0; fail=0; skipped=0; barren=0; generated=0
declare -a misses skips barrens
: >"$TMP/generated.txt"
: >"$TMP/classified.txt"
while IFS= read -r f; do
  rel="${f#"$CORPUS"/}"
  generated=$((generated+1)); printf '%s\n' "$rel" >>"$TMP/generated.txt"
  timeout 15 "$ORACLE" "$f" >"$TMP/n.out" 2>/dev/null; nrc=$?
  timeout 15 "$NODEJS" "$f" >"$TMP/j.out" 2>/dev/null; jrc=$?
  # A side that timed out never reached a verdict, so this case was NOT compared.
  # `timeout` reports 124 when it kills the child.
  if [ $nrc -eq 124 ] || [ $jrc -eq 124 ]; then
    skipped=$((skipped+1)); skips+=("$rel|$nrc|$jrc"); printf '%s\n' "$rel" >>"$TMP/classified.txt"
    continue
  fi
  nbytes=$(wc -c <"$TMP/n.out" | tr -d ' ')
  jbytes=$(wc -c <"$TMP/j.out" | tr -d ' ')
  # Both sides silent AND both failing. Two empty files compare equal and two
  # non-zero exits "agree", so this scored as a PASS while comparing nothing at
  # all — a `node-js` that died instantly on every input would have banked the
  # whole corpus as green. Nothing observable was checked, so it is not a pass.
  if [ "$nbytes" -eq 0 ] && [ "$jbytes" -eq 0 ] && [ $nrc -ne 0 ] && [ $jrc -ne 0 ]; then
    barren=$((barren+1)); barrens+=("$rel|$nrc|$jrc"); printf '%s\n' "$rel" >>"$TMP/classified.txt"
    continue
  fi
  # Exit status compared EXACTLY, not collapsed to zero-vs-nonzero.
  #
  # The looser test treated 1 and 7 as agreement, on the reasoning that an
  # interpreter may pick its own code for a failure. node-js reproduces Node's
  # codes exactly, and the loose form hid a whole observable: `process.exitCode
  # = 3` prints nothing, so with stdout empty on both sides and the status
  # collapsed to a boolean there was nothing left to compare — which is how
  # `process.exitCode` came to be stored and then ignored at exit, with this
  # corpus green throughout.
  ok_rc=0; [ "$nrc" -eq "$jrc" ] || ok_rc=1
  printf '%s\n' "$rel" >>"$TMP/classified.txt"
  if cmp -s "$TMP/n.out" "$TMP/j.out" && [ $ok_rc -eq 0 ]; then
    pass=$((pass+1))
  else
    fail=$((fail+1)); misses+=("$rel|$nrc|$jrc")
    if [ "$VERBOSE" = "-v" ]; then
      echo "=== DIFF $rel  (node rc=$nrc, node-js rc=$jrc) ==="
      diff <(cat "$TMP/n.out") <(cat "$TMP/j.out") | head -20
      [ "$nrc" -ne "$jrc" ] && echo "  exit status: node=$nrc node-js=$jrc"
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

# ── the gate ────────────────────────────────────────────────────────────────
# A green result has to mean "every generated case was really compared and
# agreed", not merely "nothing recorded a divergence". Those come apart when a
# run compares nothing: an empty corpus left pass=fail=0 and the script exited
# 0, and per-case, a silent mutual failure was counted as a match. Buckets that
# are only counted change nothing, so each one below is consumed here.
#
# Every generated case lands in exactly one of matched / diverged / skipped /
# barren, so the four must add back up to the generated count; a shortfall means
# a case fell out of the loop unclassified and is named rather than ignored.
compared=$((pass+fail))
unaccounted=$(comm -23 "$TMP/generated.txt" <(sort "$TMP/classified.txt") | sort)
n_unaccounted=$(printf '%s' "$unaccounted" | grep -c . || true)
echo "accounting: generated=$generated compared=$compared (matched=$pass diverged=$fail) skipped=$skipped barren=$barren unaccounted=$n_unaccounted"

gate=0
if [ "$generated" -eq 0 ]; then
  echo "GATE FAIL: no cases were generated — the corpus is empty or unreadable ($CORPUS)."
  gate=1
fi
if [ "$generated" -gt 0 ] && [ "$compared" -eq 0 ]; then
  echo "GATE FAIL: $generated case(s) generated but NONE was compared — this run verified nothing."
  gate=1
fi
if [ "$skipped" -gt 0 ]; then
  echo "GATE FAIL: $skipped case(s) never produced a verdict (timed out):"
  for m in "${skips[@]}"; do
    IFS='|' read -r rel nrc jrc <<<"$m"
    echo "  SKIP  $rel  (node rc=$nrc, node-js rc=$jrc)"
  done
  gate=1
fi
if [ "$barren" -gt 0 ]; then
  echo "GATE FAIL: $barren case(s) compared nothing (no output from either side, both failed):"
  for m in "${barrens[@]}"; do
    IFS='|' read -r rel nrc jrc <<<"$m"
    echo "  BARREN  $rel  (node rc=$nrc, node-js rc=$jrc)"
  done
  gate=1
fi
if [ "$n_unaccounted" -gt 0 ]; then
  echo "GATE FAIL: $n_unaccounted generated case(s) reached no bucket:"
  printf '  UNACCOUNTED  %s\n' $unaccounted
  gate=1
fi

[ $fail -eq 0 ] && [ $gate -eq 0 ]
