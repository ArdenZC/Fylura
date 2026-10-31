; W4-04 synchronous package lifecycle owner.
; Included only by installer-lifecycle-wrapper.nsh in the package-only custom
; Tauri 2.11.2 NSIS template. Correctness is owned synchronously here; the
; legacy .onInstFailed / un.onUninstFailed callbacks are compatibility
; dispatch shims only.

; Stage 0 is inactive, stage 5 is successful completion, and stages 2-4 are
; monotonic after generated product mutation begins. Failure owners never
; rewrite a later stage to an earlier one.
!define ZC_LIFECYCLE_STAGE_INACTIVE 0
!define ZC_LIFECYCLE_STAGE_REVERSIBLE_PREPARATION 1
!define ZC_LIFECYCLE_STAGE_FILE_MUTATION 2
!define ZC_LIFECYCLE_STAGE_GENERATED_MUTATION 3
!define ZC_LIFECYCLE_STAGE_POST_GENERATED_INTEGRATION 4
!define ZC_LIFECYCLE_STAGE_COMPLETE 5

; ---------------------------------------------------------------------------
; Desktop app gate. The exact-owned Global Index service must already be
; STOPPED before these name-only process operations are permitted.
; ---------------------------------------------------------------------------

Function ZCResolveMainAppGate
  StrCpy $ZC_LIFECYCLE_GATE_OK 0
  nsis_tauri_utils::StrReplace "$(appRunningOkKill)" "{{product_name}}" "${PRODUCTNAME}"
  Pop $R2
  nsis_tauri_utils::StrReplace "$(failedToKillApp)" "{{product_name}}" "${PRODUCTNAME}"
  Pop $R3

  nsis_tauri_utils::FindProcess "${MAINBINARYNAME}.exe"
  Pop $R0
  ${If} $R0 != 0
    StrCpy $ZC_LIFECYCLE_GATE_OK 1
    Return
  ${EndIf}

  IfSilent zc_install_gate_kill zc_install_gate_prompt
zc_install_gate_prompt:
  MessageBox MB_OKCANCEL "$R2" IDOK zc_install_gate_kill IDCANCEL zc_install_gate_cancel
zc_install_gate_kill:
  nsis_tauri_utils::KillProcess "${MAINBINARYNAME}.exe"
  Pop $R0
  Sleep 500
  nsis_tauri_utils::FindProcess "${MAINBINARYNAME}.exe"
  Pop $R0
  ${If} $R0 != 0
    StrCpy $ZC_LIFECYCLE_GATE_OK 1
    Return
  ${EndIf}
  MessageBox MB_ICONSTOP|MB_OK "$R3" /SD IDOK
zc_install_gate_cancel:
FunctionEnd

Function un.ZCResolveMainAppGate
  StrCpy $ZC_LIFECYCLE_GATE_OK 0
  nsis_tauri_utils::StrReplace "$(appRunningOkKill)" "{{product_name}}" "${PRODUCTNAME}"
  Pop $R2
  nsis_tauri_utils::StrReplace "$(failedToKillApp)" "{{product_name}}" "${PRODUCTNAME}"
  Pop $R3

  nsis_tauri_utils::FindProcess "${MAINBINARYNAME}.exe"
  Pop $R0
  ${If} $R0 != 0
    StrCpy $ZC_LIFECYCLE_GATE_OK 1
    Return
  ${EndIf}

  IfSilent zc_uninstall_gate_kill zc_uninstall_gate_prompt
zc_uninstall_gate_prompt:
  MessageBox MB_OKCANCEL "$R2" IDOK zc_uninstall_gate_kill IDCANCEL zc_uninstall_gate_cancel
zc_uninstall_gate_kill:
  nsis_tauri_utils::KillProcess "${MAINBINARYNAME}.exe"
  Pop $R0
  Sleep 500
  nsis_tauri_utils::FindProcess "${MAINBINARYNAME}.exe"
  Pop $R0
  ${If} $R0 != 0
    StrCpy $ZC_LIFECYCLE_GATE_OK 1
    Return
  ${EndIf}
  MessageBox MB_ICONSTOP|MB_OK "$R3" /SD IDOK
zc_uninstall_gate_cancel:
FunctionEnd

; ---------------------------------------------------------------------------
; Exact-owned service stop. Runtime state is consulted only after ImagePath
; ownership has been re-read. These functions never Abort so the caller owns
; synchronous recovery.
; ---------------------------------------------------------------------------

Function ZCStopCapturedServiceForLifecycle
  StrCpy $ZC_LIFECYCLE_STOP_OK 0
  Call ReadZenCanvasIndexServiceOwnership
  ${If} $ZC_PREEXISTING_SERVICE == 0
    ${If} $ZC_INDEX_SERVICE_OWNERSHIP == 0
      StrCpy $ZC_LIFECYCLE_STOP_OK 1
    ${EndIf}
    Return
  ${EndIf}
  ${If} $ZC_INDEX_SERVICE_OWNERSHIP != 1
    Return
  ${EndIf}

  Call ReadZenCanvasIndexServiceRuntimeState
  ${If} $ZC_INDEX_SERVICE_RUNTIME_STATE == 2
    StrCpy $ZC_LIFECYCLE_STOP_OK 1
    Return
  ${EndIf}
  ${If} $ZC_INDEX_SERVICE_RUNTIME_STATE == 3
    Call WaitForZenCanvasIndexServiceStopped
    ${If} $ZC_INDEX_SERVICE_STOPPED_READY == 1
      StrCpy $ZC_LIFECYCLE_STOP_OK 1
    ${EndIf}
    Return
  ${EndIf}
  ${If} $ZC_INDEX_SERVICE_RUNTIME_STATE != 1
    Return
  ${EndIf}

  Call ReadZenCanvasIndexServiceOwnership
  ${If} $ZC_INDEX_SERVICE_OWNERSHIP != 1
    Return
  ${EndIf}
  nsExec::ExecToStack '"$SYSDIR\sc.exe" stop "${ZC_INDEX_SERVICE_NAME}"'
  Pop $0
  Pop $1
  ${If} $0 != 0
    Call ReadZenCanvasIndexServiceOwnership
    ${If} $ZC_INDEX_SERVICE_OWNERSHIP != 1
      Return
    ${EndIf}
    Call ReadZenCanvasIndexServiceRuntimeState
    ${If} $ZC_INDEX_SERVICE_RUNTIME_STATE == 2
      StrCpy $ZC_LIFECYCLE_STOP_OK 1
    ${EndIf}
    Return
  ${EndIf}
  Call WaitForZenCanvasIndexServiceStopped
  ${If} $ZC_INDEX_SERVICE_STOPPED_READY == 1
    StrCpy $ZC_LIFECYCLE_STOP_OK 1
  ${EndIf}
