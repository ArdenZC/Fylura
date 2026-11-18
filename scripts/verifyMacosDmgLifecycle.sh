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
LEGACY_DATABASE_PATH="$PROFILE_ROOT/Library/Application Support/$LEGACY_BUNDLE_ID/zen-canvas.sqlite3"
APP_SUPPORT_ROOT="$PROFILE_ROOT/Library/Application Support/$BUNDLE_ID"
LEGACY_DATA_ROOT="$PROFILE_ROOT/Library/Application Support/$LEGACY_BUNDLE_ID"
CACHE_ROOT="$PROFILE_ROOT/Library/Caches/$BUNDLE_ID"
PREFERENCES_PATH="$PROFILE_ROOT/Library/Preferences/$BUNDLE_ID.plist"
LOGS_ROOT="$PROFILE_ROOT/Library/Logs/$BUNDLE_ID"
LAUNCH_AGENT_PATH="$PROFILE_ROOT/Library/LaunchAgents/Fylura.plist"
DOCUMENTS_SAFE_TRASH="$PROFILE_ROOT/Documents/.fylura-trash/operations/task-owned-fixture"
DOCUMENTS_LEGACY_TRASH="$PROFILE_ROOT/Documents/.zen-canvas-trash/operations/task-owned-fixture"
FINDER_TRASH="$PROFILE_ROOT/.Trash"
APP_PID=""
SOURCE_APP=""
APP_BINARY=""
MOUNT_ATTACHED=0
LAUNCH_COUNT=0
DATABASE_INTEGRITY="not_checked"
LEGACY_DATABASE_ABSENT=1
MULTIPLE_PROFILE_CREDENTIALS_REMOVED=0
SAFE_TRASH_PRESERVED=0
APP_DATA_PRESERVED_ON_ORDINARY_REMOVAL=0
APP_REMOVED=0
USER_DATABASE_PRESERVED=0
DATA_CLEANUP="not_checked"
STATUS="FAIL"
ORIGINAL_DEFAULT_KEYCHAIN=""
ORIGINAL_KEYCHAINS=()
ORIGINAL_KEYCHAIN_COUNT=0
TEST_KEYCHAIN=""
TASK_CREDENTIALS=()
TASK_CREDENTIAL_COUNT=0

write_evidence() {
  mkdir -p "$(dirname "$EVIDENCE_PATH")"
  STATUS="$STATUS" \
    DMG_PATH="$DMG_PATH" \
    TASK_ROOT="$TASK_ROOT" \
    SOURCE_APP="$SOURCE_APP" \
    INSTALLED_APP="$INSTALLED_APP" \
    BUNDLE_ID="$BUNDLE_ID" \
    DATABASE_PATH="$DATABASE_PATH" \
    LEGACY_DATABASE_PATH="$LEGACY_DATABASE_PATH" \
    LAUNCH_COUNT="$LAUNCH_COUNT" \
    DATABASE_INTEGRITY="$DATABASE_INTEGRITY" \
    LEGACY_DATABASE_ABSENT="$LEGACY_DATABASE_ABSENT" \
    APP_REMOVED="$APP_REMOVED" \
    USER_DATABASE_PRESERVED="$USER_DATABASE_PRESERVED" \
    DATA_CLEANUP="$DATA_CLEANUP" \
    MULTIPLE_PROFILE_CREDENTIALS_REMOVED="$MULTIPLE_PROFILE_CREDENTIALS_REMOVED" \
    SAFE_TRASH_PRESERVED="$SAFE_TRASH_PRESERVED" \
    APP_DATA_PRESERVED_ON_ORDINARY_REMOVAL="$APP_DATA_PRESERVED_ON_ORDINARY_REMOVAL" \
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
    "legacyDatabasePath": os.environ["LEGACY_DATABASE_PATH"],
    "successfulLaunches": int(os.environ["LAUNCH_COUNT"]),
    "databaseIntegrity": os.environ["DATABASE_INTEGRITY"],
    "legacyDatabaseAbsent": os.environ["LEGACY_DATABASE_ABSENT"] == "1",
    "appBundleRemoved": os.environ["APP_REMOVED"] == "1",
    "userDatabasePreservedOnOrdinaryRemoval": os.environ["USER_DATABASE_PRESERVED"] == "1",
    "explicitDataCleanup": os.environ["DATA_CLEANUP"],
    "multipleProfileCredentialsRemoved": os.environ["MULTIPLE_PROFILE_CREDENTIALS_REMOVED"] == "1",
    "safeTrashPreserved": os.environ["SAFE_TRASH_PRESERVED"] == "1",
    "appDataPreservedOnOrdinaryRemoval": os.environ["APP_DATA_PRESERVED_ON_ORDINARY_REMOVAL"] == "1",
}
with open(sys.argv[1], "w", encoding="utf-8") as evidence:
    json.dump(document, evidence, indent=2)
    evidence.write("\n")
