; Narrow Win32 registry authority primitives shared by the production NSIS
; lifecycle and the executable HKCU semantic smoke. Product identities and
; lifecycle decisions intentionally remain in installer-hooks.nsh.

!ifndef ZC_REGISTRY_AUTHORITY_NSH
!define ZC_REGISTRY_AUTHORITY_NSH

!include "LogicLib.nsh"

; Predefined HKEY values are signed pointer-sized pseudo-handles. Decimal
; negatives ensure System::Call sign-extends them in a 64-bit installer.
!define ZC_REG_ROOT_HKCU -2147483647
!define ZC_REG_ROOT_HKLM -2147483646

!define ZC_REG_ERROR_SUCCESS 0
!define ZC_REG_ERROR_FILE_NOT_FOUND 2
!define ZC_REG_ERROR_PATH_NOT_FOUND 3
!define ZC_REG_ERROR_NO_MORE_ITEMS 259

!define ZC_REG_KEY_QUERY_VALUE 0x0001
!define ZC_REG_KEY_SET_VALUE_WOW64_64KEY 0x0102
!define ZC_REG_KEY_SET_VALUE_WOW64_32KEY 0x0202
!define ZC_REG_KEY_ENUMERATE_SUB_KEYS 0x0008
!define ZC_REG_KEY_WOW64_64KEY 0x0100
!define ZC_REG_KEY_READ_64 0x0109

!define ZC_REG_TYPE_SZ 1
!define ZC_REG_TYPE_EXPAND_SZ 2
!define ZC_REG_TYPE_DWORD 4

!define ZC_REG_STRING_SZ_ONLY 1
!define ZC_REG_STRING_EXPAND_SZ_ONLY 2
!define ZC_REG_STRING_SZ_OR_EXPAND_SZ 3

!define ZC_REG_KEY_ABSENT 0
!define ZC_REG_KEY_PRESENT 1
!define ZC_REG_KEY_UNKNOWN 2

!define ZC_REG_VALUE_ABSENT 0
!define ZC_REG_VALUE_EXACT 1
!define ZC_REG_VALUE_FOREIGN 2
!define ZC_REG_VALUE_UNKNOWN 3

!define ZC_REG_ENUM_ITEM 0
!define ZC_REG_ENUM_END 1
!define ZC_REG_ENUM_UNKNOWN 2

Var ZC_REG_KEY_STATE
Var ZC_REG_VALUE_STATE
Var ZC_REG_ENUM_STATE
Var ZC_REG_ENUM_NAME
Var ZC_REG_VALUE_DATA
Var ZC_REG_VALUE_TYPE
Var ZC_REG_VALUE_SIZE
Var ZC_REG_EXPECTED_SIZE
Var ZC_REG_HANDLE
Var ZC_REG_RESULT
Var ZC_REG_CLOSE_RESULT
Var ZC_REG_ENUM_NAME_LENGTH
Var ZC_REG_DELETE_VALUE_CLEAN

; Delete one exact value in an explicit registry view. This avoids depending on
; NSIS's mutable SetRegView state for user startup entries.
!macro ZC_REG_DELETE_VALUE_VIEW ROOT PATH NAME ACCESS
  StrCpy $ZC_REG_DELETE_VALUE_CLEAN 0
  StrCpy $ZC_REG_HANDLE 0
  System::Call 'advapi32::RegOpenKeyExW(p ${ROOT}, w "${PATH}", i 0, i ${ACCESS}, *p .r8) i.r9'
  StrCpy $ZC_REG_RESULT $9
  ${If} $ZC_REG_RESULT == ${ZC_REG_ERROR_SUCCESS}
    StrCpy $ZC_REG_HANDLE $8
    System::Call 'advapi32::RegDeleteValueW(p r8, w "${NAME}") i.r9'
    StrCpy $ZC_REG_RESULT $9
    ${If} $ZC_REG_RESULT == ${ZC_REG_ERROR_SUCCESS}
    ${OrIf} $ZC_REG_RESULT == ${ZC_REG_ERROR_FILE_NOT_FOUND}
      StrCpy $ZC_REG_DELETE_VALUE_CLEAN 1
    ${EndIf}
    System::Call 'advapi32::RegCloseKey(p r8) i.r9'
    StrCpy $ZC_REG_CLOSE_RESULT $9
    ${If} $ZC_REG_CLOSE_RESULT != ${ZC_REG_ERROR_SUCCESS}
      StrCpy $ZC_REG_DELETE_VALUE_CLEAN 0
    ${EndIf}
  ${ElseIf} $ZC_REG_RESULT == ${ZC_REG_ERROR_FILE_NOT_FOUND}
  ${OrIf} $ZC_REG_RESULT == ${ZC_REG_ERROR_PATH_NOT_FOUND}
    StrCpy $ZC_REG_DELETE_VALUE_CLEAN 1
  ${EndIf}