FunctionEnd

Function un.ZCStopCapturedServiceForLifecycle
  StrCpy $ZC_LIFECYCLE_STOP_OK 0
  Call un.ReadZenCanvasIndexServiceOwnership
  ${If} $ZC_UNINSTALL_ORIGINAL_SERVICE == 0
    ${If} $ZC_INDEX_SERVICE_OWNERSHIP == 0
      StrCpy $ZC_LIFECYCLE_STOP_OK 1
    ${EndIf}
    Return
  ${EndIf}
  ${If} $ZC_INDEX_SERVICE_OWNERSHIP != 1
    Return
  ${EndIf}

  Call un.ReadZenCanvasIndexServiceRuntimeState
  ${If} $ZC_INDEX_SERVICE_RUNTIME_STATE == 2
    StrCpy $ZC_LIFECYCLE_STOP_OK 1
    Return
  ${EndIf}
  ${If} $ZC_INDEX_SERVICE_RUNTIME_STATE == 3
    Call un.WaitForZenCanvasIndexServiceStopped
    ${If} $ZC_INDEX_SERVICE_STOPPED_READY == 1
      StrCpy $ZC_LIFECYCLE_STOP_OK 1
    ${EndIf}
    Return
  ${EndIf}
  ${If} $ZC_INDEX_SERVICE_RUNTIME_STATE != 1
    Return
  ${EndIf}

  Call un.ReadZenCanvasIndexServiceOwnership
  ${If} $ZC_INDEX_SERVICE_OWNERSHIP != 1
    Return
  ${EndIf}
  nsExec::ExecToStack '"$SYSDIR\sc.exe" stop "${ZC_INDEX_SERVICE_NAME}"'
  Pop $0
  Pop $1
  ${If} $0 != 0
    Call un.ReadZenCanvasIndexServiceOwnership
    ${If} $ZC_INDEX_SERVICE_OWNERSHIP != 1
      Return
    ${EndIf}
    Call un.ReadZenCanvasIndexServiceRuntimeState
    ${If} $ZC_INDEX_SERVICE_RUNTIME_STATE == 2
      StrCpy $ZC_LIFECYCLE_STOP_OK 1
    ${EndIf}
    Return
  ${EndIf}
  Call un.WaitForZenCanvasIndexServiceStopped
  ${If} $ZC_INDEX_SERVICE_STOPPED_READY == 1
    StrCpy $ZC_LIFECYCLE_STOP_OK 1
  ${EndIf}
FunctionEnd

; ---------------------------------------------------------------------------
; Non-aborting Preview withdrawal. Core values are strict because the complete
; core was validated before any service mutation. Association slots remain
; conflict-safe: only the exact Zen production CLSID is withdrawn.
; ---------------------------------------------------------------------------

!macro ZC_LIFECYCLE_WITHDRAW_ASSOC PATH ROLLBACK_FUNCTION NOTIFY_FUNCTION
  !insertmacro ZC_REG_QUERY_STRING_STATE ${ZC_REG_ROOT_HKLM} "${PATH}" "" "${ZC_PREVIEW_PRODUCTION_CLSID}" ${ZC_REG_STRING_SZ_ONLY}
  ${If} $ZC_REG_VALUE_STATE == ${ZC_REG_VALUE_EXACT}
    !insertmacro ZC_RECORD_REG_VALUE "${PATH}" "" "${ZC_PREVIEW_PRODUCTION_CLSID}"
    ${If} $ZC_PREVIEW_TXN_CAPTURE_OK != 1
      Call ${ROLLBACK_FUNCTION}
      Call ${NOTIFY_FUNCTION}
      StrCpy $ZC_LIFECYCLE_PREVIEW_OK 0
      Return
    ${EndIf}
    ClearErrors
    DeleteRegValue HKLM "${PATH}" ""
    !insertmacro ZC_REG_QUERY_STRING_STATE ${ZC_REG_ROOT_HKLM} "${PATH}" "" "${ZC_PREVIEW_PRODUCTION_CLSID}" ${ZC_REG_STRING_SZ_ONLY}
    ${If} ${Errors}
    ${OrIf} $ZC_REG_VALUE_STATE != ${ZC_REG_VALUE_ABSENT}
      Call ${ROLLBACK_FUNCTION}
      Call ${NOTIFY_FUNCTION}
      StrCpy $ZC_LIFECYCLE_PREVIEW_OK 0
      Return
    ${EndIf}
  ${ElseIf} $ZC_REG_VALUE_STATE == ${ZC_REG_VALUE_UNKNOWN}
    Call ${ROLLBACK_FUNCTION}
    Call ${NOTIFY_FUNCTION}
    StrCpy $ZC_LIFECYCLE_PREVIEW_OK 0
    Return
  ${EndIf}
!macroend

