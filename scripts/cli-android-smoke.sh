#!/bin/sh
# Run on an attached Android device/emulator; ANDROID_SERIAL selects the device.
set -eu
root=$(CDPATH= cd "$(dirname "$0")/.." && pwd)
adb push "${1:?Pass the Android CLI executable}" /data/local/tmp/forwiroh-cli
adb shell chmod 700 /data/local/tmp/forwiroh-cli
adb shell sh < "$root/scripts/cli-android-device.sh"