!macroend

; ROOT is a predefined HKEY constant and PATH is the exact subkey path.
!macro ZC_REG_QUERY_KEY_STATE ROOT PATH
  StrCpy $ZC_REG_KEY_STATE ${ZC_REG_KEY_UNKNOWN}
  StrCpy $ZC_REG_HANDLE 0
  System::Call 'advapi32::RegOpenKeyExW(p ${ROOT}, w "${PATH}", i 0, i ${ZC_REG_KEY_READ_64}, *p .r8) i.r9'
  StrCpy $ZC_REG_HANDLE $8
  StrCpy $ZC_REG_RESULT $9
  ${If} $ZC_REG_RESULT == ${ZC_REG_ERROR_SUCCESS}
    StrCpy $ZC_REG_KEY_STATE ${ZC_REG_KEY_PRESENT}
    System::Call 'advapi32::RegCloseKey(p r8) i.r9'
    StrCpy $ZC_REG_CLOSE_RESULT $9
    ${If} $ZC_REG_CLOSE_RESULT != ${ZC_REG_ERROR_SUCCESS}
      StrCpy $ZC_REG_KEY_STATE ${ZC_REG_KEY_UNKNOWN}
    ${EndIf}
  ${ElseIf} $ZC_REG_RESULT == ${ZC_REG_ERROR_FILE_NOT_FOUND}
    StrCpy $ZC_REG_KEY_STATE ${ZC_REG_KEY_ABSENT}
  ${ElseIf} $ZC_REG_RESULT == ${ZC_REG_ERROR_PATH_NOT_FOUND}
    StrCpy $ZC_REG_KEY_STATE ${ZC_REG_KEY_ABSENT}
  ${EndIf}
!macroend

; Query an exact raw string. MODE selects REG_SZ, REG_EXPAND_SZ, or either.
; A present wrong type/size/value is FOREIGN. API uncertainty is UNKNOWN.
!macro ZC_REG_QUERY_STRING_STATE ROOT PATH NAME EXPECTED MODE
  !insertmacro ZC_REG_QUERY_STRING_STATE_IMPL ${ROOT} "${PATH}" "${NAME}" "${EXPECTED}" ${MODE} ${__COUNTER__}
!macroend

