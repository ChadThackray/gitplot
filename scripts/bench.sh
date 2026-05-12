#!/usr/bin/env bash
# Repeatable wall-time bench for git_walk.
# Usage: scripts/bench.sh <repo-path> [runs] [workers]
set -euo pipefail

repo="${1:?usage: bench.sh <repo> [runs] [workers]}"
runs="${2:-3}"
workers="${3:-}"

# Ensure release build is current.
cargo build --release --example walk_summary >/dev/null

# Warmup (drop first result).
./target/release/examples/walk_summary "$repo" >/dev/null

echo "repo=$repo runs=$runs workers=${workers:-default}"
times=()
loc=""
for i in $(seq 1 "$runs"); do
    out=$(mktemp)
    err=$(mktemp)
    if [[ -n "$workers" ]]; then
        GITPLOT_WORKERS="$workers" \
            /usr/bin/time -f "%e" ./target/release/examples/walk_summary "$repo" >"$out" 2>"$err"
    else
        /usr/bin/time -f "%e" ./target/release/examples/walk_summary "$repo" >"$out" 2>"$err"
    fi
    t=$(tail -1 "$err")
    times+=("$t")
    # Capture LOC from last line "span: ... (... → N LOC)"
    loc=$(grep -oE '[0-9]+ LOC\)' "$out" | tail -1 | grep -oE '[0-9]+' || echo "?")
    printf "  run %d: %ss (latest LOC=%s)\n" "$i" "$t" "$loc"
    rm -f "$out" "$err"
done

# Median
sorted=$(printf "%s\n" "${times[@]}" | sort -n)
mid=$(( ${#times[@]} / 2 ))
median=$(echo "$sorted" | sed -n "$((mid + 1))p")
echo "median=${median}s loc=$loc"