!macro ZC_LIFECYCLE_WITHDRAW_CORE PATH NAME EXPECTED ROLLBACK_FUNCTION NOTIFY_FUNCTION
  ${If} $ZC_PREVIEW_CORE_PRESENT == 1
    !insertmacro ZC_REG_QUERY_STRING_STATE ${ZC_REG_ROOT_HKLM} "${PATH}" "${NAME}" "${EXPECTED}" ${ZC_REG_STRING_SZ_ONLY}
    ${If} $ZC_REG_VALUE_STATE != ${ZC_REG_VALUE_EXACT}
      Call ${ROLLBACK_FUNCTION}
      Call ${NOTIFY_FUNCTION}
      StrCpy $ZC_LIFECYCLE_PREVIEW_OK 0
      Return
    ${EndIf}
    !insertmacro ZC_RECORD_REG_VALUE "${PATH}" "${NAME}" "${EXPECTED}"
    ${If} $ZC_PREVIEW_TXN_CAPTURE_OK != 1
      Call ${ROLLBACK_FUNCTION}
      Call ${NOTIFY_FUNCTION}
      StrCpy $ZC_LIFECYCLE_PREVIEW_OK 0
      Return
    ${EndIf}
    ClearErrors
    DeleteRegValue HKLM "${PATH}" "${NAME}"
    !insertmacro ZC_REG_QUERY_STRING_STATE ${ZC_REG_ROOT_HKLM} "${PATH}" "${NAME}" "${EXPECTED}" ${ZC_REG_STRING_SZ_ONLY}
    ${If} ${Errors}
    ${OrIf} $ZC_REG_VALUE_STATE != ${ZC_REG_VALUE_ABSENT}
      Call ${ROLLBACK_FUNCTION}
      Call ${NOTIFY_FUNCTION}
      StrCpy $ZC_LIFECYCLE_PREVIEW_OK 0
      Return
    ${EndIf}
  ${EndIf}
!macroend

!macro ZC_LIFECYCLE_REMOVE_STALE_BODY ROLLBACK_FUNCTION NOTIFY_FUNCTION
  SetRegView 64
  StrCpy $0 0
zc_lifecycle_stale_loop:
  !insertmacro ZC_REG_ENUM_KEY_STATE ${ZC_REG_ROOT_HKLM} "${ZC_PREVIEW_ASSOCIATION_ROOT}" $0
  ${If} $ZC_REG_ENUM_STATE == ${ZC_REG_ENUM_END}
    Return
  ${ElseIf} $ZC_REG_ENUM_STATE == ${ZC_REG_ENUM_UNKNOWN}
    Call ${ROLLBACK_FUNCTION}
    Call ${NOTIFY_FUNCTION}
    StrCpy $ZC_LIFECYCLE_PREVIEW_OK 0
    Return
  ${EndIf}
  StrCpy $1 $ZC_REG_ENUM_NAME
  StrCpy $2 $1 1
  ${If} $2 == "."
    !insertmacro ZC_REG_QUERY_STRING_STATE ${ZC_REG_ROOT_HKLM} "${ZC_PREVIEW_ASSOCIATION_ROOT}\$1\shellex\${ZC_PREVIEW_SHELLEX_CATEGORY}" "" "${ZC_PREVIEW_PRODUCTION_CLSID}" ${ZC_REG_STRING_SZ_ONLY}
    ${If} $ZC_REG_VALUE_STATE == ${ZC_REG_VALUE_UNKNOWN}
      Call ${ROLLBACK_FUNCTION}
      Call ${NOTIFY_FUNCTION}
      StrCpy $ZC_LIFECYCLE_PREVIEW_OK 0
      Return
    ${ElseIf} $ZC_REG_VALUE_STATE == ${ZC_REG_VALUE_EXACT}
      !insertmacro ZC_RECORD_REG_VALUE "${ZC_PREVIEW_ASSOCIATION_ROOT}\$1\shellex\${ZC_PREVIEW_SHELLEX_CATEGORY}" "" "${ZC_PREVIEW_PRODUCTION_CLSID}"
      ${If} $ZC_PREVIEW_TXN_CAPTURE_OK != 1
        Call ${ROLLBACK_FUNCTION}
        Call ${NOTIFY_FUNCTION}
        StrCpy $ZC_LIFECYCLE_PREVIEW_OK 0
        Return
      ${EndIf}
      ClearErrors
      DeleteRegValue HKLM "${ZC_PREVIEW_ASSOCIATION_ROOT}\$1\shellex\${ZC_PREVIEW_SHELLEX_CATEGORY}" ""
      !insertmacro ZC_REG_QUERY_STRING_STATE ${ZC_REG_ROOT_HKLM} "${ZC_PREVIEW_ASSOCIATION_ROOT}\$1\shellex\${ZC_PREVIEW_SHELLEX_CATEGORY}" "" "${ZC_PREVIEW_PRODUCTION_CLSID}" ${ZC_REG_STRING_SZ_ONLY}
      ${If} ${Errors}
      ${OrIf} $ZC_REG_VALUE_STATE != ${ZC_REG_VALUE_ABSENT}
        Call ${ROLLBACK_FUNCTION}
        Call ${NOTIFY_FUNCTION}
        StrCpy $ZC_LIFECYCLE_PREVIEW_OK 0
        Return
      ${EndIf}
      Goto zc_lifecycle_stale_loop
    ${EndIf}
  ${EndIf}
  IntOp $0 $0 + 1
  Goto zc_lifecycle_stale_loop
!macroend

Function ZCLifecycleRemoveStalePreviewAssociations
  !insertmacro ZC_LIFECYCLE_REMOVE_STALE_BODY RollbackZenCanvasPreviewRegistration NotifyZenCanvasPreviewAssociationChanged
FunctionEnd

Function un.ZCLifecycleRemoveStalePreviewAssociations
  !insertmacro ZC_LIFECYCLE_REMOVE_STALE_BODY un.RollbackZenCanvasPreviewRegistration un.NotifyZenCanvasPreviewAssociationChanged
FunctionEnd

