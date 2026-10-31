Unicode true
RequestExecutionLevel user
SilentInstall silent
AutoCloseWindow true
ShowInstDetails nevershow

!ifndef ZC_SMOKE_OUTFILE
!error "ZC_SMOKE_OUTFILE must point to the smoke executable."
!endif
!ifndef ZC_SMOKE_SOURCE
!error "ZC_SMOKE_SOURCE must point to the packaged Preview Handler DLL."
!endif
!ifndef ZC_PREVIEW_DLL_SERVICING_FILE
!error "ZC_PREVIEW_DLL_SERVICING_FILE must point to the Preview servicing macros."
!endif

OutFile "${ZC_SMOKE_OUTFILE}"
InstallDir "$EXEDIR\unused"
Name "Zen Canvas Preview Resource Smoke"

Var ZC_PREVIEW_RELEASE_READY
Var ZC_POSTINSTALL_FAILURE_REASON
Var ZC_SMOKE_FAILED
Var ZC_SMOKE_LOG
Var ZC_SMOKE_ROOT
Var ZC_SMOKE_OLD_HANDLE
Var ZC_SMOKE_REPLACEMENT_HANDLE

!define ZC_PREVIEW_INSTALLED_DLL "$INSTDIR\native\fylura_windows_preview_handler.dll"

!include "FileFunc.nsh"
!include "Util.nsh"
!include "${ZC_PREVIEW_DLL_SERVICING_FILE}"

!macro ZC_SMOKE_ASSERT_PATHS LABEL
  IfFileExists "$ZC_SMOKE_ROOT\native\fylura_windows_preview_handler.dll" 0 ${LABEL}_canonical_missing
  IfFileExists "$ZC_SMOKE_ROOT\nativefylura_windows_preview_handler.dll" ${LABEL}_flattened_present ${LABEL}_done
${LABEL}_canonical_missing:
  StrCpy $ZC_SMOKE_FAILED 1
  FileWrite $ZC_SMOKE_LOG "${LABEL}: canonical Preview DLL is missing$\r$\n"
  Goto ${LABEL}_done
${LABEL}_flattened_present:
  StrCpy $ZC_SMOKE_FAILED 1
  FileWrite $ZC_SMOKE_LOG "${LABEL}: flattened Preview DLL path exists$\r$\n"
${LABEL}_done:
!macroend

Section
  SetErrorLevel 1
  StrCpy $ZC_SMOKE_FAILED 0
  FileOpen $ZC_SMOKE_LOG "$EXEDIR\windows-preview-resource-smoke.log" w

  ; Fresh path: the parent directory exists before the resource servicing
  ; macro writes the canonical nested NSIS destination. Use the forward
  ; representation to exercise flexible identity matching.
  StrCpy $INSTDIR "$EXEDIR\fresh-root"
  StrCpy $ZC_SMOKE_ROOT $INSTDIR
  SetOutPath $INSTDIR
  ClearErrors
  CreateDirectory "$ZC_SMOKE_ROOT\native"
  IfErrors smoke_fresh_directory_failure
  Call ZCResetPreviewDllMutationState
  !insertmacro ZC_INSTALL_RESOURCE "native/fylura_windows_preview_handler.dll" "${ZC_SMOKE_SOURCE}"
  !insertmacro ZC_SMOKE_ASSERT_PATHS "fresh"
  Goto smoke_mapped_start

smoke_fresh_directory_failure:
  StrCpy $ZC_SMOKE_FAILED 1
  FileWrite $ZC_SMOKE_LOG "fresh: native parent directory could not be created$\r$\n"

smoke_mapped_start:
  ; Mapped path: seed and load the old image, then let the same servicing
  ; macro retire it and replace the canonical path. Use the generated
  ; backslash representation for this invocation.
  StrCpy $INSTDIR "$EXEDIR\mapped-root"
  StrCpy $ZC_SMOKE_ROOT $INSTDIR
  SetOutPath $INSTDIR
  ClearErrors
  CreateDirectory "$ZC_SMOKE_ROOT\native"
  IfErrors smoke_mapped_directory_failure
  ClearErrors
  File /a "/oname=native\mapped-seed.dll" "${ZC_SMOKE_SOURCE}"
  IfErrors smoke_mapped_seed_failure
  ClearErrors
  Rename "$ZC_SMOKE_ROOT\native\mapped-seed.dll" "$ZC_SMOKE_ROOT\native\fylura_windows_preview_handler.dll"
  IfErrors smoke_mapped_seed_failure
  IfFileExists "$ZC_SMOKE_ROOT\native\fylura_windows_preview_handler.dll" 0 smoke_mapped_seed_failure

  ; The NSIS toolchain is a 32-bit stub while the production Preview DLL is
  ; x64. Hold the exact x64 file with a movable, no-write-sharing handle so
  ; the servicing probe sees sharing violation while MoveFileEx can retire it.
  System::Call 'kernel32::CreateFileW(w "$ZC_SMOKE_ROOT\native\fylura_windows_preview_handler.dll", i 0x80000000, i 5, p 0, i 3, i 0x00000080, p 0) p.r1 ?e'
  StrCpy $ZC_SMOKE_OLD_HANDLE $1
  ${IntPtrCmp} $ZC_SMOKE_OLD_HANDLE -1 smoke_mapped_handle_failure smoke_mapped_handle_ready smoke_mapped_handle_ready

