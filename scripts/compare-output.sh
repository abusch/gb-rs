#!/usr/bin/env bash
# Check that the working copy emulates exactly like another revision: run the `headless` example of
# both on every ROM, in parallel, and compare the hashes of their video and audio output.
#
# Usage: scripts/compare-output.sh [REV] [FRAMES]
#
# REV is a jj revision (default: @-). The ROMs are the DMG test suites in test_roms/, the games in
# roms/, and, if $GBRS_ROMSET names a directory of zipped games, those too. They're unzipped once
# into target/compare/romset/, which later runs keep using: delete it to pick up changes or to
# leave the games out.
set -euo pipefail

rev=${1:-@-}
frames=${2:-1200}
root=$(cd "$(dirname "$0")/.." && pwd)
work="$root/target/compare"
mkdir -p "$work"

commit=$(jj log --no-graph -r "$rev" -T commit_id --repository "$root")
echo "Comparing the working copy with $rev (${commit:0:12}), $frames frames per ROM"

# Build the old revision in a temporary workspace, with a target directory kept between runs.
src="$work/src-$$"
jj workspace add --quiet --repository "$root" --name "compare-$$" -r "$commit" "$src"
cleanup() {
    jj workspace forget --quiet --repository "$root" "compare-$$" || true
    rm -rf "$src"
}
trap cleanup EXIT
CARGO_TARGET_DIR="$work/target" cargo build -q --release --example headless -p gbrs \
    --manifest-path "$src/Cargo.toml"
cp "$work/target/release/examples/headless" "$work/headless-old"

cargo build -q --release --example headless -p gbrs --manifest-path "$root/Cargo.toml"
cp "$root/target/release/examples/headless" "$work/headless-new"

if [[ -n ${GBRS_ROMSET:-} && ! -d "$work/romset" ]]; then
    echo "Unzipping $GBRS_ROMSET"
    mkdir -p "$work/romset"
    for zip in "$GBRS_ROMSET"/*.zip; do
        unzip -oqj "$zip" '*.gb' -d "$work/romset" 2>/dev/null || true
    done
fi

dirs=()
for dir in test_roms/blargg test_roms/mooneye-test-suite test_roms/dmg-acid2 \
    test_roms/mealybug-tearoom-tests test_roms/age-test-roms test_roms/rtc3test roms; do
    [[ -d "$root/$dir" ]] && dirs+=("$root/$dir")
done
[[ -d "$work/romset" ]] && dirs+=("$work/romset")

export OLD="$work/headless-old" NEW="$work/headless-new" FRAMES="$frames"
results=$(find "${dirs[@]}" -name '*.gb' -print0 |
    xargs -0 -P "$(getconf _NPROCESSORS_ONLN)" -n 1 sh -c '
        old=$("$OLD" "$1" "$FRAMES" 2>&1 | grep -E "^(video|audio)")
        new=$("$NEW" "$1" "$FRAMES" 2>&1 | grep -E "^(video|audio)")
        if [ "$old" = "$new" ]; then echo same; else echo "differs: $1"; fi' _)

grep -v '^same$' <<<"$results" | sort || true
total=$(wc -l <<<"$results" | tr -d ' ')
differ=$(grep -vc '^same$' <<<"$results" || true)
echo "$differ of $total ROMs differ"
[[ $differ -eq 0 ]]
