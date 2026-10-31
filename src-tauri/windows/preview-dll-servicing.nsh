; Exact Preview Handler DLL servicing for the generated Tauri NSIS resource.
; A mapped image may keep the canonical path unavailable even after its
; registration has been withdrawn.  This file moves only that exact current
; product DLL to a private same-volume retirement path, lets NSIS write the
; canonical resource, and retains the old bytes until post-integration has
; succeeded or exact recovery has been attempted.

!ifndef ZC_PREVIEW_DLL_SERVICING_NSH
!define ZC_PREVIEW_DLL_SERVICING_NSH

!include "LogicLib.nsh"

!define ZC_PREVIEW_DLL_RESOURCE_PATH_FORWARD "native/fylura_windows_preview_handler.dll"
!define ZC_PREVIEW_DLL_RESOURCE_PATH_BACKSLASH "native\fylura_windows_preview_handler.dll"
!define ZC_PREVIEW_DLL_RETIREMENT_DIRECTORY ".fylura-retired"
!define ZC_PREVIEW_DLL_ERROR_FILE_NOT_FOUND 2
!define ZC_PREVIEW_DLL_ERROR_SHARING_VIOLATION 32
!define ZC_PREVIEW_DLL_ERROR_LOCK_VIOLATION 33
!define ZC_PREVIEW_DLL_RETIREMENT_FLAGS_NONE 0
!define ZC_PREVIEW_DLL_RETIREMENT_FLAGS_DELAY_UNTIL_REBOOT 0x4

Var ZC_PREVIEW_RETIRED_PATH
Var ZC_PREVIEW_RETIRED_ACTIVE
Var ZC_PREVIEW_RETIREMENT_DIR
Var ZC_PREVIEW_MUTATION_READY
Var ZC_PREVIEW_MUTATION_ERROR

; Reset is deliberately explicit at each lifecycle start.  The generated
; resource macro owns the one exact Preview DLL invocation for that lifecycle.
Function ZCResetPreviewDllMutationState
  StrCpy $ZC_PREVIEW_RETIRED_PATH ""
  StrCpy $ZC_PREVIEW_RETIRED_ACTIVE 0
  StrCpy $ZC_PREVIEW_RETIREMENT_DIR ""
  StrCpy $ZC_PREVIEW_MUTATION_READY 0
  StrCpy $ZC_PREVIEW_MUTATION_ERROR 0
  StrCpy $ZC_PREVIEW_RELEASE_READY 0
FunctionEnd

Function un.ZCResetPreviewDllMutationState
  StrCpy $ZC_PREVIEW_RETIRED_PATH ""
  StrCpy $ZC_PREVIEW_RETIRED_ACTIVE 0
  StrCpy $ZC_PREVIEW_RETIREMENT_DIR ""
  StrCpy $ZC_PREVIEW_MUTATION_READY 0
  StrCpy $ZC_PREVIEW_MUTATION_ERROR 0
  StrCpy $ZC_PREVIEW_RELEASE_READY 0
FunctionEnd

; Prepare an exact canonical path for a generated File/Delete operation.
; Canonical absence and a successful direct write/delete-class probe are both
; ready without retirement.  Only sharing/lock errors enter the retirement
; path; every other Win32 error remains a fail-closed mutation error.
Function ZCPreparePreviewDllMutation
  StrCpy $ZC_PREVIEW_MUTATION_READY 0
  StrCpy $ZC_PREVIEW_MUTATION_ERROR 0
  StrCpy $ZC_PREVIEW_RETIRED_ACTIVE 0
  StrCpy $ZC_PREVIEW_RETIRED_PATH ""
  StrCpy $ZC_PREVIEW_RETIREMENT_DIR ""

  IfFileExists "${ZC_PREVIEW_INSTALLED_DLL}" zc_preview_prepare_install_probe 0
  StrCpy $ZC_PREVIEW_MUTATION_READY 1
  Return

