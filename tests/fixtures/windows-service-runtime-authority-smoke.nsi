Unicode true
RequestExecutionLevel admin
SilentInstall silent
AutoCloseWindow true
ShowInstDetails nevershow

!ifndef ZC_SMOKE_OUTFILE
!define ZC_SMOKE_OUTFILE "windows-service-runtime-authority-smoke.exe"
!endif
!ifndef ZC_SMOKE_SERVICE_NAME
!define ZC_SMOKE_SERVICE_NAME "FyluraServiceRuntimeSmoke"
!endif
!ifndef ZC_SMOKE_ABSENT_SERVICE_NAME
!define ZC_SMOKE_ABSENT_SERVICE_NAME "FyluraServiceRuntimeSmokeAbsent"
!endif
!ifndef ZC_SERVICE_RUNTIME_AUTHORITY_FILE
!error "ZC_SERVICE_RUNTIME_AUTHORITY_FILE must point to the shared SCM authority."
!endif

OutFile "${ZC_SMOKE_OUTFILE}"
Name "Zen Canvas Service Runtime Authority Smoke"

!include "${ZC_SERVICE_RUNTIME_AUTHORITY_FILE}"

Var ZC_SMOKE_FAILED
Var ZC_SMOKE_LOG
Var ZC_SMOKE_SERVICE_CREATED

!macro ZC_SMOKE_REQUIRE LABEL EXPECTED
  ${If} $ZC_SERVICE_RUNTIME_STATE != ${EXPECTED}
    StrCpy $ZC_SMOKE_FAILED 1
    FileWrite $ZC_SMOKE_LOG "${LABEL}: expected ${EXPECTED}, got $ZC_SERVICE_RUNTIME_STATE$\r$\n"
  ${EndIf}
!macroend

!macro ZC_SMOKE_REQUIRE_MAPPING SCM_STATE EXPECTED
  StrCpy $ZC_SERVICE_RUNTIME_CURRENT_STATE ${SCM_STATE}
  Call ZCMapServiceRuntimeState
  !insertmacro ZC_SMOKE_REQUIRE "mapping-${SCM_STATE}" ${EXPECTED}
!macroend

!macro ZC_SMOKE_QUERY_SERVICE LABEL SERVICE_NAME EXPECTED
  !insertmacro ZC_QUERY_SERVICE_RUNTIME_STATE "${SERVICE_NAME}"
  FileWrite $ZC_SMOKE_LOG "${LABEL}: state=$ZC_SERVICE_RUNTIME_STATE error=$ZC_SERVICE_RUNTIME_ERROR current=$ZC_SERVICE_RUNTIME_CURRENT_STATE close=$ZC_SERVICE_RUNTIME_CLOSE_RESULT$\r$\n"
  !insertmacro ZC_SMOKE_REQUIRE "${LABEL}" ${EXPECTED}
!macroend

!ifdef ZC_SMOKE_CLEANUP_ONLY

Section
  SetErrorLevel 1
  StrCpy $ZC_SMOKE_FAILED 0
  FileOpen $ZC_SMOKE_LOG "$EXEDIR\service-runtime-authority-smoke.log" w
  StrCpy $2 0
smoke_cleanup_verify_loop:
  IntCmp $2 20 smoke_cleanup_verify_failed 0 0
  !insertmacro ZC_SMOKE_QUERY_SERVICE "cleanup-only" "${ZC_SMOKE_SERVICE_NAME}" ${ZC_SERVICE_RUNTIME_ABSENT}
  ${If} $ZC_SERVICE_RUNTIME_STATE == ${ZC_SERVICE_RUNTIME_ABSENT}
    Goto smoke_cleanup_verify_success
  ${EndIf}
  Sleep 250
  IntOp $2 $2 + 1
  Goto smoke_cleanup_verify_loop

smoke_cleanup_verify_failed:
  StrCpy $ZC_SMOKE_FAILED 1
smoke_cleanup_verify_success:
  FileClose $ZC_SMOKE_LOG
  ${If} $ZC_SMOKE_FAILED == 0
    SetErrorLevel 0
  ${EndIf}
SectionEnd

!else

