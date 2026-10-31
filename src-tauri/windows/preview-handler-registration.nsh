; Canonical W4-04 production registration identity and association matrix.
; Rust contract tests include this file and compare every product constant and
; extension with the handler/planner. Keep this file limited to literals so
; the per-machine NSIS authority can consume the same guarded representation.
!ifndef ZC_PREVIEW_HANDLER_REGISTRATION_NSH
!define ZC_PREVIEW_HANDLER_REGISTRATION_NSH

!define ZC_PREVIEW_PRODUCTION_CLSID "{3D1A446C-162E-4313-A026-8ADC792C4862}"
!define ZC_PREVIEW_FRIENDLY_NAME "Fylura Preview Handler"
!define ZC_PREVIEW_LEGACY_FRIENDLY_NAME "Zen Canvas Preview Handler"
!define ZC_PREVIEW_SHELLEX_CATEGORY "{8895B1C6-B41F-4C1C-A562-0D564250836F}"
!define ZC_PREVIEW_PREVHOST_APP_ID "{6D2B5079-2F0B-48DD-AB7F-97CEC514D30B}"
!define ZC_PREVIEW_THREADING_MODEL "Apartment"
!define ZC_PREVIEW_DLL_RELATIVE_PATH "native\fylura_windows_preview_handler.dll"
!define ZC_PREVIEW_EXTENSION_COUNT "16"
; A 20-attempt window with 250 ms between failed probes is bounded to less
; than five seconds and only accommodates normal prevhost release latency.
!define ZC_PREVIEW_QUIESCE_ATTEMPTS 20
!define ZC_PREVIEW_QUIESCE_DELAY_MS 250
!define ZC_PREVIEW_EXTENSION_01 ".md"
!define ZC_PREVIEW_EXTENSION_02 ".markdown"
!define ZC_PREVIEW_EXTENSION_03 ".rs"
!define ZC_PREVIEW_EXTENSION_04 ".py"
!define ZC_PREVIEW_EXTENSION_05 ".js"
!define ZC_PREVIEW_EXTENSION_06 ".jsx"
!define ZC_PREVIEW_EXTENSION_07 ".ts"
!define ZC_PREVIEW_EXTENSION_08 ".tsx"
!define ZC_PREVIEW_EXTENSION_09 ".java"
!define ZC_PREVIEW_EXTENSION_10 ".c"
!define ZC_PREVIEW_EXTENSION_11 ".h"
!define ZC_PREVIEW_EXTENSION_12 ".cpp"
!define ZC_PREVIEW_EXTENSION_13 ".hpp"
!define ZC_PREVIEW_EXTENSION_14 ".ps1"
!define ZC_PREVIEW_EXTENSION_15 ".sh"
!define ZC_PREVIEW_EXTENSION_16 ".sql"

!endif
