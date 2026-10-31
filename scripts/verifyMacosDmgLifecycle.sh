#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <fylura.dmg> <evidence.json>" >&2
  exit 2
fi

DMG_PATH="$(cd "$(dirname "$1")" && pwd -P)/$(basename "$1")"
EVIDENCE_PATH="$2"
RUNNER_TEMP="${RUNNER_TEMP:?RUNNER_TEMP must be set by the isolated macOS runner}"
RUN_ID="${GITHUB_RUN_ID:?GITHUB_RUN_ID must be set by GitHub Actions}"
TASK_ROOT="$RUNNER_TEMP/fylura-dmg-runtime-$RUN_ID"
MOUNT_POINT="$TASK_ROOT/mount"
INSTALL_ROOT="$TASK_ROOT/Applications"
INSTALLED_APP="$INSTALL_ROOT/Fylura.app"
PROFILE_ROOT="$TASK_ROOT/profile"
TEMP_ROOT="$TASK_ROOT/tmp"
BUNDLE_ID="com.startlan.fylura"
LEGACY_BUNDLE_ID="com.startlan.zencanvas"
DATABASE_PATH="$PROFILE_ROOT/Library/Application Support/$BUNDLE_ID/fylura.sqlite3"
APP_PID=""
SOURCE_APP=""
APP_BINARY=""
MOUNT_ATTACHED=0
LAUNCH_COUNT=0
DATABASE_INTEGRITY="not_checked"
APP_REMOVED=0
STATUS="FAIL"

write_evidence() {
  mkdir -p "$(dirname "$EVIDENCE_PATH")"
  STATUS="$STATUS" \
    DMG_PATH="$DMG_PATH" \
    TASK_ROOT="$TASK_ROOT" \
    SOURCE_APP="$SOURCE_APP" \
    INSTALLED_APP="$INSTALLED_APP" \
    BUNDLE_ID="$BUNDLE_ID" \
    DATABASE_PATH="$DATABASE_PATH" \
    LAUNCH_COUNT="$LAUNCH_COUNT" \
    DATABASE_INTEGRITY="$DATABASE_INTEGRITY" \
    APP_REMOVED="$APP_REMOVED" \
    python3 - "$EVIDENCE_PATH" <<'PY'
import json
import os
import sys

document = {
    "product": "Fylura",
    "status": os.environ["STATUS"],
    "dmg": os.environ["DMG_PATH"],
    "taskRoot": os.environ["TASK_ROOT"],
    "sourceApp": os.environ["SOURCE_APP"],
    "installedApp": os.environ["INSTALLED_APP"],
    "bundleIdentifier": os.environ["BUNDLE_ID"],
    "databasePath": os.environ["DATABASE_PATH"],
    "successfulLaunches": int(os.environ["LAUNCH_COUNT"]),
    "databaseIntegrity": os.environ["DATABASE_INTEGRITY"],
    "appBundleRemoved": os.environ["APP_REMOVED"] == "1",
    "userDatabasePreserved": os.environ["APP_REMOVED"] == "1",
}
with open(sys.argv[1], "w", encoding="utf-8") as evidence:
    json.dump(document, evidence, indent=2)
    evidence.write("\n")
PY
}

cleanup() {
  exit_code=$?
  trap - EXIT

  if [[ -n "$APP_PID" ]] && kill -0 "$APP_PID" 2>/dev/null; then
    kill -TERM "$APP_PID" 2>/dev/null || true
    for _ in $(seq 1 15); do
      if ! kill -0 "$APP_PID" 2>/dev/null; then
        break
      fi
      process_state="$(ps -p "$APP_PID" -o stat= 2>/dev/null | tr -d '[:space:]' || true)"
      if [[ "$process_state" == Z* ]]; then
        break
      fi
      sleep 1
    done
    wait "$APP_PID" 2>/dev/null || true
    APP_PID=""
  fi

  if [[ "$MOUNT_ATTACHED" -eq 1 ]]; then
    if hdiutil detach "$MOUNT_POINT" -quiet; then
      MOUNT_ATTACHED=0
    else
      echo "Failed to detach task-owned DMG mount at $MOUNT_POINT." >&2
      STATUS="FAIL"
      exit_code=1
    fi
  fi

  if [[ "$exit_code" -ne 0 ]]; then
    STATUS="FAIL"
  fi
  write_evidence || exit_code=1
  exit "$exit_code"
}
trap cleanup EXIT