PY
}

remove_task_credentials() {
  local index item service account
  for ((index = 0; index < TASK_CREDENTIAL_COUNT; index++)); do
    item="${TASK_CREDENTIALS[$index]}"
    IFS='|' read -r service account <<<"$item"
    HOME="$PROFILE_ROOT" CFFIXED_USER_HOME="$PROFILE_ROOT" \
      /usr/bin/security delete-generic-password -s "$service" -a "$account" "$TEST_KEYCHAIN" >/dev/null 2>&1 || true
  done
  TASK_CREDENTIALS=()
  TASK_CREDENTIAL_COUNT=0
}

restore_task_keychain() {
  [[ -n "$TEST_KEYCHAIN" ]] || return 0
  remove_task_credentials
  local result=0
  if [[ -n "$ORIGINAL_DEFAULT_KEYCHAIN" ]]; then
    /usr/bin/security default-keychain -d user -s "$ORIGINAL_DEFAULT_KEYCHAIN" >/dev/null 2>&1 || result=1
  fi
  if [[ "$ORIGINAL_KEYCHAIN_COUNT" -gt 0 ]]; then
    /usr/bin/security list-keychains -d user -s "${ORIGINAL_KEYCHAINS[@]}" >/dev/null 2>&1 || result=1
  fi
  if [[ -e "$TEST_KEYCHAIN" ]]; then
    /usr/bin/security delete-keychain "$TEST_KEYCHAIN" >/dev/null 2>&1 || result=1
  fi
  TEST_KEYCHAIN=""
  return "$result"
}

credential_exists() {
  local service="$1" account="$2"
  HOME="$PROFILE_ROOT" CFFIXED_USER_HOME="$PROFILE_ROOT" \
    /usr/bin/security find-generic-password -s "$service" -a "$account" "$TEST_KEYCHAIN" >/dev/null 2>&1
}

prepare_task_keychain() {
  local entry item service account password
  ORIGINAL_DEFAULT_KEYCHAIN="$(/usr/bin/security default-keychain -d user | sed -E 's/^[[:space:]]*"//; s/"[[:space:]]*$//')"
  while IFS= read -r entry; do
    entry="${entry#\"}"
    entry="${entry%\"}"
    if [[ -n "$entry" ]]; then
      ORIGINAL_KEYCHAINS[$ORIGINAL_KEYCHAIN_COUNT]="$entry"
      ORIGINAL_KEYCHAIN_COUNT=$((ORIGINAL_KEYCHAIN_COUNT + 1))
    fi
  done < <(/usr/bin/security list-keychains -d user)
  [[ -n "$ORIGINAL_DEFAULT_KEYCHAIN" ]] || { echo "Could not identify the runner's current default Keychain." >&2; return 1; }

  for item in 'com.startlan.fylura|ai-api-key' 'com.startlan.fylura.ai-profile|ai-api-key-qa_alpha' 'com.startlan.fylura.ai-profile|ai-api-key-qa_beta'; do
    IFS='|' read -r service account <<<"$item"
    if /usr/bin/security find-generic-password -s "$service" -a "$account" >/dev/null 2>&1; then
      echo "Refusing pre-existing Fylura Keychain credential fixture: $service/$account" >&2
      return 1
    fi
  done

  TEST_KEYCHAIN="$PROFILE_ROOT/Library/Keychains/fylura-test.keychain-db"
  password="$(/usr/bin/uuidgen)"
  mkdir -p "$(dirname "$TEST_KEYCHAIN")"
  HOME="$PROFILE_ROOT" CFFIXED_USER_HOME="$PROFILE_ROOT" \
    /usr/bin/security create-keychain -p "$password" "$TEST_KEYCHAIN" >/dev/null
  HOME="$PROFILE_ROOT" CFFIXED_USER_HOME="$PROFILE_ROOT" \
    /usr/bin/security unlock-keychain -p "$password" "$TEST_KEYCHAIN" >/dev/null
  HOME="$PROFILE_ROOT" CFFIXED_USER_HOME="$PROFILE_ROOT" \
    /usr/bin/security set-keychain-settings -lut 21600 "$TEST_KEYCHAIN" >/dev/null
  HOME="$PROFILE_ROOT" CFFIXED_USER_HOME="$PROFILE_ROOT" \
    /usr/bin/security list-keychains -d user -s "$TEST_KEYCHAIN" >/dev/null
  HOME="$PROFILE_ROOT" CFFIXED_USER_HOME="$PROFILE_ROOT" \
    /usr/bin/security default-keychain -d user -s "$TEST_KEYCHAIN" >/dev/null
}