Section
  SetErrorLevel 1
  StrCpy $ZC_SMOKE_FAILED 0
  StrCpy $ZC_SMOKE_SERVICE_CREATED 0
  FileOpen $ZC_SMOKE_LOG "$EXEDIR\service-runtime-authority-smoke.log" w

  ; 18A: a unique service name is positively ABSENT before creation.
  !insertmacro ZC_SMOKE_QUERY_SERVICE "absent-before-create" "${ZC_SMOKE_ABSENT_SERVICE_NAME}" ${ZC_SERVICE_RUNTIME_ABSENT}

  ; 18D: an invalid SCM service name is an API uncertainty, not ABSENT.
  !insertmacro ZC_SMOKE_QUERY_SERVICE "invalid-service-name" "FyluraServiceRuntimeSmoke/invalid" ${ZC_SERVICE_RUNTIME_UNKNOWN}

  ; T87: exercise the complete numeric SERVICE_* mapping through the same
  ; mapper used after QueryServiceStatusEx. No hosted system-service
  ; assumption is needed for the RUNNING case.
  !insertmacro ZC_SMOKE_REQUIRE_MAPPING 1 ${ZC_SERVICE_RUNTIME_STOPPED}
  !insertmacro ZC_SMOKE_REQUIRE_MAPPING 2 ${ZC_SERVICE_RUNTIME_PENDING}
  !insertmacro ZC_SMOKE_REQUIRE_MAPPING 3 ${ZC_SERVICE_RUNTIME_PENDING}
  !insertmacro ZC_SMOKE_REQUIRE_MAPPING 4 ${ZC_SERVICE_RUNTIME_RUNNING}
  !insertmacro ZC_SMOKE_REQUIRE_MAPPING 5 ${ZC_SERVICE_RUNTIME_PENDING}
  !insertmacro ZC_SMOKE_REQUIRE_MAPPING 6 ${ZC_SERVICE_RUNTIME_PENDING}
  !insertmacro ZC_SMOKE_REQUIRE_MAPPING 7 ${ZC_SERVICE_RUNTIME_UNKNOWN}

  ; 18B: create a harmless, demand-start test service and leave it STOPPED.
  ; It is never started; notepad.exe is only a harmless service image path.
  nsExec::ExecToStack '"$SYSDIR\sc.exe" create "${ZC_SMOKE_SERVICE_NAME}" binPath= $SYSDIR\notepad.exe start= demand'
  Pop $0
  Pop $1
  FileWrite $ZC_SMOKE_LOG "create-status=$0 output=$1$\r$\n"
  ${If} $0 == 0
    StrCpy $ZC_SMOKE_SERVICE_CREATED 1
  ${Else}
    StrCpy $ZC_SMOKE_FAILED 1
  ${EndIf}

  ${If} $ZC_SMOKE_SERVICE_CREATED == 1
    !insertmacro ZC_SMOKE_QUERY_SERVICE "created-service" "${ZC_SMOKE_SERVICE_NAME}" ${ZC_SERVICE_RUNTIME_STOPPED}

    ; Delete only the service created by this executable, then prove absence
    ; through the same SCM authority with a bounded read-only wait.
    nsExec::ExecToStack '"$SYSDIR\sc.exe" delete "${ZC_SMOKE_SERVICE_NAME}"'
    Pop $0
    Pop $1
    FileWrite $ZC_SMOKE_LOG "delete-status=$0 output=$1$\r$\n"
    ${If} $0 != 0
    ${AndIf} $0 != 1060
      StrCpy $ZC_SMOKE_FAILED 1
    ${EndIf}

    StrCpy $2 0
smoke_cleanup_loop:
    IntCmp $2 20 smoke_cleanup_timeout 0 0
    !insertmacro ZC_SMOKE_QUERY_SERVICE "cleanup-after-delete" "${ZC_SMOKE_SERVICE_NAME}" ${ZC_SERVICE_RUNTIME_ABSENT}
    ${If} $ZC_SERVICE_RUNTIME_STATE == ${ZC_SERVICE_RUNTIME_ABSENT}
      Goto smoke_cleanup_done
    ${EndIf}
    Sleep 250
    IntOp $2 $2 + 1
    Goto smoke_cleanup_loop

smoke_cleanup_timeout:
    StrCpy $ZC_SMOKE_FAILED 1
smoke_cleanup_done:
  ${EndIf}

  FileClose $ZC_SMOKE_LOG
  ${If} $ZC_SMOKE_FAILED == 0
    SetErrorLevel 0
  ${EndIf}
SectionEnd

!endif