!macro ZC_REG_QUERY_STRING_STATE_IMPL ROOT PATH NAME EXPECTED MODE ID
  StrCpy $ZC_REG_VALUE_STATE ${ZC_REG_VALUE_UNKNOWN}
  StrCpy $ZC_REG_VALUE_DATA ""
  StrCpy $ZC_REG_VALUE_TYPE 0
  StrCpy $ZC_REG_VALUE_SIZE 0
  StrCpy $ZC_REG_HANDLE 0
  System::Call 'advapi32::RegOpenKeyExW(p ${ROOT}, w "${PATH}", i 0, i ${ZC_REG_KEY_READ_64}, *p .r8) i.r9'
  StrCpy $ZC_REG_HANDLE $8
  StrCpy $ZC_REG_RESULT $9
  ${If} $ZC_REG_RESULT == ${ZC_REG_ERROR_FILE_NOT_FOUND}
    StrCpy $ZC_REG_VALUE_STATE ${ZC_REG_VALUE_ABSENT}
    Goto zc_reg_string_done_${ID}
  ${ElseIf} $ZC_REG_RESULT == ${ZC_REG_ERROR_PATH_NOT_FOUND}
    StrCpy $ZC_REG_VALUE_STATE ${ZC_REG_VALUE_ABSENT}
    Goto zc_reg_string_done_${ID}
  ${ElseIf} $ZC_REG_RESULT != ${ZC_REG_ERROR_SUCCESS}
    Goto zc_reg_string_done_${ID}
  ${EndIf}

  System::Call 'advapi32::RegQueryValueExW(p r8, w "${NAME}", p 0, *i .r7, p 0, *i .r6) i.r9'
  StrCpy $ZC_REG_VALUE_TYPE $7
  StrCpy $ZC_REG_VALUE_SIZE $6
  StrCpy $ZC_REG_RESULT $9
  ${If} $ZC_REG_RESULT == ${ZC_REG_ERROR_FILE_NOT_FOUND}
    StrCpy $ZC_REG_VALUE_STATE ${ZC_REG_VALUE_ABSENT}
    Goto zc_reg_string_close_${ID}
  ${ElseIf} $ZC_REG_RESULT != ${ZC_REG_ERROR_SUCCESS}
    Goto zc_reg_string_close_${ID}
  ${EndIf}

  ${If} ${MODE} == ${ZC_REG_STRING_SZ_ONLY}
    ${If} $ZC_REG_VALUE_TYPE != ${ZC_REG_TYPE_SZ}
      StrCpy $ZC_REG_VALUE_STATE ${ZC_REG_VALUE_FOREIGN}
      Goto zc_reg_string_close_${ID}
    ${EndIf}
  ${ElseIf} ${MODE} == ${ZC_REG_STRING_EXPAND_SZ_ONLY}
    ${If} $ZC_REG_VALUE_TYPE != ${ZC_REG_TYPE_EXPAND_SZ}
      StrCpy $ZC_REG_VALUE_STATE ${ZC_REG_VALUE_FOREIGN}
      Goto zc_reg_string_close_${ID}
    ${EndIf}
  ${Else}
    ${If} $ZC_REG_VALUE_TYPE != ${ZC_REG_TYPE_SZ}
    ${AndIf} $ZC_REG_VALUE_TYPE != ${ZC_REG_TYPE_EXPAND_SZ}
      StrCpy $ZC_REG_VALUE_STATE ${ZC_REG_VALUE_FOREIGN}
      Goto zc_reg_string_close_${ID}
    ${EndIf}
  ${EndIf}

  StrLen $ZC_REG_EXPECTED_SIZE "${EXPECTED}"
  IntOp $ZC_REG_EXPECTED_SIZE $ZC_REG_EXPECTED_SIZE + 1
  IntOp $ZC_REG_EXPECTED_SIZE $ZC_REG_EXPECTED_SIZE * 2
  ${If} $ZC_REG_VALUE_SIZE != $ZC_REG_EXPECTED_SIZE
    StrCpy $ZC_REG_VALUE_STATE ${ZC_REG_VALUE_FOREIGN}
    Goto zc_reg_string_close_${ID}
  ${EndIf}
  ; RegQueryValueExW can return unterminated strings. Exact byte size plus an
  ; exact decoded comparison prevents a truncated or embedded-NUL value from
  ; becoming current authority.
  StrCpy $6 $ZC_REG_VALUE_SIZE
  System::Call 'advapi32::RegQueryValueExW(p r8, w "${NAME}", p 0, *i .r7, w .r5, *i r6) i.r9'
  StrCpy $ZC_REG_VALUE_TYPE $7
  StrCpy $ZC_REG_VALUE_DATA $5
  StrCpy $ZC_REG_VALUE_SIZE $6
  StrCpy $ZC_REG_RESULT $9
  ${If} $ZC_REG_RESULT != ${ZC_REG_ERROR_SUCCESS}
    StrCpy $ZC_REG_VALUE_STATE ${ZC_REG_VALUE_UNKNOWN}
    Goto zc_reg_string_close_${ID}
  ${EndIf}
  ${If} ${MODE} == ${ZC_REG_STRING_SZ_ONLY}
    ${If} $ZC_REG_VALUE_TYPE != ${ZC_REG_TYPE_SZ}
      StrCpy $ZC_REG_VALUE_STATE ${ZC_REG_VALUE_FOREIGN}
      Goto zc_reg_string_close_${ID}
    ${EndIf}
  ${ElseIf} ${MODE} == ${ZC_REG_STRING_EXPAND_SZ_ONLY}
    ${If} $ZC_REG_VALUE_TYPE != ${ZC_REG_TYPE_EXPAND_SZ}
      StrCpy $ZC_REG_VALUE_STATE ${ZC_REG_VALUE_FOREIGN}
      Goto zc_reg_string_close_${ID}
    ${EndIf}
  ${Else}
    ${If} $ZC_REG_VALUE_TYPE != ${ZC_REG_TYPE_SZ}
    ${AndIf} $ZC_REG_VALUE_TYPE != ${ZC_REG_TYPE_EXPAND_SZ}
      StrCpy $ZC_REG_VALUE_STATE ${ZC_REG_VALUE_FOREIGN}
      Goto zc_reg_string_close_${ID}
    ${EndIf}
  ${EndIf}
  ${If} $ZC_REG_VALUE_SIZE != $ZC_REG_EXPECTED_SIZE
    StrCpy $ZC_REG_VALUE_STATE ${ZC_REG_VALUE_FOREIGN}
  ${ElseIf} $ZC_REG_VALUE_DATA == "${EXPECTED}"
    StrCpy $ZC_REG_VALUE_STATE ${ZC_REG_VALUE_EXACT}
  ${Else}
    StrCpy $ZC_REG_VALUE_STATE ${ZC_REG_VALUE_FOREIGN}
  ${EndIf}