add_task_credential() {
  local service="$1" account="$2" secret
  secret="$(/usr/bin/uuidgen)"
  TASK_CREDENTIALS[$TASK_CREDENTIAL_COUNT]="$service|$account"
  TASK_CREDENTIAL_COUNT=$((TASK_CREDENTIAL_COUNT + 1))
  HOME="$PROFILE_ROOT" CFFIXED_USER_HOME="$PROFILE_ROOT" \
    /usr/bin/security add-generic-password -U -A -s "$service" -a "$account" -w "$secret" "$TEST_KEYCHAIN" >/dev/null
  credential_exists "$service" "$account"
}

seed_cleanup_fixtures() {
  local cleanup_binary="$INSTALLED_APP/Contents/MacOS/$executable_name"
  local safe_new="$APP_SUPPORT_ROOT/.fylura-trash/items/task-owned-fixture/real-file.txt"
  local safe_legacy="$APP_SUPPORT_ROOT/.zen-canvas-trash/items/task-owned-fixture/real-file.txt"
  local safe_named="$APP_SUPPORT_ROOT/safe-trash/items/task-owned-fixture/real-file.txt"
  local external_new="$DOCUMENTS_SAFE_TRASH/user-file.txt"
  local external_legacy="$DOCUMENTS_LEGACY_TRASH/user-file.txt"
  local finder_trash_file="$FINDER_TRASH/user-file.txt"

  [[ -x "$SOURCE_APP/Contents/Resources/Uninstall Fylura Data.command" ]] || {
    echo "The Fylura app data cleanup command is missing or not executable in the DMG." >&2
    return 1
  }
  bash -n "$SOURCE_APP/Contents/Resources/Uninstall Fylura Data.command"
  [[ ! -e "$LAUNCH_AGENT_PATH" ]] || { echo "Refusing a pre-existing task-owned Fylura LaunchAgent fixture." >&2; return 1; }

  python3 - "$DATABASE_PATH" <<'PY'
import json
import sqlite3
import sys

connection = sqlite3.connect(sys.argv[1])
connection.execute(
    "INSERT INTO app_settings (key, value) VALUES (?, ?) "
    "ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    ("ai_settings_v1", json.dumps({"customProfiles": [{"id": "qa_alpha"}, {"id": "qa_beta"}]})),
)
connection.commit()
connection.close()
PY

  mkdir -p "$(dirname "$safe_new")" "$(dirname "$safe_legacy")" "$(dirname "$safe_named")" \
    "$CACHE_ROOT/quick-look" "$LOGS_ROOT" "$(dirname "$PREFERENCES_PATH")" \
    "$(dirname "$LAUNCH_AGENT_PATH")" "$DOCUMENTS_SAFE_TRASH" "$DOCUMENTS_LEGACY_TRASH" "$FINDER_TRASH" "$LEGACY_DATA_ROOT"
  printf '%s\n' 'task-owned database-adjacent data' >"$APP_SUPPORT_ROOT/unrelated-app-data.sentinel"
  printf '%s\n' 'task-owned cache' >"$CACHE_ROOT/quick-look/cache.sentinel"
  printf '%s\n' 'task-owned log' >"$LOGS_ROOT/app.log"
  printf '%s\n' 'task-owned preference' >"$PREFERENCES_PATH"
  printf '%s\n' 'real file in current app-data Safe Trash' >"$safe_new"
  printf '%s\n' 'real file in legacy app-data Safe Trash' >"$safe_legacy"
  printf '%s\n' 'real file in named app-data Safe Trash' >"$safe_named"
  printf '%s\n' 'real file in user Documents Safe Trash' >"$external_new"
  printf '%s\n' 'real file in legacy user Documents Safe Trash' >"$external_legacy"
  printf '%s\n' 'real file in Finder Trash' >"$finder_trash_file"
  printf '%s\n' 'legacy development data' >"$LEGACY_DATA_ROOT/legacy-development-data.sentinel"
  python3 - "$LAUNCH_AGENT_PATH" "$cleanup_binary" <<'PY'
import os
import plistlib
import sys

with open(sys.argv[1], "wb") as agent:
    plistlib.dump({"Label": "Fylura", "ProgramArguments": [os.path.realpath(sys.argv[2]), "--background"]}, agent)
PY

  prepare_task_keychain
  add_task_credential 'com.startlan.fylura' 'ai-api-key'
  add_task_credential 'com.startlan.fylura.ai-profile' 'ai-api-key-qa_alpha'
  add_task_credential 'com.startlan.fylura.ai-profile' 'ai-api-key-qa_beta'
}