!macro ZC_LIFECYCLE_WITHDRAW_PREVIEW_BODY ROLLBACK_FUNCTION STALE_FUNCTION NOTIFY_FUNCTION
  SetRegView 64
  StrCpy $ZC_LIFECYCLE_PREVIEW_OK 1
  StrCpy $ZC_PREVIEW_TXN_COUNT 0
  StrCpy $ZC_PREVIEW_ROLLBACK_CLEAN 1
  StrCpy $ZC_PREVIEW_QUIESCE_ACTIVE 1

  !insertmacro ZC_LIFECYCLE_WITHDRAW_ASSOC "${ZC_PREVIEW_ASSOCIATION_ROOT}\${ZC_PREVIEW_EXTENSION_01}\shellex\${ZC_PREVIEW_SHELLEX_CATEGORY}" ${ROLLBACK_FUNCTION} ${NOTIFY_FUNCTION}
  !insertmacro ZC_LIFECYCLE_WITHDRAW_ASSOC "${ZC_PREVIEW_ASSOCIATION_ROOT}\${ZC_PREVIEW_EXTENSION_02}\shellex\${ZC_PREVIEW_SHELLEX_CATEGORY}" ${ROLLBACK_FUNCTION} ${NOTIFY_FUNCTION}
  !insertmacro ZC_LIFECYCLE_WITHDRAW_ASSOC "${ZC_PREVIEW_ASSOCIATION_ROOT}\${ZC_PREVIEW_EXTENSION_03}\shellex\${ZC_PREVIEW_SHELLEX_CATEGORY}" ${ROLLBACK_FUNCTION} ${NOTIFY_FUNCTION}
  !insertmacro ZC_LIFECYCLE_WITHDRAW_ASSOC "${ZC_PREVIEW_ASSOCIATION_ROOT}\${ZC_PREVIEW_EXTENSION_04}\shellex\${ZC_PREVIEW_SHELLEX_CATEGORY}" ${ROLLBACK_FUNCTION} ${NOTIFY_FUNCTION}
  !insertmacro ZC_LIFECYCLE_WITHDRAW_ASSOC "${ZC_PREVIEW_ASSOCIATION_ROOT}\${ZC_PREVIEW_EXTENSION_05}\shellex\${ZC_PREVIEW_SHELLEX_CATEGORY}" ${ROLLBACK_FUNCTION} ${NOTIFY_FUNCTION}
  !insertmacro ZC_LIFECYCLE_WITHDRAW_ASSOC "${ZC_PREVIEW_ASSOCIATION_ROOT}\${ZC_PREVIEW_EXTENSION_06}\shellex\${ZC_PREVIEW_SHELLEX_CATEGORY}" ${ROLLBACK_FUNCTION} ${NOTIFY_FUNCTION}
  !insertmacro ZC_LIFECYCLE_WITHDRAW_ASSOC "${ZC_PREVIEW_ASSOCIATION_ROOT}\${ZC_PREVIEW_EXTENSION_07}\shellex\${ZC_PREVIEW_SHELLEX_CATEGORY}" ${ROLLBACK_FUNCTION} ${NOTIFY_FUNCTION}
  !insertmacro ZC_LIFECYCLE_WITHDRAW_ASSOC "${ZC_PREVIEW_ASSOCIATION_ROOT}\${ZC_PREVIEW_EXTENSION_08}\shellex\${ZC_PREVIEW_SHELLEX_CATEGORY}" ${ROLLBACK_FUNCTION} ${NOTIFY_FUNCTION}
  !insertmacro ZC_LIFECYCLE_WITHDRAW_ASSOC "${ZC_PREVIEW_ASSOCIATION_ROOT}\${ZC_PREVIEW_EXTENSION_09}\shellex\${ZC_PREVIEW_SHELLEX_CATEGORY}" ${ROLLBACK_FUNCTION} ${NOTIFY_FUNCTION}
  !insertmacro ZC_LIFECYCLE_WITHDRAW_ASSOC "${ZC_PREVIEW_ASSOCIATION_ROOT}\${ZC_PREVIEW_EXTENSION_10}\shellex\${ZC_PREVIEW_SHELLEX_CATEGORY}" ${ROLLBACK_FUNCTION} ${NOTIFY_FUNCTION}
  !insertmacro ZC_LIFECYCLE_WITHDRAW_ASSOC "${ZC_PREVIEW_ASSOCIATION_ROOT}\${ZC_PREVIEW_EXTENSION_11}\shellex\${ZC_PREVIEW_SHELLEX_CATEGORY}" ${ROLLBACK_FUNCTION} ${NOTIFY_FUNCTION}
  !insertmacro ZC_LIFECYCLE_WITHDRAW_ASSOC "${ZC_PREVIEW_ASSOCIATION_ROOT}\${ZC_PREVIEW_EXTENSION_12}\shellex\${ZC_PREVIEW_SHELLEX_CATEGORY}" ${ROLLBACK_FUNCTION} ${NOTIFY_FUNCTION}
  !insertmacro ZC_LIFECYCLE_WITHDRAW_ASSOC "${ZC_PREVIEW_ASSOCIATION_ROOT}\${ZC_PREVIEW_EXTENSION_13}\shellex\${ZC_PREVIEW_SHELLEX_CATEGORY}" ${ROLLBACK_FUNCTION} ${NOTIFY_FUNCTION}
  !insertmacro ZC_LIFECYCLE_WITHDRAW_ASSOC "${ZC_PREVIEW_ASSOCIATION_ROOT}\${ZC_PREVIEW_EXTENSION_14}\shellex\${ZC_PREVIEW_SHELLEX_CATEGORY}" ${ROLLBACK_FUNCTION} ${NOTIFY_FUNCTION}
  !insertmacro ZC_LIFECYCLE_WITHDRAW_ASSOC "${ZC_PREVIEW_ASSOCIATION_ROOT}\${ZC_PREVIEW_EXTENSION_15}\shellex\${ZC_PREVIEW_SHELLEX_CATEGORY}" ${ROLLBACK_FUNCTION} ${NOTIFY_FUNCTION}
  !insertmacro ZC_LIFECYCLE_WITHDRAW_ASSOC "${ZC_PREVIEW_ASSOCIATION_ROOT}\${ZC_PREVIEW_EXTENSION_16}\shellex\${ZC_PREVIEW_SHELLEX_CATEGORY}" ${ROLLBACK_FUNCTION} ${NOTIFY_FUNCTION}

  !insertmacro ZC_LIFECYCLE_WITHDRAW_CORE "${ZC_PREVIEW_HANDLERS_KEY}" "${ZC_PREVIEW_PRODUCTION_CLSID}" "${ZC_PREVIEW_FRIENDLY_NAME}" ${ROLLBACK_FUNCTION} ${NOTIFY_FUNCTION}
  !insertmacro ZC_LIFECYCLE_WITHDRAW_CORE "${ZC_PREVIEW_INPROC_KEY}" "" "$ZC_PREVIEW_DLL_PROBE_PATH" ${ROLLBACK_FUNCTION} ${NOTIFY_FUNCTION}
  !insertmacro ZC_LIFECYCLE_WITHDRAW_CORE "${ZC_PREVIEW_INPROC_KEY}" "ThreadingModel" "${ZC_PREVIEW_THREADING_MODEL}" ${ROLLBACK_FUNCTION} ${NOTIFY_FUNCTION}
  !insertmacro ZC_LIFECYCLE_WITHDRAW_CORE "${ZC_PREVIEW_CLSID_KEY}" "AppID" "${ZC_PREVIEW_PREVHOST_APP_ID}" ${ROLLBACK_FUNCTION} ${NOTIFY_FUNCTION}
  !insertmacro ZC_LIFECYCLE_WITHDRAW_CORE "${ZC_PREVIEW_CLSID_KEY}" "" "${ZC_PREVIEW_FRIENDLY_NAME}" ${ROLLBACK_FUNCTION} ${NOTIFY_FUNCTION}

  Call ${STALE_FUNCTION}
  ${If} $ZC_LIFECYCLE_PREVIEW_OK != 1
    Return
  ${EndIf}
  ; Registration withdrawal plus the shell notification is the lifecycle
  ; quiesce boundary. Exact DLL servicing is owned by the generated resource
  ; macro; release polling is not a hard pre-file gate.
  Call ${NOTIFY_FUNCTION}
  StrCpy $ZC_LIFECYCLE_PREVIEW_OK 1