zc_preview_prepare_install_probe:
  ; GENERIC_WRITE | DELETE, no sharing, OPEN_EXISTING, normal attributes.
  System::Call 'kernel32::CreateFileW(w "${ZC_PREVIEW_INSTALLED_DLL}", i 0x40000000|0x00010000, i 0, p 0, i 3, i 0x00000080, p 0) p.r1 ?e'
  Pop $2
  ${IntPtrCmp} $1 -1 zc_preview_prepare_install_error zc_preview_prepare_install_ready zc_preview_prepare_install_ready

zc_preview_prepare_install_ready:
  System::Call 'kernel32::CloseHandle(p $1) i.r3'
  ${If} $3 == 0
    StrCpy $ZC_PREVIEW_MUTATION_ERROR 6
    Return
  ${EndIf}
  StrCpy $ZC_PREVIEW_MUTATION_READY 1
  Return

zc_preview_prepare_install_error:
  StrCpy $ZC_PREVIEW_MUTATION_ERROR $2
  ${If} $2 == ${ZC_PREVIEW_DLL_ERROR_FILE_NOT_FOUND}
    ; The canonical file disappeared between the existence check and probe.
    StrCpy $ZC_PREVIEW_MUTATION_READY 1
    Return
  ${EndIf}
  ${If} $2 != ${ZC_PREVIEW_DLL_ERROR_SHARING_VIOLATION}
  ${AndIf} $2 != ${ZC_PREVIEW_DLL_ERROR_LOCK_VIOLATION}
    DetailPrint "Preview DLL mutation probe failed with Win32 error $2; mutation was not attempted."
    Return
  ${EndIf}

  ; The retirement directory is outside $INSTDIR but on its parent volume.
  ; It is intentionally narrow and is never recursively enumerated/deleted.
  ${GetParent} "$INSTDIR" $0
  ${If} $0 == ""
    StrCpy $ZC_PREVIEW_MUTATION_ERROR 5
    DetailPrint "Preview DLL retirement parent could not be derived; mutation was not attempted."
    Return
  ${EndIf}
  StrCpy $ZC_PREVIEW_RETIREMENT_DIR "$0\${ZC_PREVIEW_DLL_RETIREMENT_DIRECTORY}"
  IfFileExists "$ZC_PREVIEW_RETIREMENT_DIR\." zc_preview_prepare_install_retirement_dir_ready 0
  ClearErrors
  CreateDirectory "$ZC_PREVIEW_RETIREMENT_DIR"
  IfErrors zc_preview_prepare_install_retirement_dir_error

zc_preview_prepare_install_retirement_dir_ready:
  ; GetTempFileName creates the unique placeholder.  Delete only that exact
  ; placeholder before MoveFileExW; never overwrite an arbitrary destination.
  GetTempFileName $0 "$ZC_PREVIEW_RETIREMENT_DIR"
  ${If} $0 == ""
    StrCpy $ZC_PREVIEW_MUTATION_ERROR 8
    DetailPrint "Preview DLL retirement placeholder could not be created; mutation was not attempted."
    Return
  ${EndIf}
  StrCpy $ZC_PREVIEW_RETIRED_PATH $0
  System::Call 'kernel32::DeleteFileW(w "$ZC_PREVIEW_RETIRED_PATH") i.r1 ?e'
  Pop $2
  ${If} $1 == 0
    IfFileExists "$ZC_PREVIEW_RETIRED_PATH" 0 zc_preview_prepare_install_retirement_placeholder_gone
    StrCpy $ZC_PREVIEW_MUTATION_ERROR $2
    DetailPrint "Preview DLL retirement placeholder could not be removed (Win32 error $2); mutation was not attempted."
    Return
  ${EndIf}
