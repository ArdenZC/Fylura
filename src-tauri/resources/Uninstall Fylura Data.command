#!/bin/bash
set -u

resource_dir="$(cd "$(dirname "$0")" && pwd)"
app_executable="${resource_dir}/../MacOS/fylura"
if [[ ! -x "$app_executable" ]]; then
  osascript -e 'display alert "Fylura cleanup unavailable" message "Open this helper from the installed Fylura.app bundle." as critical'
  exit 1
fi

first_language="$(/usr/bin/defaults read -g AppleLanguages 2>/dev/null | /usr/bin/sed -n 's/^[[:space:]]*"\([^"]*\)".*/\1/p' | /usr/bin/head -n 1 || true)"
if [[ "$first_language" == zh* ]]; then
  warning='这会删除此 Mac 用户的 Fylura 设置、数据库、索引、缓存、日志、API 密钥和 Fylura 登录项。删除数据库会移除 History/Restore 恢复记录。Safe Trash 中的真实文件会保留在原位置；普通移除 Fylura.app 不会删除这些数据。'
  title='彻底清理 Fylura 应用数据'
  done_title='Fylura 数据清理完成'
  failed_title='Fylura 数据清理未完成'
  cancel_label='取消'
  delete_label='删除应用数据'
else
  warning='This removes this Mac user’s Fylura settings, database, index, cache, logs, API credentials, and Fylura login item. Removing the database also removes History/Restore records. Real files in Safe Trash stay at their current paths. Dragging Fylura.app to Trash by itself does not remove this data.'
  title='Remove Fylura app data'
  done_title='Fylura data cleanup complete'
  failed_title='Fylura data cleanup incomplete'
  cancel_label='Cancel'
  delete_label='Delete app data'
fi

if /usr/bin/pgrep -f '[F]ylura.app/Contents/MacOS/fylura' >/dev/null 2>&1; then
  /usr/bin/osascript - "$title" <<'APPLESCRIPT'
on run argv
  display alert (item 1 of argv) message "Quit Fylura before running data cleanup." as critical
end run
APPLESCRIPT
  exit 1
fi

choice="$(/usr/bin/osascript - "$title" "$warning" "$cancel_label" "$delete_label" <<'APPLESCRIPT'
on run argv
  display dialog (item 2 of argv) with title (item 1 of argv) buttons {item 3 of argv, item 4 of argv} default button (item 3 of argv) cancel button (item 3 of argv) with icon caution
end run
APPLESCRIPT
 )"
choice_status=$?
if [[ $choice_status -ne 0 || "$choice" != "button returned:$delete_label" ]]; then
  exit 0
fi

output="$("$app_executable" --fylura-uninstall-clean-user-data 2>&1)"
status=$?
if [[ $status -eq 0 ]]; then
  /usr/bin/osascript - "$done_title" "$output" <<'APPLESCRIPT'
on run argv
  display dialog (item 2 of argv) with title (item 1 of argv) buttons {"OK"} default button "OK" with icon note
end run
APPLESCRIPT
  exit 0
fi

/usr/bin/osascript - "$failed_title" "$output" <<'APPLESCRIPT'
on run argv
  display alert (item 1 of argv) message (item 2 of argv) as critical buttons {"OK"} default button "OK"
end run
APPLESCRIPT
exit "$status"