!macroend

Function ZCQuiescePreviewForLifecycle
  !insertmacro ZC_LIFECYCLE_WITHDRAW_PREVIEW_BODY RollbackZenCanvasPreviewRegistration ZCLifecycleRemoveStalePreviewAssociations NotifyZenCanvasPreviewAssociationChanged
FunctionEnd

Function un.ZCQuiescePreviewForLifecycle
  !insertmacro ZC_LIFECYCLE_WITHDRAW_PREVIEW_BODY un.RollbackZenCanvasPreviewRegistration un.ZCLifecycleRemoveStalePreviewAssociations un.NotifyZenCanvasPreviewAssociationChanged
FunctionEnd

; ---------------------------------------------------------------------------
; Reversible recovery and MUI cancellation ownership.
; ---------------------------------------------------------------------------

Function ZCRecoverInstallReversible
  ${If} $ZC_LIFECYCLE_INSTALL_STAGE != ${ZC_LIFECYCLE_STAGE_REVERSIBLE_PREPARATION}
    Return
  ${EndIf}
  ${If} $ZC_LIFECYCLE_INSTALL_RECOVERY_DONE == 1
    Return
  ${EndIf}
  StrCpy $ZC_LIFECYCLE_INSTALL_RECOVERY_DONE 1
  ${If} $ZC_PREVIEW_QUIESCE_ACTIVE == 1
    Call RollbackZenCanvasPreviewQuiesce
  ${EndIf}
  ${If} $ZC_PREVIEW_ROLLBACK_CLEAN != 1
    StrCpy $ZC_LIFECYCLE_PREVIEW_FAILURE_CLEAN 0
  ${EndIf}
  StrCpy $ZC_POSTINSTALL_SERVICE_CLEAN 1
  Call RestoreZenCanvasPreexistingService
  StrCpy $ZC_LIFECYCLE_INSTALL_STAGE ${ZC_LIFECYCLE_STAGE_INACTIVE}
  StrCpy $ZC_INSTALL_LIFECYCLE_ACTIVE 0
FunctionEnd

Function un.ZCRecoverUninstallReversible
  ${If} $ZC_LIFECYCLE_UNINSTALL_STAGE != ${ZC_LIFECYCLE_STAGE_REVERSIBLE_PREPARATION}
    Return
  ${EndIf}
  ${If} $ZC_LIFECYCLE_UNINSTALL_RECOVERY_DONE == 1
    Return
  ${EndIf}
  StrCpy $ZC_LIFECYCLE_UNINSTALL_RECOVERY_DONE 1
  ${If} $ZC_PREVIEW_QUIESCE_ACTIVE == 1
    Call un.RollbackZenCanvasPreviewQuiesce
    ${If} $ZC_PREVIEW_ROLLBACK_CLEAN == 1
      Call un.VerifyZenCanvasPreviewRecovery
    ${Else}
      StrCpy $ZC_UNINSTALL_PREVIEW_RECOVERED 0
    ${EndIf}
  ${Else}
    StrCpy $ZC_UNINSTALL_PREVIEW_RECOVERED 1
  ${EndIf}
  Call un.RestoreZenCanvasOriginalService
  StrCpy $ZC_LIFECYCLE_UNINSTALL_STAGE ${ZC_LIFECYCLE_STAGE_INACTIVE}
  StrCpy $ZC_UNINSTALL_LIFECYCLE_STAGE ${ZC_LIFECYCLE_STAGE_INACTIVE}
FunctionEnd

Function ZCLifecycleUserAbort
  ${If} $ZC_LIFECYCLE_INSTALL_STAGE == ${ZC_LIFECYCLE_STAGE_REVERSIBLE_PREPARATION}
    Call ZCRecoverInstallReversible
    ${If} $ZC_POSTINSTALL_SERVICE_CLEAN != 1
      Abort
    ${EndIf}
    Return
  ${EndIf}
  ${If} $ZC_LIFECYCLE_INSTALL_STAGE >= ${ZC_LIFECYCLE_STAGE_FILE_MUTATION}
    Abort
  ${EndIf}