zc_preview_prepare_install_retirement_placeholder_gone:
  System::Call 'kernel32::MoveFileExW(w "${ZC_PREVIEW_INSTALLED_DLL}", w "$ZC_PREVIEW_RETIRED_PATH", i ${ZC_PREVIEW_DLL_RETIREMENT_FLAGS_NONE}) i.r1 ?e'
  Pop $2
  ${If} $1 == 0
    StrCpy $ZC_PREVIEW_MUTATION_ERROR $2
    DetailPrint "Preview DLL retirement move failed with Win32 error $2; canonical artifact was preserved."
    Return
  ${EndIf}
  StrCpy $ZC_PREVIEW_RETIRED_ACTIVE 1
  IfFileExists "${ZC_PREVIEW_INSTALLED_DLL}" zc_preview_prepare_install_retirement_verify_failed 0
  IfFileExists "$ZC_PREVIEW_RETIRED_PATH" 0 zc_preview_prepare_install_retirement_verify_failed
  StrCpy $ZC_PREVIEW_MUTATION_READY 1
  DetailPrint "Preview DLL was retired to the exact same-volume recovery path before generated replacement."
  Return

zc_preview_prepare_install_retirement_verify_failed:
  StrCpy $ZC_PREVIEW_MUTATION_ERROR 13
  DetailPrint "Preview DLL retirement post-state was not verifiable; generated replacement was not attempted."
  Return

zc_preview_prepare_install_retirement_dir_error:
  StrCpy $ZC_PREVIEW_MUTATION_ERROR 5
  DetailPrint "Preview DLL retirement directory could not be created; mutation was not attempted."
FunctionEnd

Function un.ZCPreparePreviewDllMutation
  StrCpy $ZC_PREVIEW_MUTATION_READY 0
  StrCpy $ZC_PREVIEW_MUTATION_ERROR 0
  StrCpy $ZC_PREVIEW_RETIRED_ACTIVE 0
  StrCpy $ZC_PREVIEW_RETIRED_PATH ""
  StrCpy $ZC_PREVIEW_RETIREMENT_DIR ""

  IfFileExists "${ZC_PREVIEW_INSTALLED_DLL}" un_zc_preview_prepare_probe 0
  StrCpy $ZC_PREVIEW_MUTATION_READY 1
  Return

un_zc_preview_prepare_probe:
  System::Call 'kernel32::CreateFileW(w "${ZC_PREVIEW_INSTALLED_DLL}", i 0x40000000|0x00010000, i 0, p 0, i 3, i 0x00000080, p 0) p.r1 ?e'
  Pop $2
  ${IntPtrCmp} $1 -1 un_zc_preview_prepare_error un_zc_preview_prepare_ready un_zc_preview_prepare_ready

un_zc_preview_prepare_ready:
  System::Call 'kernel32::CloseHandle(p $1) i.r3'
  ${If} $3 == 0
    StrCpy $ZC_PREVIEW_MUTATION_ERROR 6
    Return
  ${EndIf}
  StrCpy $ZC_PREVIEW_MUTATION_READY 1
  Return

un_zc_preview_prepare_error:
  StrCpy $ZC_PREVIEW_MUTATION_ERROR $2
  ${If} $2 == ${ZC_PREVIEW_DLL_ERROR_FILE_NOT_FOUND}
    StrCpy $ZC_PREVIEW_MUTATION_READY 1
    Return
  ${EndIf}
  ${If} $2 != ${ZC_PREVIEW_DLL_ERROR_SHARING_VIOLATION}
  ${AndIf} $2 != ${ZC_PREVIEW_DLL_ERROR_LOCK_VIOLATION}
    DetailPrint "Preview DLL uninstall probe failed with Win32 error $2; mutation was not attempted."
    Return
  ${EndIf}

  ${GetParent} "$INSTDIR" $0
  ${If} $0 == ""
    StrCpy $ZC_PREVIEW_MUTATION_ERROR 5
    DetailPrint "Preview DLL uninstall retirement parent could not be derived; mutation was not attempted."
    Return
  ${EndIf}
  StrCpy $ZC_PREVIEW_RETIREMENT_DIR "$0\${ZC_PREVIEW_DLL_RETIREMENT_DIRECTORY}"
  IfFileExists "$ZC_PREVIEW_RETIREMENT_DIR\." un_zc_preview_prepare_retirement_dir_ready 0
  ClearErrors
  CreateDirectory "$ZC_PREVIEW_RETIREMENT_DIR"
  IfErrors un_zc_preview_prepare_retirement_dir_error

