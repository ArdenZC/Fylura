#!/usr/bin/env bash
set -euo pipefail
# No existing disks or user directories: create, authorize and remove one image.
task_root="${RUNNER_TEMP:?}/zen-smart-lean-${GITHUB_RUN_ID:-local}-${GITHUB_RUN_ATTEMPT:-1}"
mount_root="$task_root/mount"
image_path="$task_root/fixture.dmg"
mkdir -p "$mount_root"
printf '%s' "$mount_root" > "$task_root/owned-volume.txt"
cleanup() {
  if test -f "$image_path" && test -f "$task_root/owned-volume.txt"; then
    hdiutil detach "$mount_root" || true
  fi
}
trap cleanup EXIT
hdiutil create -size 128m -fs APFS -volname ZenSmartLeanQA "$image_path"
hdiutil attach -nobrowse -mountpoint "$mount_root" "$image_path"
# Hosted Spotlight availability is reported by the test, never invented as PASS.
mdutil -i on "$mount_root" > "$task_root/mdutil.log" 2>&1 || true
export ZEN_SMART_LEAN_VOLUME_ROOT="$mount_root"
export ZEN_SMART_LEAN_NATIVE_REPORT="$PWD/smart-lean-native-macos.json"
cargo test --locked --manifest-path src-tauri/Cargo.toml --features desktop-runtime --lib scoped_spotlight_fsevents_qualification -- --ignored --nocapture --test-threads=1