FunctionEnd

Function un.ZCLifecycleUserAbort
  ${If} $ZC_LIFECYCLE_UNINSTALL_STAGE == ${ZC_LIFECYCLE_STAGE_REVERSIBLE_PREPARATION}
    Call un.ZCRecoverUninstallReversible
    ${If} $ZC_UNINSTALL_PREVIEW_RECOVERED != 1
    ${OrIf} $ZC_UNINSTALL_SERVICE_CLEAN != 1
      Abort
    ${EndIf}
    Return
  ${EndIf}
  ${If} $ZC_LIFECYCLE_UNINSTALL_STAGE >= ${ZC_LIFECYCLE_STAGE_FILE_MUTATION}
    Abort
  ${EndIf}
FunctionEnd

; ---------------------------------------------------------------------------
; Install / repair lifecycle.
; ---------------------------------------------------------------------------

Function ZCInitializeInstallLifecycle
  StrCpy $ZC_MAIN_BINARY_FILENAME "${MAINBINARYNAME}.exe"
  StrCpy $ZC_UNINSTALLER_REGISTRY_KEY "${UNINSTKEY}"
  StrCpy $ZC_MANUFACTURER_PRODUCT_KEY "${MANUPRODUCTKEY}"
  StrCpy $ZC_PRODUCT_NAME "${PRODUCTNAME}"
  StrCpy $ZC_MANUFACTURER_NAME "${MANUFACTURER}"
  StrCpy $ZC_INDEX_SERVICE_CREATED 0
  StrCpy $ZC_INDEX_SERVICE_CREATE_SUCCEEDED 0
  StrCpy $ZC_INDEX_SERVICE_CREATE_OWNERSHIP_VERIFIED 0
  StrCpy $ZC_POSTINSTALL_ACTIVE 0
  StrCpy $ZC_POSTINSTALL_SERVICE_CLEAN 1
  StrCpy $ZC_POSTINSTALL_METADATA_CLEAN 1
  StrCpy $ZC_INSTALL_FAILURE_OWNER_DONE 0
  StrCpy $ZC_LIFECYCLE_PRODUCT_COHERENT 0
  StrCpy $ZC_LIFECYCLE_PREVIEW_FAILURE_CLEAN 1
  StrCpy $ZC_INSTALL_LIFECYCLE_ACTIVE 1
  StrCpy $ZC_PREVIEW_QUIESCE_ACTIVE 0
  StrCpy $ZC_PREVIEW_TXN_COUNT 0
  StrCpy $ZC_PREVIEW_ROLLBACK_CLEAN 1
  StrCpy $ZC_PREEXISTING_SERVICE 0
  StrCpy $ZC_PREEXISTING_SERVICE_WAS_RUNNING 0
  StrCpy $ZC_PREEXISTING_SERVICE_STATE_CAPTURED 0
  StrCpy $ZC_LIFECYCLE_INSTALL_STAGE ${ZC_LIFECYCLE_STAGE_REVERSIBLE_PREPARATION}
  StrCpy $ZC_LIFECYCLE_INSTALL_RECOVERY_DONE 0
  Call ZCResetPreviewDllMutationState

  ; All ownership/evidence is validated before the first service state change.
  Call ValidateLegacyZenCanvasIndexServiceAbsent
  Call ValidateZenCanvasPreexistingProduct
  Call ValidateZenCanvasPreviewCore
  Call ValidateZenCanvasIndexServiceOwnership
  ${If} $ZC_PREEXISTING_PRODUCT == 0
  ${AndIf} $ZC_INDEX_SERVICE_OWNERSHIP == 1
    MessageBox MB_ICONSTOP|MB_OK "A Fylura Global Index service already exists without consistent Tauri product metadata. Installation was not changed." /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
  ${If} $ZC_INDEX_SERVICE_OWNERSHIP == 1
    StrCpy $ZC_PREEXISTING_SERVICE 1
    Call CaptureZenCanvasPreexistingServiceState
  ${EndIf}
FunctionEnd

Function ZCPrepareInstallLifecycle
  Call ZCInitializeInstallLifecycle

  ; 1) SCM-owned service stop. 2) name-only desktop app gate. 3) Preview
  ; withdrawal + Shell notification. Exact DLL servicing is performed around
  ; the generated resource mutation.
  Call ZCStopCapturedServiceForLifecycle
  ${If} $ZC_LIFECYCLE_STOP_OK != 1
    Call ZCRecoverInstallReversible
    MessageBox MB_ICONSTOP|MB_OK "Fylura could not stop the exact captured Global Index service before file mutation. The captured service state was restored where verifiable." /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}

  Call ZCResolveMainAppGate
  ${If} $ZC_LIFECYCLE_GATE_OK != 1
    Call ZCRecoverInstallReversible
    MessageBox MB_ICONSTOP|MB_OK "Fylura installation was cancelled or the desktop process could not be closed before file mutation. The captured service state was restored where verifiable." /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}

  Call ZCQuiescePreviewForLifecycle
  ${If} $ZC_LIFECYCLE_PREVIEW_OK != 1
    Call ZCRecoverInstallReversible
    MessageBox MB_ICONSTOP|MB_OK "Fylura could not withdraw the exact Preview Handler registration before file mutation. Preview and the captured service state were restored where verifiable." /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
FunctionEnd

Function ZCMarkInstallIrreversible
  ${If} $ZC_LIFECYCLE_INSTALL_STAGE < ${ZC_LIFECYCLE_STAGE_FILE_MUTATION}
    StrCpy $ZC_LIFECYCLE_INSTALL_STAGE ${ZC_LIFECYCLE_STAGE_FILE_MUTATION}
  ${EndIf}
FunctionEnd