un_zc_preview_prepare_retirement_dir_ready:
  GetTempFileName $0 "$ZC_PREVIEW_RETIREMENT_DIR"
  ${If} $0 == ""
    StrCpy $ZC_PREVIEW_MUTATION_ERROR 8
    DetailPrint "Preview DLL uninstall retirement placeholder could not be created; mutation was not attempted."
    Return
  ${EndIf}
  StrCpy $ZC_PREVIEW_RETIRED_PATH $0
  System::Call 'kernel32::DeleteFileW(w "$ZC_PREVIEW_RETIRED_PATH") i.r1 ?e'
  Pop $2
  ${If} $1 == 0
    IfFileExists "$ZC_PREVIEW_RETIRED_PATH" 0 un_zc_preview_prepare_placeholder_gone
    StrCpy $ZC_PREVIEW_MUTATION_ERROR $2
    DetailPrint "Preview DLL uninstall retirement placeholder could not be removed (Win32 error $2); mutation was not attempted."
    Return
  ${EndIf}
un_zc_preview_prepare_placeholder_gone:
  System::Call 'kernel32::MoveFileExW(w "${ZC_PREVIEW_INSTALLED_DLL}", w "$ZC_PREVIEW_RETIRED_PATH", i ${ZC_PREVIEW_DLL_RETIREMENT_FLAGS_NONE}) i.r1 ?e'
  Pop $2
  ${If} $1 == 0
    StrCpy $ZC_PREVIEW_MUTATION_ERROR $2
    DetailPrint "Preview DLL uninstall retirement move failed with Win32 error $2; canonical artifact was preserved."
    Return
  ${EndIf}
  StrCpy $ZC_PREVIEW_RETIRED_ACTIVE 1
  IfFileExists "${ZC_PREVIEW_INSTALLED_DLL}" un_zc_preview_prepare_retirement_verify_failed 0
  IfFileExists "$ZC_PREVIEW_RETIRED_PATH" 0 un_zc_preview_prepare_retirement_verify_failed
  StrCpy $ZC_PREVIEW_MUTATION_READY 1
  DetailPrint "Preview DLL was retired to the exact same-volume recovery path before generated uninstall deletion."
  Return

un_zc_preview_prepare_retirement_verify_failed:
  StrCpy $ZC_PREVIEW_MUTATION_ERROR 13
  DetailPrint "Preview DLL uninstall retirement post-state was not verifiable; generated deletion was not attempted."
  Return

un_zc_preview_prepare_retirement_dir_error:
  StrCpy $ZC_PREVIEW_MUTATION_ERROR 5
  DetailPrint "Preview DLL uninstall retirement directory could not be created; mutation was not attempted."
FunctionEnd

; Restore only the exact retired path.  If generated File/Delete returned an
; error after retirement, an existing canonical path is the current attempt's
; exact output and may be removed before the old bytes are moved back.
Function ZCRecoverPreviewDllMutation
  ${If} $ZC_PREVIEW_RETIRED_ACTIVE != 1
    Return
  ${EndIf}
  IfFileExists "$ZC_PREVIEW_RETIRED_PATH" 0 zc_preview_recover_missing_retired
  IfFileExists "${ZC_PREVIEW_INSTALLED_DLL}" 0 zc_preview_recover_move
  System::Call 'kernel32::DeleteFileW(w "${ZC_PREVIEW_INSTALLED_DLL}") i.r1 ?e'
  Pop $2
  ${If} $1 == 0
    StrCpy $ZC_PREVIEW_MUTATION_ERROR $2
    DetailPrint "Preview DLL recovery could not remove the exact current canonical output (Win32 error $2)."
    Return
  ${EndIf}
  IfFileExists "${ZC_PREVIEW_INSTALLED_DLL}" 0 zc_preview_recover_move
  Return