zc_reg_string_close_${ID}:
  System::Call 'advapi32::RegCloseKey(p r8) i.r9'
  StrCpy $ZC_REG_CLOSE_RESULT $9
  ${If} $ZC_REG_CLOSE_RESULT != ${ZC_REG_ERROR_SUCCESS}
    StrCpy $ZC_REG_VALUE_STATE ${ZC_REG_VALUE_UNKNOWN}
  ${EndIf}
zc_reg_string_done_${ID}:
!macroend

!macro ZC_REG_QUERY_DWORD_STATE ROOT PATH NAME EXPECTED
  !insertmacro ZC_REG_QUERY_DWORD_STATE_IMPL ${ROOT} "${PATH}" "${NAME}" ${EXPECTED} ${__COUNTER__}
!macroend

!macro ZC_REG_QUERY_DWORD_STATE_IMPL ROOT PATH NAME EXPECTED ID
  StrCpy $ZC_REG_VALUE_STATE ${ZC_REG_VALUE_UNKNOWN}
  StrCpy $ZC_REG_VALUE_DATA 0
  StrCpy $ZC_REG_VALUE_TYPE 0
  StrCpy $ZC_REG_VALUE_SIZE 4
  StrCpy $ZC_REG_HANDLE 0
  System::Call 'advapi32::RegOpenKeyExW(p ${ROOT}, w "${PATH}", i 0, i ${ZC_REG_KEY_READ_64}, *p .r8) i.r9'
  StrCpy $ZC_REG_HANDLE $8
  StrCpy $ZC_REG_RESULT $9
  ${If} $ZC_REG_RESULT == ${ZC_REG_ERROR_FILE_NOT_FOUND}
    StrCpy $ZC_REG_VALUE_STATE ${ZC_REG_VALUE_ABSENT}
    Goto zc_reg_dword_done_${ID}
  ${ElseIf} $ZC_REG_RESULT == ${ZC_REG_ERROR_PATH_NOT_FOUND}
    StrCpy $ZC_REG_VALUE_STATE ${ZC_REG_VALUE_ABSENT}
    Goto zc_reg_dword_done_${ID}
  ${ElseIf} $ZC_REG_RESULT != ${ZC_REG_ERROR_SUCCESS}
    Goto zc_reg_dword_done_${ID}
  ${EndIf}

  StrCpy $6 4
  System::Call 'advapi32::RegQueryValueExW(p r8, w "${NAME}", p 0, *i .r7, *i .r5, *i r6) i.r9'
  StrCpy $ZC_REG_VALUE_TYPE $7
  StrCpy $ZC_REG_VALUE_DATA $5
  StrCpy $ZC_REG_VALUE_SIZE $6
  StrCpy $ZC_REG_RESULT $9
  ${If} $ZC_REG_RESULT == ${ZC_REG_ERROR_FILE_NOT_FOUND}
    StrCpy $ZC_REG_VALUE_STATE ${ZC_REG_VALUE_ABSENT}
  ${ElseIf} $ZC_REG_RESULT != ${ZC_REG_ERROR_SUCCESS}
    StrCpy $ZC_REG_VALUE_STATE ${ZC_REG_VALUE_UNKNOWN}
  ${ElseIf} $ZC_REG_VALUE_TYPE != ${ZC_REG_TYPE_DWORD}
    StrCpy $ZC_REG_VALUE_STATE ${ZC_REG_VALUE_FOREIGN}
  ${ElseIf} $ZC_REG_VALUE_SIZE != 4
    StrCpy $ZC_REG_VALUE_STATE ${ZC_REG_VALUE_FOREIGN}
  ${ElseIf} $ZC_REG_VALUE_DATA == ${EXPECTED}
    StrCpy $ZC_REG_VALUE_STATE ${ZC_REG_VALUE_EXACT}
  ${Else}
    StrCpy $ZC_REG_VALUE_STATE ${ZC_REG_VALUE_FOREIGN}
  ${EndIf}
  System::Call 'advapi32::RegCloseKey(p r8) i.r9'
  StrCpy $ZC_REG_CLOSE_RESULT $9
  ${If} $ZC_REG_CLOSE_RESULT != ${ZC_REG_ERROR_SUCCESS}
    StrCpy $ZC_REG_VALUE_STATE ${ZC_REG_VALUE_UNKNOWN}
  ${EndIf}
