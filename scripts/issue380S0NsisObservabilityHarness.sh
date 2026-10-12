#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
probe_source="$repo_root/scripts/issue380S0NsisObservabilityProbe.nsi"
output_root="${1:?usage: issue380S0NsisObservabilityHarness.sh OUTPUT_DIR}"
mkdir -p "$output_root"
output_root="$(cd "$output_root" && pwd)"
temp_root="$(mktemp -d)"
trap 'rm -rf "$temp_root"' EXIT

if ! command -v makensis >/dev/null 2>&1; then
  echo "makensis is required" >&2
  exit 2
fi
wine_command="$(command -v wine64 || command -v wine || true)"
wineboot_command="$(command -v wineboot || true)"
winepath_command="$(command -v winepath || true)"
if [[ -z "$wine_command" || -z "$wineboot_command" || -z "$winepath_command" ]]; then
  echo "wine64/wine, wineboot, and winepath are required for the isolated NSIS probe" >&2
  exit 2
fi

compiler_header="$(makensis -HDRINFO 2>&1)"
if grep -q 'NSIS_CONFIG_LOG' <<<"$compiler_header"; then
  logset_support="present"
else
  logset_support="absent"
fi
printf 'makensis_version=%s\nNSIS_CONFIG_LOG=%s\n' \
  "$(makensis -VERSION)" "$logset_support" > "$output_root/observability-results.txt"

if [[ "$logset_support" == "absent" ]]; then
  cat > "$temp_root/logset-negative.nsi" <<'NSI'
Unicode True
Name "Issue 380 S0 LogSet negative probe"
OutFile "logset-negative.exe"
Section
  LogSet on
SectionEnd
NSI
  set +e
  makensis -NOCD -V2 "$temp_root/logset-negative.nsi" > "$temp_root/logset-negative.out" 2>&1
  logset_rc=$?
  set -e
  if [[ $logset_rc -eq 0 ]] || ! grep -q 'LogSet specified, NSIS_CONFIG_LOG not defined' "$temp_root/logset-negative.out"; then
    cat "$temp_root/logset-negative.out" >&2
    echo "LogSet negative compile did not confirm disabled logging support" >&2
    exit 1
  fi
  printf 'LogSet_negative_compile=PASS(exit=%s,error=NSIS_CONFIG_LOG_not_defined)\n' \
    "$logset_rc" >> "$output_root/observability-results.txt"
fi

export WINEDEBUG=-all
export WINEARCH=win32
for mode in detail file; do
  mode_dir="$output_root/$mode"
  prefix="$temp_root/wine-$mode"
  mkdir -p "$mode_dir"
  export WINEPREFIX="$prefix"

  (
    cd "$mode_dir"
    makensis -NOCD -V2 "-DTRACE_MODE=$mode" -DRUN_MODE=stage \
      -DOUTPUT_NAME=stage.exe "$probe_source" > compile-stage.log 2>&1
    stage_sha="$(sha256sum stage.exe | cut -d ' ' -f1)"
    makensis -NOCD -V2 "-DTRACE_MODE=$mode" -DRUN_MODE=abort \
      -DOUTPUT_NAME=installer-abort.exe "$probe_source" > compile-abort.log 2>&1
    abort_sha="$(sha256sum installer-abort.exe | cut -d ' ' -f1)"
    printf '%s_stage_exe_sha256=%s\n%s_abort_exe_sha256=%s\n' \
      "$mode" "$stage_sha" "$mode" "$abort_sha" >> "$output_root/observability-results.txt"
  )

  "$wineboot_command" -u >/dev/null 2>&1 || true
  installer_log_path="$("$winepath_command" -w "$mode_dir/installer-runtime.log")"
  uninstaller_log_path="$("$winepath_command" -w "$mode_dir/uninstaller-runtime.log")"
  set +e
  "$wine_command" "$mode_dir/stage.exe" /S > "$mode_dir/stage.stdout.txt" 2> "$mode_dir/stage.stderr.txt"
  stage_rc=$?
  "$wine_command" "$mode_dir/installer-abort.exe" /S \
    "/LOG=$installer_log_path" \
    > "$mode_dir/installer.stdout.txt" 2> "$mode_dir/installer.stderr.txt"
  installer_rc=$?
  "$wine_command" "$mode_dir/generated-uninstaller.exe" /S \
    "/LOG=$uninstaller_log_path" \
    > "$mode_dir/uninstaller.stdout.txt" 2> "$mode_dir/uninstaller.stderr.txt"
  uninstaller_rc=$?
  set -e

  if [[ $stage_rc -ne 0 || $installer_rc -ne 37 || $uninstaller_rc -ne 42 ]]; then
    printf 'mode=%s stage_exit=%s installer_abort_exit=%s uninstaller_abort_exit=%s\n' \
      "$mode" "$stage_rc" "$installer_rc" "$uninstaller_rc" >&2
    cat "$mode_dir"/*.stderr.txt >&2
    exit 1
  fi

  installer_log_exists=false
  uninstaller_log_exists=false
  [[ -s "$mode_dir/installer-runtime.log" ]] && installer_log_exists=true
  [[ -s "$mode_dir/uninstaller-runtime.log" ]] && uninstaller_log_exists=true
  printf '%s_stage_exit=%s\n%s_installer_abort_exit=%s\n%s_uninstaller_abort_exit=%s\n%s_installer_slash_log_file=%s\n%s_uninstaller_slash_log_file=%s\n' \
    "$mode" "$stage_rc" "$mode" "$installer_rc" "$mode" "$uninstaller_rc" \
    "$mode" "$installer_log_exists" "$mode" "$uninstaller_log_exists" \
    >> "$output_root/observability-results.txt"
done

file_trace="$output_root/file/direct-evidence.log"
if [[ ! -s "$file_trace" ]]; then
  echo "explicit NSIS FileWrite trace was not present after Abort and process exit" >&2
  exit 1
fi
grep -F 'context=installer' "$file_trace" >/dev/null
grep -F 'context=uninstaller' "$file_trace" >/dev/null
grep -F 'expected=35150 actual=35149 result=FAIL' "$file_trace" >/dev/null

if [[ -e "$output_root/detail/installer-runtime.log" || -e "$output_root/detail/uninstaller-runtime.log" ]]; then
  printf 'result=NSIS_SLASH_LOG_CREATED_FILES_BUT_RUNTIME_CONTENT_REQUIRES_INSPECTION\n' \
    >> "$output_root/observability-results.txt"
else
  printf 'result=DETAILPRINT_AND_SLASH_LOG_DID_NOT_PERSIST_WITHOUT_NSIS_CONFIG_LOG\n' \
    >> "$output_root/observability-results.txt"
fi
printf 'explicit_file_trace_sha256=%s\n' "$(sha256sum "$file_trace" | cut -d ' ' -f1)" \
  >> "$output_root/observability-results.txt"
printf 'explicit_file_trace_lines=%s\n' "$(wc -l < "$file_trace" | tr -d ' ')" \
  >> "$output_root/observability-results.txt"

rm -f "$output_root"/{detail,file}/{stage.exe,installer-abort.exe,generated-uninstaller.exe}
echo "NSIS observability probe passed; results: $output_root/observability-results.txt"
