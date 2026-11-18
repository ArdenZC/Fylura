#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <fylura.dmg> <evidence.json>" >&2
  exit 2
fi
if [[ "${GITHUB_ACTIONS:-}" != "true" || "${RUNNER_ENVIRONMENT:-}" != "github-hosted" ]]; then
  echo "Refusing destructive macOS lifecycle verification outside a disposable GitHub-hosted macOS VM." >&2
  exit 1
fi

DMG_PATH="$(cd "$(dirname "$1")" && pwd -P)/$(basename "$1")"
EVIDENCE_PATH="$2"
RUNNER_TEMP="${RUNNER_TEMP:?RUNNER_TEMP must be set by the isolated macOS runner}"
RUN_ID="${GITHUB_RUN_ID:?GITHUB_RUN_ID must be set by GitHub Actions}"
TASK_ROOT="$RUNNER_TEMP/fylura-dmg-runtime-$RUN_ID"
MOUNT_POINT="$TASK_ROOT/mount"
INSTALL_ROOT="$TASK_ROOT/Applications"
INSTALLED_APP="$INSTALL_ROOT/Fylura.app"
PROFILE_ROOT="${HOME:?HOME must identify the disposable runner account}"
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
DOCUMENTS_SAFE_TRASH="$PROFILE_ROOT/Documents/.fylura-trash/operations/task-owned-fixture-$RUN_ID"
DOCUMENTS_LEGACY_TRASH="$PROFILE_ROOT/Documents/.zen-canvas-trash/operations/task-owned-fixture-$RUN_ID"
FINDER_TRASH="$PROFILE_ROOT/.Trash"
FINDER_TRASH_FILE="$FINDER_TRASH/fylura-qa-$RUN_ID.txt"
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
TEST_KEYCHAIN_CREATED=0
TASK_CREDENTIALS=()
TASK_CREDENTIAL_COUNT=0
TASK_ROOT_CREATED=0
APP_PROFILE_ROOTS_OWNED=0
LEGACY_DATA_ROOT_OWNED=0
DOCUMENTS_SAFE_TRASH_FILE_CREATED=0
DOCUMENTS_LEGACY_TRASH_FILE_CREATED=0
FINDER_TRASH_FILE_CREATED=0
OWNED_FIXTURE_DIRECTORIES=()
PROFILE_ROOTS_TO_CHECK=()

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
  if [[ "$TEST_KEYCHAIN_CREATED" -eq 1 && -e "$TEST_KEYCHAIN" ]]; then
    /usr/bin/security delete-keychain "$TEST_KEYCHAIN" >/dev/null 2>&1 || result=1
  fi
  TEST_KEYCHAIN=""
  TEST_KEYCHAIN_CREATED=0
  return "$result"
}

credential_exists() {
  local service="$1" account="$2"
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

  local runner_home="${HOME:?HOME must identify the disposable runner account}"
  TEST_KEYCHAIN="$runner_home/Library/Keychains/fylura-test-$RUN_ID.keychain-db"
  [[ ! -e "$TEST_KEYCHAIN" ]] || {
    echo "Refusing a pre-existing task-owned Keychain path: $TEST_KEYCHAIN" >&2
    return 1
  }
  password="$(/usr/bin/uuidgen)"
  if /usr/bin/security create-keychain -p "$password" "$TEST_KEYCHAIN" >/dev/null; then
    TEST_KEYCHAIN_CREATED=1
  else
    [[ -e "$TEST_KEYCHAIN" ]] && TEST_KEYCHAIN_CREATED=1
    return 1
  fi
  /usr/bin/security unlock-keychain -p "$password" "$TEST_KEYCHAIN" >/dev/null
  /usr/bin/security set-keychain-settings -lut 21600 "$TEST_KEYCHAIN" >/dev/null
  # Point the disposable runner's real login search list at the task-owned
  # keychain so Security.framework and the security CLI see the same unlocked
  # test credentials; restore it on exit.
  /usr/bin/security list-keychains -d user -s "$TEST_KEYCHAIN" >/dev/null
  /usr/bin/security default-keychain -d user -s "$TEST_KEYCHAIN" >/dev/null
}

