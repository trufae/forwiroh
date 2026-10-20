#!/system/bin/sh
# Executed on Android by cli-android-smoke.sh.
set -eu
binary=/data/local/tmp/forwiroh-cli
state=$(mktemp -d /data/local/tmp/forwiroh-test.XXXXXX)
daemon=
cleanup() {
    result=$?
    trap - EXIT
    if [ -n "$daemon" ]; then
        kill "$daemon" 2>/dev/null || true
        wait "$daemon" 2>/dev/null || true
    fi
    if [ "$result" -ne 0 ]; then
        cat "$state/log" 2>/dev/null || true
        cat "$state/error" 2>/dev/null || true
        logcat -d -t 80 2>/dev/null || true
    fi
    rm -rf "$state"
    exit "$result"
}
trap cleanup EXIT
"$binary" --version
"$binary" --state "$state" serve --offline --name android-cli >"$state/log" 2>&1 &
daemon=$!
attempts=0
until "$binary" --state "$state" status >"$state/status" 2>"$state/error"; do
    kill -0 "$daemon" 2>/dev/null || exit 1
    attempts=$((attempts + 1))
    [ "$attempts" -lt 30 ] || exit 1
    sleep 1
done
cat "$state/status"
grep -q '"name": "android-cli"' "$state/status"
"$binary" --state "$state" pending
echo 'Standalone Android CLI version, daemon startup and local control: PASS'