zc_preview_recover_move:
  System::Call 'kernel32::MoveFileExW(w "$ZC_PREVIEW_RETIRED_PATH", w "${ZC_PREVIEW_INSTALLED_DLL}", i ${ZC_PREVIEW_DLL_RETIREMENT_FLAGS_NONE}) i.r1 ?e'
  Pop $2
  ${If} $1 == 0
    StrCpy $ZC_PREVIEW_MUTATION_ERROR $2
    DetailPrint "Preview DLL recovery move failed with Win32 error $2; retired bytes were preserved."
    Return
  ${EndIf}
  IfFileExists "${ZC_PREVIEW_INSTALLED_DLL}" 0 zc_preview_recover_verify_failed
  IfFileExists "$ZC_PREVIEW_RETIRED_PATH" zc_preview_recover_verify_failed 0
  StrCpy $ZC_PREVIEW_RETIRED_ACTIVE 0
  StrCpy $ZC_PREVIEW_RETIRED_PATH ""
  ClearErrors
  RMDir "$ZC_PREVIEW_RETIREMENT_DIR"
  Return

zc_preview_recover_missing_retired:
  StrCpy $ZC_PREVIEW_MUTATION_ERROR ${ZC_PREVIEW_DLL_ERROR_FILE_NOT_FOUND}
  DetailPrint "Preview DLL recovery could not find the exact retired bytes; registration remains under failure handling."
  Return

zc_preview_recover_verify_failed:
  StrCpy $ZC_PREVIEW_MUTATION_ERROR 13
  DetailPrint "Preview DLL recovery post-state was not verifiable; retired bytes were preserved."
FunctionEnd

Function un.ZCRecoverPreviewDllMutation
  ${If} $ZC_PREVIEW_RETIRED_ACTIVE != 1
    Return
  ${EndIf}
  IfFileExists "$ZC_PREVIEW_RETIRED_PATH" 0 un_zc_preview_recover_missing_retired
  IfFileExists "${ZC_PREVIEW_INSTALLED_DLL}" 0 un_zc_preview_recover_move
  System::Call 'kernel32::DeleteFileW(w "${ZC_PREVIEW_INSTALLED_DLL}") i.r1 ?e'
  Pop $2
  ${If} $1 == 0
    StrCpy $ZC_PREVIEW_MUTATION_ERROR $2
    DetailPrint "Preview DLL uninstall recovery could not remove the exact current canonical output (Win32 error $2)."
    Return
  ${EndIf}
  IfFileExists "${ZC_PREVIEW_INSTALLED_DLL}" 0 un_zc_preview_recover_move
  Return

un_zc_preview_recover_move:
  System::Call 'kernel32::MoveFileExW(w "$ZC_PREVIEW_RETIRED_PATH", w "${ZC_PREVIEW_INSTALLED_DLL}", i ${ZC_PREVIEW_DLL_RETIREMENT_FLAGS_NONE}) i.r1 ?e'
  Pop $2
  ${If} $1 == 0
    StrCpy $ZC_PREVIEW_MUTATION_ERROR $2
    DetailPrint "Preview DLL uninstall recovery move failed with Win32 error $2; retired bytes were preserved."
    Return
  ${EndIf}
  IfFileExists "${ZC_PREVIEW_INSTALLED_DLL}" 0 un_zc_preview_recover_verify_failed
  IfFileExists "$ZC_PREVIEW_RETIRED_PATH" un_zc_preview_recover_verify_failed 0
  StrCpy $ZC_PREVIEW_RETIRED_ACTIVE 0
  StrCpy $ZC_PREVIEW_RETIRED_PATH ""
  ClearErrors
  RMDir "$ZC_PREVIEW_RETIREMENT_DIR"
  Return