Function ZCMarkInstallGeneratedMutation
  ${If} $ZC_LIFECYCLE_INSTALL_STAGE < ${ZC_LIFECYCLE_STAGE_GENERATED_MUTATION}
    StrCpy $ZC_LIFECYCLE_INSTALL_STAGE ${ZC_LIFECYCLE_STAGE_GENERATED_MUTATION}
  ${EndIf}
FunctionEnd

Function ZCMarkInstallPostGeneratedIntegration
  ${If} $ZC_LIFECYCLE_INSTALL_STAGE < ${ZC_LIFECYCLE_STAGE_POST_GENERATED_INTEGRATION}
    StrCpy $ZC_LIFECYCLE_INSTALL_STAGE ${ZC_LIFECYCLE_STAGE_POST_GENERATED_INTEGRATION}
  ${EndIf}
FunctionEnd

Function ZCFailInstallReversible
  Call ZCRecoverInstallReversible
  MessageBox MB_ICONSTOP|MB_OK "Fylura could not replace the main executable before generated install mutation became irreversible. The prior Preview and captured service state were restored where verifiable." /SD IDOK
  SetErrorLevel 2
  Abort
FunctionEnd

Function ZCFailInstallPartial
  ${If} $ZC_POSTINSTALL_FAILURE_REASON == ""
    StrCpy $ZC_POSTINSTALL_FAILURE_REASON "The generated NSIS install section failed before the product lifecycle completed."
  ${EndIf}
  SetErrorLevel 2
  Call ZCHandleGeneratedInstallFailureFinal
  MessageBox MB_ICONSTOP|MB_OK "Fylura installation did not complete after generated product mutation began. Preview remains withdrawn unless exact current product coherence proved a safe rollback; service and fresh metadata cleanup were limited to exact ownership evidence." /SD IDOK
  Abort
FunctionEnd

Function ZCFinishInstallLifecycle
  StrCpy $ZC_LIFECYCLE_INSTALL_STAGE ${ZC_LIFECYCLE_STAGE_COMPLETE}
  StrCpy $ZC_LIFECYCLE_INSTALL_RECOVERY_DONE 1
  StrCpy $ZC_INSTALL_LIFECYCLE_ACTIVE 0
FunctionEnd

; ---------------------------------------------------------------------------
; Uninstall lifecycle.
; ---------------------------------------------------------------------------

Function un.ZCInitializeUninstallLifecycle
  StrCpy $ZC_MAIN_BINARY_FILENAME "${MAINBINARYNAME}.exe"
  StrCpy $ZC_UNINSTALLER_REGISTRY_KEY "${UNINSTKEY}"
  StrCpy $ZC_MANUFACTURER_PRODUCT_KEY "${MANUPRODUCTKEY}"
  StrCpy $ZC_PRODUCT_NAME "${PRODUCTNAME}"
  StrCpy $ZC_MANUFACTURER_NAME "${MANUFACTURER}"
  StrCpy $ZC_EXPECTED_INSTALL_LOCATION "$\"$INSTDIR$\""
  StrCpy $ZC_EXPECTED_UNINSTALL_STRING "$\"$INSTDIR\uninstall.exe$\""
  StrCpy $ZC_EXPECTED_DISPLAY_ICON "$\"$INSTDIR\$ZC_MAIN_BINARY_FILENAME$\""
  StrCpy $ZC_EXPECTED_DISPLAY_VERSION "${VERSION}"
  StrCpy $ZC_EXPECTED_PUBLISHER "$ZC_MANUFACTURER_NAME"
  StrCpy $ZC_EXPECTED_HOMEPAGE "${HOMEPAGE}"
  ${GetSize} "$INSTDIR" "/M=uninstall.exe /S=0K /G=0" $0 $1 $2
  IntOp $0 $0 + ${ESTIMATEDSIZE}
  StrCpy $ZC_EXPECTED_ESTIMATED_SIZE $0
  StrCpy $ZC_PREVIEW_ARTIFACT_REMOVED 0
  StrCpy $ZC_UNINSTALL_SERVICE_CLEAN 1
  StrCpy $ZC_UNINSTALL_MANUFACTURER_CLEAN 0
  StrCpy $ZC_PREVIEW_QUIESCE_ACTIVE 0
  StrCpy $ZC_PREVIEW_TXN_COUNT 0
  StrCpy $ZC_PREVIEW_ROLLBACK_CLEAN 1
  Call un.ZCResetPreviewDllMutationState
  StrCpy $ZC_UNINSTALL_LIFECYCLE_STAGE ${ZC_LIFECYCLE_STAGE_INACTIVE}
  StrCpy $ZC_UNINSTALL_RECOVERY_DONE 0
  StrCpy $ZC_UNINSTALL_ORIGINAL_SERVICE 0
  StrCpy $ZC_UNINSTALL_ORIGINAL_SERVICE_WAS_RUNNING 0
  StrCpy $ZC_UNINSTALL_SERVICE_STATE_CAPTURED 0
  StrCpy $ZC_UNINSTALL_PREDELETE_EVIDENCE_CAPTURED 0
  StrCpy $ZC_UNINSTALL_PREDELETE_COHERENT 0
  StrCpy $ZC_UNINSTALL_PREVIEW_RECOVERED 0
  StrCpy $ZC_LIFECYCLE_UNINSTALL_STAGE ${ZC_LIFECYCLE_STAGE_REVERSIBLE_PREPARATION}
  StrCpy $ZC_UNINSTALL_LIFECYCLE_STAGE ${ZC_LIFECYCLE_STAGE_REVERSIBLE_PREPARATION}
  StrCpy $ZC_LIFECYCLE_UNINSTALL_RECOVERY_DONE 0

  ; Capture every authority before the service is stopped.
  Call un.ValidateZenCanvasPreviewCore
  Call un.ValidateZenCanvasIndexServiceOwnership
  Call un.CaptureZenCanvasOriginalServiceState
  Call un.CheckZenCanvasPreDeleteProductEvidence
  ${If} $ZC_UNINSTALL_PREDELETE_COHERENT != 1
    MessageBox MB_ICONSTOP|MB_OK "The installed Fylura product could not be verified as a coherent pre-delete installation. Uninstall was not changed." /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
  StrCpy $ZC_UNINSTALL_PREDELETE_EVIDENCE_CAPTURED 1