case "$TASK_ROOT/" in
  "$RUNNER_TEMP"/*) ;;
  *) echo "Refusing task directory outside RUNNER_TEMP: $TASK_ROOT" >&2; exit 1 ;;
esac

if [[ ! -f "$DMG_PATH" ]]; then
  echo "Fylura DMG was not found: $DMG_PATH" >&2
  exit 1
fi
if [[ -e "$TASK_ROOT" ]]; then
  echo "Refusing pre-existing task-owned directory: $TASK_ROOT" >&2
  exit 1
fi

mkdir -p "$MOUNT_POINT" "$INSTALL_ROOT" "$PROFILE_ROOT" "$TEMP_ROOT"
hdiutil attach "$DMG_PATH" -readonly -nobrowse -mountpoint "$MOUNT_POINT"
MOUNT_ATTACHED=1

SOURCE_APP="$(find "$MOUNT_POINT" -maxdepth 3 -type d -name 'Fylura.app' -print -quit)"
if [[ -z "$SOURCE_APP" || ! -d "$SOURCE_APP" ]]; then
  echo "The mounted DMG does not contain Fylura.app." >&2
  exit 1
fi

INFO_PLIST="$SOURCE_APP/Contents/Info.plist"
if [[ ! -f "$INFO_PLIST" ]]; then
  echo "Fylura.app is missing Contents/Info.plist." >&2
  exit 1
fi
PLIST_BUDDY="/usr/libexec/PlistBuddy"
bundle_id="$("$PLIST_BUDDY" -c 'Print :CFBundleIdentifier' "$INFO_PLIST")"
bundle_name="$("$PLIST_BUDDY" -c 'Print :CFBundleName' "$INFO_PLIST")"
executable_name="$("$PLIST_BUDDY" -c 'Print :CFBundleExecutable' "$INFO_PLIST")"
if [[ "$bundle_id" != "$BUNDLE_ID" ]]; then
  echo "Unexpected Fylura bundle identifier: $bundle_id" >&2
  exit 1
fi
if [[ "$bundle_name" != "Fylura" ]]; then
  echo "Unexpected Fylura bundle name: $bundle_name" >&2
  exit 1
fi

ditto "$SOURCE_APP" "$INSTALLED_APP"
APP_BINARY="$INSTALLED_APP/Contents/MacOS/$executable_name"
if [[ ! -x "$APP_BINARY" ]]; then
  echo "Fylura.app executable is missing or not executable: $APP_BINARY" >&2
  exit 1
fi

check_database() {
  if [[ ! -f "$DATABASE_PATH" ]]; then
    echo "Fylura did not create its database at the isolated app-data path: $DATABASE_PATH" >&2
    return 1
  fi
  DATABASE_INTEGRITY="$(python3 - "$DATABASE_PATH" <<'PY'
import sqlite3
import sys

connection = sqlite3.connect(sys.argv[1])
try:
    result = connection.execute("PRAGMA integrity_check").fetchone()[0]
    print(result)
    if result != "ok":
        raise SystemExit(1)
finally:
    connection.close()
PY
)"
  [[ "$DATABASE_INTEGRITY" == "ok" ]]
}

launch_and_check() {
  launch_number=$((LAUNCH_COUNT + 1))
  log_path="$TASK_ROOT/fylura-launch-$launch_number.log"
  (
    export HOME="$PROFILE_ROOT"
    export CFFIXED_USER_HOME="$PROFILE_ROOT"
    export TMPDIR="$TEMP_ROOT"
    exec "$APP_BINARY"
  ) >"$log_path" 2>&1 &
  APP_PID=$!

  for _ in $(seq 1 60); do
    if [[ -f "$DATABASE_PATH" ]]; then
      check_database || { cat "$log_path" >&2; return 1; }
      LAUNCH_COUNT="$launch_number"
      return 0
    fi
    if ! kill -0 "$APP_PID" 2>/dev/null; then
      wait "$APP_PID" 2>/dev/null || true
      APP_PID=""
      cat "$log_path" >&2
      echo "Fylura exited before creating its isolated database on launch $launch_number." >&2
      return 1
    fi
    sleep 1
  done

  cat "$log_path" >&2
  echo "Timed out waiting for Fylura to launch and create its isolated database." >&2
  return 1
}

stop_app() {
  if [[ -z "$APP_PID" ]]; then
    return 0
  fi
  kill -TERM "$APP_PID"
  for _ in $(seq 1 15); do
    if ! kill -0 "$APP_PID" 2>/dev/null; then
      break
    fi
    process_state="$(ps -p "$APP_PID" -o stat= 2>/dev/null | tr -d '[:space:]' || true)"
    if [[ "$process_state" == Z* ]]; then
      break
    fi
    sleep 1
  done
  if kill -0 "$APP_PID" 2>/dev/null; then
    process_state="$(ps -p "$APP_PID" -o stat= 2>/dev/null | tr -d '[:space:]' || true)"
    if [[ "$process_state" != Z* ]]; then
      echo "Fylura process $APP_PID did not stop after SIGTERM." >&2
      return 1
    fi
  fi
  wait "$APP_PID" 2>/dev/null || true
  APP_PID=""
}

launch_and_check
stop_app
check_database
launch_and_check
stop_app
check_database

case "$INSTALLED_APP/" in
  "$TASK_ROOT"/*) ;;
  *) echo "Refusing to remove an app bundle outside the task-owned directory." >&2; exit 1 ;;
esac
/bin/rm -R "$INSTALLED_APP"
if [[ -e "$INSTALLED_APP" ]]; then
  echo "Task-owned Fylura.app remained after the test uninstall." >&2
  exit 1
fi
APP_REMOVED=1
check_database

hdiutil detach "$MOUNT_POINT" -quiet
MOUNT_ATTACHED=0
STATUS="PASS"
echo "PASS: Fylura DMG bundle identity, two launches, database integrity, and app-removal data preservation."
