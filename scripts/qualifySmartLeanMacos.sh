#!/usr/bin/env bash
set -euo pipefail
# No existing disks or user directories: create, authorize and remove one image.
task_root="${RUNNER_TEMP:?}/zen-smart-lean-${GITHUB_RUN_ID:-local}-${GITHUB_RUN_ATTEMPT:-1}"
volume_name="ZenSmartLean-${GITHUB_RUN_ID:-local}-${GITHUB_RUN_ATTEMPT:-1}"
mount_root="/Volumes/$volume_name"
image_path="$task_root/fixture.dmg"
mkdir -p "$task_root"
if test -e "$mount_root"; then echo "Refusing existing mount: $mount_root" >&2; exit 1; fi
printf '%s' "$mount_root" > "$task_root/owned-volume.txt"
cleanup() {
  if test -f "$image_path" && test -f "$task_root/owned-volume.txt"; then
    hdiutil detach "$mount_root" || true
  fi
}
trap cleanup EXIT
hdiutil create -size 128m -fs APFS -volname "$volume_name" "$image_path"
hdiutil attach -nobrowse "$image_path"
test -d "$mount_root"
# Hosted Spotlight availability is reported by the test, never invented as PASS.
mdutil -i on "$mount_root" > "$task_root/mdutil.log" 2>&1 || true
export ZEN_SMART_LEAN_TASK_ROOT="$task_root"
export ZEN_SMART_LEAN_VOLUME_ROOT="$mount_root"
export ZEN_SMART_LEAN_NATIVE_REPORT="$PWD/smart-lean-native-macos.json"
# Compile before starting the bounded native run. On a stalled native call,
# sample only this task's test child, then fail rather than hanging the runner.
cargo build --locked --manifest-path src-tauri/Cargo.toml --features "desktop-runtime native-qa" --bin smart-lean-macos-qa
cargo run --locked --manifest-path src-tauri/Cargo.toml --features "desktop-runtime native-qa" --bin smart-lean-macos-qa &
test_owner=$!
(
  sleep 120
  for child in $(pgrep -P "$test_owner" || true); do
    sample "$child" 3 -file "$PWD/smart-lean-native-macos-sample.txt" || true
    kill -TERM "$child" || true
  done
) &
watchdog=$!
result=0
wait "$test_owner" || result=$?
kill "$watchdog" 2>/dev/null || true
wait "$watchdog" 2>/dev/null || true
exit "$result"