run_explicit_data_cleanup() {
  local output status cleanup_binary="$INSTALLED_APP/Contents/MacOS/$executable_name"
  if output="$(HOME="$PROFILE_ROOT" CFFIXED_USER_HOME="$PROFILE_ROOT" TMPDIR="$TEMP_ROOT" \
    "$cleanup_binary" --fylura-uninstall-clean-user-data 2>&1)"; then
    DATA_CLEANUP="PASS"
  else
    status=$?
    printf '%s\n' "$output" >&2
    echo "Fylura's explicit data cleanup command failed with exit code $status." >&2
    DATA_CLEANUP="FAIL"
    return 1
  fi

  [[ ! -e "$DATABASE_PATH" ]] || { echo "Fylura's database remains after explicit cleanup." >&2; return 1; }
  [[ ! -e "$CACHE_ROOT" ]] || { echo "Fylura cache remains after explicit cleanup." >&2; return 1; }
  [[ ! -e "$LOGS_ROOT" ]] || { echo "Fylura logs remain after explicit cleanup." >&2; return 1; }
  [[ ! -e "$PREFERENCES_PATH" ]] || { echo "Fylura preferences remain after explicit cleanup." >&2; return 1; }
  [[ ! -e "$LAUNCH_AGENT_PATH" ]] || { echo "Fylura LaunchAgent remains after explicit cleanup." >&2; return 1; }
  [[ -f "$LEGACY_DATA_ROOT/legacy-development-data.sentinel" ]] || { echo "Legacy development data was changed by Fylura cleanup." >&2; return 1; }
  [[ -f "$DOCUMENTS_SAFE_TRASH/user-file.txt" && -f "$DOCUMENTS_LEGACY_TRASH/user-file.txt" ]] || {
    echo "User Safe Trash data was changed by Fylura cleanup." >&2
    return 1
  }
  [[ -f "$FINDER_TRASH/user-file.txt" ]] || { echo "Finder Trash was changed by Fylura cleanup." >&2; return 1; }

  python3 - "$APP_SUPPORT_ROOT" <<'PY'
import os
import sys

expected = {"safe-trash", ".fylura-trash", ".zen-canvas-trash"}
actual = set(os.listdir(sys.argv[1]))
if actual != expected:
    raise SystemExit("unexpected app-support items remain after cleanup")
PY
  for safe_file in \
    "$APP_SUPPORT_ROOT/.fylura-trash/items/task-owned-fixture/real-file.txt" \
    "$APP_SUPPORT_ROOT/.zen-canvas-trash/items/task-owned-fixture/real-file.txt" \
    "$APP_SUPPORT_ROOT/safe-trash/items/task-owned-fixture/real-file.txt"; do
    [[ -f "$safe_file" ]] || { echo "Safe Trash data was changed by Fylura cleanup: $safe_file" >&2; return 1; }
  done

  for item in 'com.startlan.fylura|ai-api-key' 'com.startlan.fylura.ai-profile|ai-api-key-qa_alpha' 'com.startlan.fylura.ai-profile|ai-api-key-qa_beta'; do
    IFS='|' read -r service account <<<"$item"
    if credential_exists "$service" "$account"; then
      echo "Fylura Keychain credential remains after explicit cleanup: $service/$account" >&2
      return 1
    fi
  done
  MULTIPLE_PROFILE_CREDENTIALS_REMOVED=1
  SAFE_TRASH_PRESERVED=1
}