preflight_profile_paths() {
  local candidate
  PROFILE_ROOTS_TO_CHECK=(
    "$APP_SUPPORT_ROOT"
    "$LEGACY_DATA_ROOT"
    "$CACHE_ROOT"
    "$PREFERENCES_PATH"
    "$LOGS_ROOT"
    "$LAUNCH_AGENT_PATH"
    "$DOCUMENTS_SAFE_TRASH"
    "$DOCUMENTS_LEGACY_TRASH"
    "$FINDER_TRASH_FILE"
    "$PROFILE_ROOT/Library/Keychains/fylura-test-$RUN_ID.keychain-db"
  )
  [[ -d "$PROFILE_ROOT" && ! -L "$PROFILE_ROOT" ]] || {
    echo "Refusing an invalid or symlinked disposable runner home: $PROFILE_ROOT" >&2
    return 1
  }
  if [[ -n "${CFFIXED_USER_HOME:-}" && "$CFFIXED_USER_HOME" != "$PROFILE_ROOT" ]]; then
    echo "Refusing a runner with CFFIXED_USER_HOME outside its actual login home." >&2
    return 1
  fi
  for candidate in "${PROFILE_ROOTS_TO_CHECK[@]}"; do
    local current="/" component
    local -a path_components
    IFS='/' read -r -a path_components <<<"${candidate#/}"
    for component in "${path_components[@]}"; do
      current="${current%/}/$component"
      [[ ! -L "$current" ]] || {
        echo "Refusing a symlink in a Fylura lifecycle test path: $current" >&2
        return 1
      }
    done
    [[ ! -e "$candidate" ]] || {
      echo "Refusing a pre-existing Fylura lifecycle test path: $candidate" >&2
      return 1
    }
  done
  [[ -d "$PROFILE_ROOT/Library/Keychains" ]] || {
    echo "The disposable runner's Keychains directory is missing; refusing to create a replacement." >&2
    return 1
  }
  APP_PROFILE_ROOTS_OWNED=1
}