zc_reg_dword_done_${ID}:
!macroend

; Enumerators distinguish a real finite END (ERROR_NO_MORE_ITEMS) from every
; other non-success return. An absent exact key is also a complete empty set.
!macro ZC_REG_ENUM_KEY_STATE ROOT PATH INDEX
  !insertmacro ZC_REG_ENUM_KEY_STATE_IMPL ${ROOT} "${PATH}" ${INDEX} ${__COUNTER__}
!macroend

!macro ZC_REG_ENUM_KEY_STATE_IMPL ROOT PATH INDEX ID
  StrCpy $ZC_REG_ENUM_STATE ${ZC_REG_ENUM_UNKNOWN}
  StrCpy $ZC_REG_ENUM_NAME ""
  StrCpy $ZC_REG_HANDLE 0
  System::Call 'advapi32::RegOpenKeyExW(p ${ROOT}, w "${PATH}", i 0, i ${ZC_REG_KEY_READ_64}, *p .r8) i.r9'
  StrCpy $ZC_REG_HANDLE $8
  StrCpy $ZC_REG_RESULT $9
  ${If} $ZC_REG_RESULT == ${ZC_REG_ERROR_FILE_NOT_FOUND}
    StrCpy $ZC_REG_ENUM_STATE ${ZC_REG_ENUM_END}
    Goto zc_reg_enum_key_done_${ID}
  ${ElseIf} $ZC_REG_RESULT == ${ZC_REG_ERROR_PATH_NOT_FOUND}
    StrCpy $ZC_REG_ENUM_STATE ${ZC_REG_ENUM_END}
    Goto zc_reg_enum_key_done_${ID}
  ${ElseIf} $ZC_REG_RESULT != ${ZC_REG_ERROR_SUCCESS}
    Goto zc_reg_enum_key_done_${ID}
  ${EndIf}
  StrCpy $ZC_REG_ENUM_NAME_LENGTH ${NSIS_MAX_STRLEN}
  StrCpy $6 $ZC_REG_ENUM_NAME_LENGTH
  System::Call 'advapi32::RegEnumKeyExW(p r8, i ${INDEX}, w .r5, *i r6, p 0, p 0, p 0, p 0) i.r9'
  StrCpy $ZC_REG_ENUM_NAME $5
  StrCpy $ZC_REG_ENUM_NAME_LENGTH $6
  StrCpy $ZC_REG_RESULT $9
  ${If} $ZC_REG_RESULT == ${ZC_REG_ERROR_SUCCESS}
    StrCpy $ZC_REG_ENUM_STATE ${ZC_REG_ENUM_ITEM}
  ${ElseIf} $ZC_REG_RESULT == ${ZC_REG_ERROR_NO_MORE_ITEMS}
    StrCpy $ZC_REG_ENUM_STATE ${ZC_REG_ENUM_END}
  ${EndIf}
  System::Call 'advapi32::RegCloseKey(p r8) i.r9'
  StrCpy $ZC_REG_CLOSE_RESULT $9
  ${If} $ZC_REG_CLOSE_RESULT != ${ZC_REG_ERROR_SUCCESS}
    StrCpy $ZC_REG_ENUM_STATE ${ZC_REG_ENUM_UNKNOWN}
  ${EndIf}
zc_reg_enum_key_done_${ID}:
!macroend

!macro ZC_REG_ENUM_VALUE_STATE ROOT PATH INDEX
  !insertmacro ZC_REG_ENUM_VALUE_STATE_IMPL ${ROOT} "${PATH}" ${INDEX} ${__COUNTER__}
!macroend