assert_data_preserved_after_ordinary_removal() {
  [[ -d "$CACHE_ROOT" && -f "$CACHE_ROOT/quick-look/cache.sentinel" ]] || { echo "Fylura cache did not survive ordinary app removal." >&2; return 1; }
  [[ -d "$LOGS_ROOT" && -f "$LOGS_ROOT/app.log" ]] || { echo "Fylura logs did not survive ordinary app removal." >&2; return 1; }
  [[ -f "$PREFERENCES_PATH" && -f "$LAUNCH_AGENT_PATH" ]] || { echo "Fylura preferences or login item did not survive ordinary app removal." >&2; return 1; }
  [[ -f "$APP_SUPPORT_ROOT/unrelated-app-data.sentinel" ]] || { echo "Fylura app data did not survive ordinary app removal." >&2; return 1; }
  [[ -f "$APP_SUPPORT_ROOT/.fylura-trash/items/task-owned-fixture/real-file.txt" \
    && -f "$APP_SUPPORT_ROOT/.zen-canvas-trash/items/task-owned-fixture/real-file.txt" \
    && -f "$APP_SUPPORT_ROOT/safe-trash/items/task-owned-fixture/real-file.txt" ]] || {
    echo "App-support Safe Trash files did not survive ordinary app removal." >&2
    return 1
  }
  [[ -f "$DOCUMENTS_SAFE_TRASH/user-file.txt" && -f "$DOCUMENTS_LEGACY_TRASH/user-file.txt" \
    && -f "$FINDER_TRASH/user-file.txt" ]] || {
    echo "User or Finder Trash data did not survive ordinary app removal." >&2
    return 1
  }
  [[ -f "$LEGACY_DATA_ROOT/legacy-development-data.sentinel" ]] || { echo "Legacy development data did not survive ordinary app removal." >&2; return 1; }
  python3 - "$DATABASE_PATH" <<'PY'
import json
import sqlite3
import sys

connection = sqlite3.connect(sys.argv[1])
row = connection.execute("SELECT value FROM app_settings WHERE key = ?", ("ai_settings_v1",)).fetchone()
connection.close()
if row is None:
    raise SystemExit("AI profile settings did not survive ordinary app removal")
profiles = json.loads(row[0]).get("customProfiles", [])
if {profile.get("id") for profile in profiles} != {"qa_alpha", "qa_beta"}:
    raise SystemExit("AI profile metadata did not survive ordinary app removal")
PY
  for item in 'com.startlan.fylura|ai-api-key' 'com.startlan.fylura.ai-profile|ai-api-key-qa_alpha' 'com.startlan.fylura.ai-profile|ai-api-key-qa_beta'; do
    IFS='|' read -r service account <<<"$item"
    credential_exists "$service" "$account" || {
      echo "Fylura Keychain data did not survive ordinary app removal: $service/$account" >&2
      return 1
    }
  done
  APP_DATA_PRESERVED_ON_ORDINARY_REMOVAL=1
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

  if ! restore_task_keychain; then
    echo "Failed to restore the isolated task-owned macOS test keychain." >&2
    STATUS="FAIL"
    exit_code=1
  fi

  if [[ "$exit_code" -ne 0 ]]; then
    STATUS="FAIL"
  fi
  write_evidence || exit_code=1
  if [[ "$exit_code" -eq 0 && "$STATUS" == "PASS" ]]; then
    echo "PASS: Fylura DMG identity, ordinary-remove data preservation, reinstall, explicit cleanup, Keychain profiles, and Safe Trash preservation."
  fi
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
  if [[ -e "$LEGACY_DATABASE_PATH" ]]; then
    LEGACY_DATABASE_ABSENT=0
    echo "Fylura created or altered the legacy database path in the isolated profile: $LEGACY_DATABASE_PATH" >&2
    return 1
  fi
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
      sleep 2
      process_state="$(ps -p "$APP_PID" -o stat= 2>/dev/null | tr -d '[:space:]' || true)"
      if ! kill -0 "$APP_PID" 2>/dev/null || [[ "$process_state" == Z* ]]; then
        wait "$APP_PID" 2>/dev/null || true
        APP_PID=""
        cat "$log_path" >&2
        echo "Fylura exited after opening its isolated database on launch $launch_number." >&2
        return 1
      fi
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
seed_cleanup_fixtures

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
USER_DATABASE_PRESERVED=1
assert_data_preserved_after_ordinary_removal

ditto "$SOURCE_APP" "$INSTALLED_APP"
APP_REMOVED=0
APP_BINARY="$INSTALLED_APP/Contents/MacOS/$executable_name"
launch_and_check
stop_app
check_database
run_explicit_data_cleanup

case "$INSTALLED_APP/" in
  "$TASK_ROOT"/*) ;;
  *) echo "Refusing to remove an app bundle outside the task-owned directory." >&2; exit 1 ;;
esac
/bin/rm -R "$INSTALLED_APP"
if [[ -e "$INSTALLED_APP" ]]; then
  echo "Task-owned Fylura.app remained after final test removal." >&2
  exit 1
fi
APP_REMOVED=1

hdiutil detach "$MOUNT_POINT" -quiet
MOUNT_ATTACHED=0
STATUS="PASS"