ensure_owned_fixture_directory() {
  local target="$1" current component relative_target prefix_length
  local -a components
  case "$target" in
    "$PROFILE_ROOT"/*) ;;
    *) echo "Refusing a fixture directory outside the disposable runner home: $target" >&2; return 1 ;;
  esac
  prefix_length=$((${#PROFILE_ROOT} + 1))
  relative_target="${target:prefix_length}"
  IFS='/' read -r -a components <<<"$relative_target"
  current="$PROFILE_ROOT"
  for component in "${components[@]}"; do
    current="$current/$component"
    if [[ -L "$current" ]]; then
      echo "Refusing a symlink in a task-owned fixture path: $current" >&2
      return 1
    elif [[ -e "$current" ]]; then
      [[ -d "$current" ]] || { echo "Fixture path is not a directory: $current" >&2; return 1; }
    else
      mkdir "$current"
      OWNED_FIXTURE_DIRECTORIES+=("$current")
    fi
  done
}

add_task_credential() {
  local service="$1" account="$2" app_path="$3" secret
  secret="$(/usr/bin/uuidgen)"
  TASK_CREDENTIALS[$TASK_CREDENTIAL_COUNT]="$service|$account"
  TASK_CREDENTIAL_COUNT=$((TASK_CREDENTIAL_COUNT + 1))
  /usr/bin/security add-generic-password -U -A -T "$app_path" -s "$service" -a "$account" -w "$secret" "$TEST_KEYCHAIN" >/dev/null
  credential_exists "$service" "$account"
}

seed_cleanup_fixtures() {
  local cleanup_binary="$INSTALLED_APP/Contents/MacOS/$executable_name"
  local safe_new="$APP_SUPPORT_ROOT/.fylura-trash/items/task-owned-fixture/real-file.txt"
  local safe_legacy="$APP_SUPPORT_ROOT/.zen-canvas-trash/items/task-owned-fixture/real-file.txt"
  local safe_named="$APP_SUPPORT_ROOT/safe-trash/items/task-owned-fixture/real-file.txt"
  local external_new="$DOCUMENTS_SAFE_TRASH/user-file.txt"
  local external_legacy="$DOCUMENTS_LEGACY_TRASH/user-file.txt"
  local finder_trash_file="$FINDER_TRASH_FILE"

  [[ -x "$SOURCE_APP/Contents/Resources/Uninstall Fylura Data.command" ]] || {
    echo "The Fylura app data cleanup command is missing or not executable in the DMG." >&2
    return 1
  }
  bash -n "$SOURCE_APP/Contents/Resources/Uninstall Fylura Data.command"
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

  ensure_owned_fixture_directory "$(dirname "$safe_new")"
  ensure_owned_fixture_directory "$(dirname "$safe_legacy")"
  ensure_owned_fixture_directory "$(dirname "$safe_named")"
  ensure_owned_fixture_directory "$CACHE_ROOT/quick-look"
  ensure_owned_fixture_directory "$LOGS_ROOT"
  ensure_owned_fixture_directory "$(dirname "$PREFERENCES_PATH")"
  ensure_owned_fixture_directory "$(dirname "$LAUNCH_AGENT_PATH")"
  ensure_owned_fixture_directory "$DOCUMENTS_SAFE_TRASH"
  ensure_owned_fixture_directory "$DOCUMENTS_LEGACY_TRASH"
  ensure_owned_fixture_directory "$FINDER_TRASH"
  ensure_owned_fixture_directory "$LEGACY_DATA_ROOT"
  LEGACY_DATA_ROOT_OWNED=1
  printf '%s\n' 'task-owned database-adjacent data' >"$APP_SUPPORT_ROOT/unrelated-app-data.sentinel"
  printf '%s\n' 'task-owned cache' >"$CACHE_ROOT/quick-look/cache.sentinel"
  printf '%s\n' 'task-owned log' >"$LOGS_ROOT/app.log"
  printf '%s\n' 'task-owned preference' >"$PREFERENCES_PATH"
  printf '%s\n' 'real file in current app-data Safe Trash' >"$safe_new"
  printf '%s\n' 'real file in legacy app-data Safe Trash' >"$safe_legacy"
  printf '%s\n' 'real file in named app-data Safe Trash' >"$safe_named"
  DOCUMENTS_SAFE_TRASH_FILE_CREATED=1
  printf '%s\n' 'real file in user Documents Safe Trash' >"$external_new"
  DOCUMENTS_LEGACY_TRASH_FILE_CREATED=1
  printf '%s\n' 'real file in legacy user Documents Safe Trash' >"$external_legacy"
  FINDER_TRASH_FILE_CREATED=1
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
  add_task_credential 'com.startlan.fylura' 'ai-api-key' "$cleanup_binary"
  add_task_credential 'com.startlan.fylura.ai-profile' 'ai-api-key-qa_alpha' "$cleanup_binary"
  add_task_credential 'com.startlan.fylura.ai-profile' 'ai-api-key-qa_beta' "$cleanup_binary"
}

run_explicit_data_cleanup() {
  local output status cleanup_binary="$INSTALLED_APP/Contents/MacOS/$executable_name"
  echo "Starting explicit Fylura user-data cleanup."
  if output="$(python3 - "$cleanup_binary" "$TEMP_ROOT" <<'PY'
import os
import subprocess
import sys

environment = os.environ.copy()
environment.update(
    TMPDIR=sys.argv[2],
    FYLURA_CLEANUP_DIAGNOSTICS="1",
)
# Use the disposable runner's real login account so Security.framework sees
# the same default Keychain that the security CLI configured for this test.
try:
    result = subprocess.run(
        [sys.argv[1], "--fylura-uninstall-clean-user-data"],
        env=environment,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        timeout=180,
        check=False,
    )
except subprocess.TimeoutExpired as error:
    output = error.stdout or ""
    if isinstance(output, bytes):
        output = output.decode("utf-8", errors="replace")
    print(output, end="")
    print("Fylura explicit user-data cleanup timed out after 180 seconds.", file=sys.stderr)
    raise SystemExit(124)

print(result.stdout, end="")
raise SystemExit(result.returncode)
PY
  )"; then
    DATA_CLEANUP="VERIFYING"
    echo "Fylura's data cleanup command returned successfully; verifying the remaining paths."
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
  [[ -f "$FINDER_TRASH_FILE" ]] || { echo "Finder Trash was changed by Fylura cleanup." >&2; return 1; }

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
  DATA_CLEANUP="PASS"
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
    && -f "$FINDER_TRASH_FILE" ]] || {
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

  if [[ "$DATA_CLEANUP" == "VERIFYING" ]]; then
    DATA_CLEANUP="FAIL"
  fi

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

  if [[ "$APP_PROFILE_ROOTS_OWNED" -eq 1 ]]; then
    local owned_path
    for owned_path in "$APP_SUPPORT_ROOT" "$CACHE_ROOT" "$LOGS_ROOT"; do
      if [[ -L "$owned_path" ]]; then
        echo "Refusing to remove a symlinked Fylura data path during test cleanup: $owned_path" >&2
        STATUS="FAIL"
        exit_code=1
      elif [[ -e "$owned_path" ]]; then
        /bin/rm -R "$owned_path" || { echo "Failed to remove task-owned Fylura test data: $owned_path" >&2; STATUS="FAIL"; exit_code=1; }
      fi
    done
    for owned_path in "$PREFERENCES_PATH" "$LAUNCH_AGENT_PATH"; do
      if [[ -L "$owned_path" ]]; then
        echo "Refusing to remove a symlinked Fylura file during test cleanup: $owned_path" >&2
        STATUS="FAIL"
        exit_code=1
      elif [[ -e "$owned_path" ]]; then
        /bin/rm -f "$owned_path" || { echo "Failed to remove task-owned Fylura test file: $owned_path" >&2; STATUS="FAIL"; exit_code=1; }
      fi
    done
  fi

  if [[ "$LEGACY_DATA_ROOT_OWNED" -eq 1 ]]; then
    if [[ -L "$LEGACY_DATA_ROOT" ]]; then
      echo "Refusing to remove a symlinked legacy fixture path: $LEGACY_DATA_ROOT" >&2
      STATUS="FAIL"
      exit_code=1
    elif [[ -e "$LEGACY_DATA_ROOT" ]]; then
      /bin/rm -R "$LEGACY_DATA_ROOT" || { echo "Failed to remove task-owned legacy fixture data." >&2; STATUS="FAIL"; exit_code=1; }
    fi
  fi

  if [[ "$DOCUMENTS_SAFE_TRASH_FILE_CREATED" -eq 1 ]]; then
    /bin/rm -f "$DOCUMENTS_SAFE_TRASH/user-file.txt" || { STATUS="FAIL"; exit_code=1; }
  fi
  if [[ "$DOCUMENTS_LEGACY_TRASH_FILE_CREATED" -eq 1 ]]; then
    /bin/rm -f "$DOCUMENTS_LEGACY_TRASH/user-file.txt" || { STATUS="FAIL"; exit_code=1; }
  fi
  if [[ "$FINDER_TRASH_FILE_CREATED" -eq 1 ]]; then
    /bin/rm -f "$FINDER_TRASH_FILE" || { STATUS="FAIL"; exit_code=1; }
  fi
  for ((directory_index=${#OWNED_FIXTURE_DIRECTORIES[@]}-1; directory_index>=0; directory_index--)); do
    if [[ -L "${OWNED_FIXTURE_DIRECTORIES[$directory_index]}" ]]; then
      echo "Refusing to remove a symlinked task-owned fixture directory: ${OWNED_FIXTURE_DIRECTORIES[$directory_index]}" >&2
      STATUS="FAIL"
      exit_code=1
    elif [[ -d "${OWNED_FIXTURE_DIRECTORIES[$directory_index]}" ]]; then
      rmdir "${OWNED_FIXTURE_DIRECTORIES[$directory_index]}" 2>/dev/null || {
        echo "Task-owned fixture directory is not empty after cleanup: ${OWNED_FIXTURE_DIRECTORIES[$directory_index]}" >&2
        STATUS="FAIL"
        exit_code=1
      }
    fi
  done

  if [[ "$TASK_ROOT_CREATED" -eq 1 ]]; then
    case "$TASK_ROOT/" in
      "$RUNNER_TEMP"/*) /bin/rm -R "$TASK_ROOT" || {
        echo "Failed to remove the task-owned macOS lifecycle fixture root." >&2
        STATUS="FAIL"
        exit_code=1
      } ;;
      *)
        echo "Refusing to remove a lifecycle fixture root outside RUNNER_TEMP." >&2
        STATUS="FAIL"
        exit_code=1
        ;;
    esac
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

TASK_ROOT_CREATED=1
mkdir -p "$MOUNT_POINT" "$INSTALL_ROOT" "$TEMP_ROOT"
preflight_profile_paths
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