smoke_mapped_handle_ready:
  ${If} $ZC_SMOKE_OLD_HANDLE == 0
    StrCpy $ZC_SMOKE_FAILED 1
    FileWrite $ZC_SMOKE_LOG "mapped: old Preview DLL could not be held$\r$\n"
    Goto smoke_mapped_release
  ${EndIf}

  Call ZCResetPreviewDllMutationState
  !insertmacro ZC_INSTALL_RESOURCE "native\fylura_windows_preview_handler.dll" "${ZC_SMOKE_SOURCE}"
  !insertmacro ZC_SMOKE_ASSERT_PATHS "mapped"
  System::Call 'kernel32::CreateFileW(w "$ZC_SMOKE_ROOT\native\fylura_windows_preview_handler.dll", i 0x80000000, i 7, p 0, i 3, i 0x00000080, p 0) p.r2 ?e'
  StrCpy $ZC_SMOKE_REPLACEMENT_HANDLE $2
  ${IntPtrCmp} $ZC_SMOKE_REPLACEMENT_HANDLE -1 smoke_mapped_replacement_handle_failure smoke_mapped_replacement_handle_ready smoke_mapped_replacement_handle_ready

smoke_mapped_replacement_handle_ready:
  ${If} $ZC_SMOKE_REPLACEMENT_HANDLE == 0
    StrCpy $ZC_SMOKE_FAILED 1
    FileWrite $ZC_SMOKE_LOG "mapped: replacement Preview DLL could not be opened$\r$\n"
  ${EndIf}

smoke_mapped_release:
  ${If} $ZC_SMOKE_REPLACEMENT_HANDLE != 0
    System::Call 'kernel32::CloseHandle(p $ZC_SMOKE_REPLACEMENT_HANDLE) i.r1'
    StrCpy $ZC_SMOKE_REPLACEMENT_HANDLE 0
  ${EndIf}
  ${If} $ZC_SMOKE_OLD_HANDLE != 0
    System::Call 'kernel32::CloseHandle(p $ZC_SMOKE_OLD_HANDLE) i.r1'
    StrCpy $ZC_SMOKE_OLD_HANDLE 0
  ${EndIf}
  Call ZCFinalizePreviewDllMutation
  Goto smoke_finish

smoke_mapped_handle_failure:
  StrCpy $ZC_SMOKE_FAILED 1
  FileWrite $ZC_SMOKE_LOG "mapped: old Preview DLL handle could not be opened$\r$\n"
  Goto smoke_mapped_release

smoke_mapped_replacement_handle_failure:
  StrCpy $ZC_SMOKE_FAILED 1
  FileWrite $ZC_SMOKE_LOG "mapped: replacement Preview DLL handle could not be opened$\r$\n"
  Goto smoke_mapped_release

smoke_mapped_directory_failure:
  StrCpy $ZC_SMOKE_FAILED 1
  FileWrite $ZC_SMOKE_LOG "mapped: native parent directory could not be created$\r$\n"
  Goto smoke_finish

smoke_mapped_seed_failure:
  StrCpy $ZC_SMOKE_FAILED 1
  FileWrite $ZC_SMOKE_LOG "mapped: initial canonical Preview DLL could not be seeded$\r$\n"
  Goto smoke_finish

smoke_finish:
  FileClose $ZC_SMOKE_LOG
  ${If} $ZC_SMOKE_FAILED == 0
    SetErrorLevel 0
  ${EndIf}
  Goto smoke_section_end

zc_install_partial_failure:
  SetErrorLevel 2
  FileClose $ZC_SMOKE_LOG
  Abort

zc_uninstall_partial_failure:
  SetErrorLevel 2
  FileClose $ZC_SMOKE_LOG
  Abort

smoke_section_end:
SectionEnd