FunctionEnd

Function un.ZCPrepareUninstallLifecycle
  Call un.ZCInitializeUninstallLifecycle

  ; 1) SCM-owned service stop. 2) name-only desktop app gate. 3) Preview
  ; withdrawal + Shell notification. Exact DLL servicing is performed around
  ; the generated resource deletion.
  Call un.ZCStopCapturedServiceForLifecycle
  ${If} $ZC_LIFECYCLE_STOP_OK != 1
    Call un.ZCRecoverUninstallReversible
    MessageBox MB_ICONSTOP|MB_OK "Fylura could not stop the exact captured Global Index service before uninstall deletion. The original service state was restored where verifiable." /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}

  Call un.ZCResolveMainAppGate
  ${If} $ZC_LIFECYCLE_GATE_OK != 1
    Call un.ZCRecoverUninstallReversible
    MessageBox MB_ICONSTOP|MB_OK "Fylura uninstall was cancelled or the desktop process could not be closed before file deletion. The original service state was restored where verifiable." /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}

  Call un.ZCQuiescePreviewForLifecycle
  ${If} $ZC_LIFECYCLE_PREVIEW_OK != 1
    Call un.ZCRecoverUninstallReversible
    MessageBox MB_ICONSTOP|MB_OK "Fylura could not withdraw the exact Preview Handler registration before file deletion. Preview and the original service state were restored where verifiable." /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
FunctionEnd

Function un.ZCMarkUninstallIrreversible
  ${If} $ZC_LIFECYCLE_UNINSTALL_STAGE < ${ZC_LIFECYCLE_STAGE_FILE_MUTATION}
    StrCpy $ZC_LIFECYCLE_UNINSTALL_STAGE ${ZC_LIFECYCLE_STAGE_FILE_MUTATION}
  ${EndIf}
  ${If} $ZC_UNINSTALL_LIFECYCLE_STAGE < ${ZC_LIFECYCLE_STAGE_FILE_MUTATION}
    StrCpy $ZC_UNINSTALL_LIFECYCLE_STAGE ${ZC_LIFECYCLE_STAGE_FILE_MUTATION}
  ${EndIf}
FunctionEnd

Function un.ZCMarkUninstallGeneratedMutation
  ${If} $ZC_LIFECYCLE_UNINSTALL_STAGE < ${ZC_LIFECYCLE_STAGE_GENERATED_MUTATION}
    StrCpy $ZC_LIFECYCLE_UNINSTALL_STAGE ${ZC_LIFECYCLE_STAGE_GENERATED_MUTATION}
  ${EndIf}
  ${If} $ZC_UNINSTALL_LIFECYCLE_STAGE < ${ZC_LIFECYCLE_STAGE_GENERATED_MUTATION}
    StrCpy $ZC_UNINSTALL_LIFECYCLE_STAGE ${ZC_LIFECYCLE_STAGE_GENERATED_MUTATION}
  ${EndIf}
FunctionEnd

Function un.ZCMarkUninstallPostGeneratedIntegration
  ${If} $ZC_LIFECYCLE_UNINSTALL_STAGE < ${ZC_LIFECYCLE_STAGE_POST_GENERATED_INTEGRATION}
    StrCpy $ZC_LIFECYCLE_UNINSTALL_STAGE ${ZC_LIFECYCLE_STAGE_POST_GENERATED_INTEGRATION}
  ${EndIf}
  ${If} $ZC_UNINSTALL_LIFECYCLE_STAGE < ${ZC_LIFECYCLE_STAGE_POST_GENERATED_INTEGRATION}
    StrCpy $ZC_UNINSTALL_LIFECYCLE_STAGE ${ZC_LIFECYCLE_STAGE_POST_GENERATED_INTEGRATION}
  ${EndIf}
FunctionEnd

Function un.ZCFailUninstallReversible
  Call un.ZCRecoverUninstallReversible
  MessageBox MB_ICONSTOP|MB_OK "Fylura could not delete the main executable before uninstall became irreversible. Preview and the original service state were restored where verifiable." /SD IDOK
  SetErrorLevel 2
  Abort
FunctionEnd

Function un.ZCFailUninstallPartial
  ${If} $ZC_LIFECYCLE_UNINSTALL_STAGE < ${ZC_LIFECYCLE_STAGE_GENERATED_MUTATION}
    StrCpy $ZC_LIFECYCLE_UNINSTALL_STAGE ${ZC_LIFECYCLE_STAGE_GENERATED_MUTATION}
  ${EndIf}
  ${If} $ZC_UNINSTALL_LIFECYCLE_STAGE < ${ZC_LIFECYCLE_STAGE_GENERATED_MUTATION}
    StrCpy $ZC_UNINSTALL_LIFECYCLE_STAGE ${ZC_LIFECYCLE_STAGE_GENERATED_MUTATION}
  ${EndIf}
  SetErrorLevel 2
  ${If} $ZC_PREVIEW_QUIESCE_ACTIVE == 1
    Call un.CommitZenCanvasPreviewQuiesce
  ${EndIf}
  Call un.DeleteZenCanvasIndexService
  MessageBox MB_ICONSTOP|MB_OK "Fylura uninstall is incomplete after generated deletion began. Preview remains withdrawn and the exact-owned Global Index service was removed where verifiable; product files or metadata may remain." /SD IDOK
  Abort
FunctionEnd

Function un.ZCFinishUninstallLifecycle
  StrCpy $ZC_LIFECYCLE_UNINSTALL_STAGE ${ZC_LIFECYCLE_STAGE_COMPLETE}
  StrCpy $ZC_UNINSTALL_LIFECYCLE_STAGE ${ZC_LIFECYCLE_STAGE_COMPLETE}
  StrCpy $ZC_LIFECYCLE_UNINSTALL_RECOVERY_DONE 1
FunctionEnd