un_zc_preview_recover_missing_retired:
  StrCpy $ZC_PREVIEW_MUTATION_ERROR ${ZC_PREVIEW_DLL_ERROR_FILE_NOT_FOUND}
  DetailPrint "Preview DLL uninstall recovery could not find the exact retired bytes; registration remains under failure handling."
  Return

un_zc_preview_recover_verify_failed:
  StrCpy $ZC_PREVIEW_MUTATION_ERROR 13
  DetailPrint "Preview DLL uninstall recovery post-state was not verifiable; retired bytes were preserved."
FunctionEnd

; Retirement cleanup is best effort.  A sharing/lock failure is scheduled for
; deletion at reboot and never turns a successful product operation into a
; failure.  The process does not request or require a reboot.
Function ZCFinalizePreviewDllMutation
  ${If} $ZC_PREVIEW_RETIRED_ACTIVE != 1
    Return
  ${EndIf}
  IfFileExists "$ZC_PREVIEW_RETIRED_PATH" 0 zc_preview_finalize_done
  System::Call 'kernel32::DeleteFileW(w "$ZC_PREVIEW_RETIRED_PATH") i.r1 ?e'
  Pop $2
  ${If} $1 != 0
    Goto zc_preview_finalize_done
  ${EndIf}
  ${If} $2 == ${ZC_PREVIEW_DLL_ERROR_SHARING_VIOLATION}
  ${OrIf} $2 == ${ZC_PREVIEW_DLL_ERROR_LOCK_VIOLATION}
    System::Call 'kernel32::MoveFileExW(w "$ZC_PREVIEW_RETIRED_PATH", p 0, i ${ZC_PREVIEW_DLL_RETIREMENT_FLAGS_DELAY_UNTIL_REBOOT}) i.r1 ?e'
    Pop $3
    ${If} $1 != 0
      DetailPrint "Preview DLL retirement cleanup is scheduled for the next reboot; no reboot is required for this result."
    ${Else}
      DetailPrint "Preview DLL retirement cleanup remained as best-effort residue (Win32 error $3)."
    ${EndIf}
  ${Else}
    DetailPrint "Preview DLL retirement cleanup remained as best-effort residue (Win32 error $2)."
  ${EndIf}
zc_preview_finalize_done:
  ClearErrors
  RMDir "$ZC_PREVIEW_RETIREMENT_DIR"
  StrCpy $ZC_PREVIEW_RETIRED_ACTIVE 0
FunctionEnd

Function un.ZCFinalizePreviewDllMutation
  ${If} $ZC_PREVIEW_RETIRED_ACTIVE != 1
    Return
  ${EndIf}
  IfFileExists "$ZC_PREVIEW_RETIRED_PATH" 0 un_zc_preview_finalize_done
  System::Call 'kernel32::DeleteFileW(w "$ZC_PREVIEW_RETIRED_PATH") i.r1 ?e'
  Pop $2
  ${If} $1 != 0
    Goto un_zc_preview_finalize_done
  ${EndIf}
  ${If} $2 == ${ZC_PREVIEW_DLL_ERROR_SHARING_VIOLATION}
  ${OrIf} $2 == ${ZC_PREVIEW_DLL_ERROR_LOCK_VIOLATION}
    System::Call 'kernel32::MoveFileExW(w "$ZC_PREVIEW_RETIRED_PATH", p 0, i ${ZC_PREVIEW_DLL_RETIREMENT_FLAGS_DELAY_UNTIL_REBOOT}) i.r1 ?e'
    Pop $3
    ${If} $1 != 0
      DetailPrint "Preview DLL uninstall retirement cleanup is scheduled for the next reboot; no reboot is required for this result."
    ${Else}
      DetailPrint "Preview DLL uninstall retirement cleanup remained as best-effort residue (Win32 error $3)."
    ${EndIf}
  ${Else}
    DetailPrint "Preview DLL uninstall retirement cleanup remained as best-effort residue (Win32 error $2)."
  ${EndIf}