!macro ZC_REG_ENUM_VALUE_STATE_IMPL ROOT PATH INDEX ID
  StrCpy $ZC_REG_ENUM_STATE ${ZC_REG_ENUM_UNKNOWN}
  StrCpy $ZC_REG_ENUM_NAME ""
  StrCpy $ZC_REG_VALUE_TYPE 0
  StrCpy $ZC_REG_HANDLE 0
  System::Call 'advapi32::RegOpenKeyExW(p ${ROOT}, w "${PATH}", i 0, i ${ZC_REG_KEY_READ_64}, *p .r8) i.r9'
  StrCpy $ZC_REG_HANDLE $8
  StrCpy $ZC_REG_RESULT $9
  ${If} $ZC_REG_RESULT == ${ZC_REG_ERROR_FILE_NOT_FOUND}
    StrCpy $ZC_REG_ENUM_STATE ${ZC_REG_ENUM_END}
    Goto zc_reg_enum_value_done_${ID}
  ${ElseIf} $ZC_REG_RESULT == ${ZC_REG_ERROR_PATH_NOT_FOUND}
    StrCpy $ZC_REG_ENUM_STATE ${ZC_REG_ENUM_END}
    Goto zc_reg_enum_value_done_${ID}
  ${ElseIf} $ZC_REG_RESULT != ${ZC_REG_ERROR_SUCCESS}
    Goto zc_reg_enum_value_done_${ID}
  ${EndIf}
  StrCpy $ZC_REG_ENUM_NAME_LENGTH ${NSIS_MAX_STRLEN}
  StrCpy $6 $ZC_REG_ENUM_NAME_LENGTH
  System::Call 'advapi32::RegEnumValueW(p r8, i ${INDEX}, w .r5, *i r6, p 0, *i .r7, p 0, p 0) i.r9'
  StrCpy $ZC_REG_ENUM_NAME $5
  StrCpy $ZC_REG_ENUM_NAME_LENGTH $6
  StrCpy $ZC_REG_VALUE_TYPE $7
  StrCpy $ZC_REG_RESULT $9
  ${If} $ZC_REG_RESULT == ${ZC_REG_ERROR_SUCCESS}
    StrCpy $ZC_REG_ENUM_STATE ${ZC_REG_ENUM_ITEM}
  ${ElseIf} $ZC_REG_RESULT == ${ZC_REG_ERROR_NO_MORE_ITEMS}
    StrCpy $ZC_REG_ENUM_STATE ${ZC_REG_ENUM_END}
  ${EndIf}
  System::Call 'advapi32::RegCloseKey(p r8) i.r9'
  StrCpy $ZC_REG_CLOSE_RESULT $9
  ${If} $ZC_REG_CLOSE_RESULT != ${ZC_REG_ERROR_SUCCESS}
    StrCpy $ZC_REG_ENUM_STATE ${ZC_REG_ENUM_UNKNOWN}
  ${EndIf}
zc_reg_enum_value_done_${ID}:
!macroend

; Test seam for executable proof of an actual Win32 error. Production callers
; use only the exact-key enumeration macros above.
!macro ZC_REG_ENUM_VALUE_INVALID_HANDLE_STATE
  StrCpy $ZC_REG_ENUM_STATE ${ZC_REG_ENUM_UNKNOWN}
  StrCpy $ZC_REG_ENUM_NAME ""
  StrCpy $ZC_REG_ENUM_NAME_LENGTH ${NSIS_MAX_STRLEN}
  StrCpy $6 $ZC_REG_ENUM_NAME_LENGTH
  System::Call 'advapi32::RegEnumValueW(p -1, i 0, w .r5, *i r6, p 0, *i .r7, p 0, p 0) i.r9'
  StrCpy $ZC_REG_ENUM_NAME $5
  StrCpy $ZC_REG_ENUM_NAME_LENGTH $6
  StrCpy $ZC_REG_VALUE_TYPE $7
  StrCpy $ZC_REG_RESULT $9
  ${If} $ZC_REG_RESULT == ${ZC_REG_ERROR_SUCCESS}
    StrCpy $ZC_REG_ENUM_STATE ${ZC_REG_ENUM_ITEM}
  ${ElseIf} $ZC_REG_RESULT == ${ZC_REG_ERROR_NO_MORE_ITEMS}
    StrCpy $ZC_REG_ENUM_STATE ${ZC_REG_ENUM_END}
  ${EndIf}
!macroend

!endif