un_zc_preview_finalize_done:
  ClearErrors
  RMDir "$ZC_PREVIEW_RETIREMENT_DIR"
  StrCpy $ZC_PREVIEW_RETIRED_ACTIVE 0
FunctionEnd

; The generated template invokes these macros for every resource.  Only the
; exact canonical Preview resource takes the servicing path; all other
; resources retain the normal ClearErrors/File/IfErrors contract.
!macro ZC_INSTALL_PREVIEW_RESOURCE SOURCE
  Call ZCPreparePreviewDllMutation
  ${If} $ZC_PREVIEW_MUTATION_READY != 1
    Call ZCRecoverPreviewDllMutation
    StrCpy $ZC_POSTINSTALL_FAILURE_REASON "The Fylura Preview Handler DLL could not be prepared for in-use servicing (Win32 error $ZC_PREVIEW_MUTATION_ERROR)."
    Goto zc_install_partial_failure
  ${EndIf}
  ClearErrors
  File /a "/oname=${ZC_PREVIEW_DLL_RESOURCE_PATH_BACKSLASH}" "${SOURCE}"
  ${If} ${Errors}
    Call ZCRecoverPreviewDllMutation
    StrCpy $ZC_POSTINSTALL_FAILURE_REASON "The Fylura Preview Handler DLL could not be replaced; exact retirement recovery was attempted (Win32 error $ZC_PREVIEW_MUTATION_ERROR)."
    Goto zc_install_partial_failure
  ${EndIf}
!macroend

!macro ZC_INSTALL_RESOURCE DESTINATION SOURCE
  !if "${DESTINATION}" == "${ZC_PREVIEW_DLL_RESOURCE_PATH_FORWARD}"
    !insertmacro ZC_INSTALL_PREVIEW_RESOURCE "${SOURCE}"
  !else if "${DESTINATION}" == "${ZC_PREVIEW_DLL_RESOURCE_PATH_BACKSLASH}"
    !insertmacro ZC_INSTALL_PREVIEW_RESOURCE "${SOURCE}"
  !else
    ClearErrors
    File /a "/oname=${DESTINATION}" "${SOURCE}"
    IfErrors zc_install_partial_failure
  !endif
!macroend

!macro ZC_UNINSTALL_PREVIEW_RESOURCE
  Call un.ZCPreparePreviewDllMutation
  ${If} $ZC_PREVIEW_MUTATION_READY != 1
    Call un.ZCRecoverPreviewDllMutation
    Goto zc_uninstall_partial_failure
  ${EndIf}
  ${If} $ZC_PREVIEW_RETIRED_ACTIVE == 1
    Goto un_zc_preview_resource_done
  ${EndIf}
  ClearErrors
  Delete "${ZC_PREVIEW_INSTALLED_DLL}"
  IfErrors zc_uninstall_partial_failure
un_zc_preview_resource_done:
!macroend

!macro ZC_UNINSTALL_RESOURCE DESTINATION
  !if "${DESTINATION}" == "${ZC_PREVIEW_DLL_RESOURCE_PATH_FORWARD}"
    !insertmacro ZC_UNINSTALL_PREVIEW_RESOURCE
  !else if "${DESTINATION}" == "${ZC_PREVIEW_DLL_RESOURCE_PATH_BACKSLASH}"
    !insertmacro ZC_UNINSTALL_PREVIEW_RESOURCE
  !else
    ClearErrors
    Delete "$INSTDIR\${DESTINATION}"
    IfErrors zc_uninstall_partial_failure
  !endif
!macroend

!endif
