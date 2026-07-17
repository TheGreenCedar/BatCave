!include "LogicLib.nsh"

!if "$%BATCAVE_UNINSTALLER_EXPORT_PATH%" != ""
  !uninstfinalize '"$%ComSpec%" /D /C copy /Y "%1" "$%BATCAVE_UNINSTALLER_EXPORT_PATH%"' = 0
!endif

!define BATCAVE_SERVICE_NAME "BatCaveCollector"
!define BATCAVE_SERVICE_REGISTRY_KEY "SYSTEM\CurrentControlSet\Services\${BATCAVE_SERVICE_NAME}"
!define BATCAVE_SERVICE_OWNER_VALUE "BatCaveInstallerOwner"
!define BATCAVE_SERVICE_OWNER_MARKER "dev.batcave.monitor/service-v1"
!define BATCAVE_SERVICE_BINARY "batcave-collector-service.exe"
!define BATCAVE_INSTALL_DIR "$PROGRAMFILES64\BatCave Monitor"

!macro BATCAVE_ABORT MESSAGE
  DetailPrint "${MESSAGE}"
  SetErrorLevel 1
  Abort
!macroend

!macro BATCAVE_REQUIRE_FIXED_INSTALL_DIR
  ${If} $INSTDIR != "${BATCAVE_INSTALL_DIR}"
    !insertmacro BATCAVE_ABORT "Collector service changes require the fixed per-machine BatCave install directory."
  ${EndIf}
!macroend

!macro BATCAVE_EXEC_SERVICE_VERB VERB DESCRIPTION
  nsExec::ExecToStack `"$INSTDIR\${BATCAVE_SERVICE_BINARY}" ${VERB}`
  Pop $R0
  Pop $R1
  ${If} $R0 != 0
    DetailPrint "${DESCRIPTION} failed with exit code $R0: $R1"
    SetErrorLevel 1
    Abort
  ${EndIf}
!macroend

!macro BATCAVE_DEFINE_ASSERT_SERVICE_FUNCTION PREFIX
Function ${PREFIX}BatCaveAssertServiceOwnedOrMissing
  StrCpy $R9 "0"
  System::Call 'advapi32::RegOpenKeyExW(p 0x80000002, w "${BATCAVE_SERVICE_REGISTRY_KEY}", i 0, i 0x20019, *p .R8) i.R7'
  ${If} $R7 == 2
    Return
  ${ElseIf} $R7 != 0
    !insertmacro BATCAVE_ABORT "Could not prove whether ${BATCAVE_SERVICE_NAME} already exists."
  ${EndIf}
  System::Call 'advapi32::RegCloseKey(p R8)'

  ClearErrors
  ReadRegDWORD $R0 HKLM "${BATCAVE_SERVICE_REGISTRY_KEY}" "Type"
  ${If} ${Errors}
    !insertmacro BATCAVE_ABORT "Refusing a malformed ${BATCAVE_SERVICE_NAME} service registration."
  ${EndIf}

  ReadRegStr $R1 HKLM "${BATCAVE_SERVICE_REGISTRY_KEY}" "${BATCAVE_SERVICE_OWNER_VALUE}"
  ${If} $R1 != "${BATCAVE_SERVICE_OWNER_MARKER}"
    !insertmacro BATCAVE_ABORT "Refusing to manage an unowned ${BATCAVE_SERVICE_NAME} service."
  ${EndIf}
  ReadRegStr $R1 HKLM "${BATCAVE_SERVICE_REGISTRY_KEY}" "ImagePath"
  ${If} $R1 != `"${BATCAVE_INSTALL_DIR}\${BATCAVE_SERVICE_BINARY}"`
    !insertmacro BATCAVE_ABORT "Refusing to manage ${BATCAVE_SERVICE_NAME} with an unexpected image path."
  ${EndIf}
  ReadRegStr $R1 HKLM "${BATCAVE_SERVICE_REGISTRY_KEY}" "ObjectName"
  ${If} $R1 != "LocalSystem"
    !insertmacro BATCAVE_ABORT "Refusing to manage ${BATCAVE_SERVICE_NAME} with an unexpected account."
  ${EndIf}
  ${If} $R0 != 16
    !insertmacro BATCAVE_ABORT "Refusing to manage ${BATCAVE_SERVICE_NAME} with an unexpected service type."
  ${EndIf}
  StrCpy $R9 "1"
FunctionEnd
!macroend

!insertmacro BATCAVE_DEFINE_ASSERT_SERVICE_FUNCTION ""
!insertmacro BATCAVE_DEFINE_ASSERT_SERVICE_FUNCTION "un."

!macro NSIS_HOOK_PREINSTALL
  !insertmacro BATCAVE_REQUIRE_FIXED_INSTALL_DIR
  !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"
  Call BatCaveAssertServiceOwnedOrMissing
  ${If} $R9 == "1"
    IfFileExists "$INSTDIR\${BATCAVE_SERVICE_BINARY}" 0 missing_owned_service_binary
    !insertmacro BATCAVE_EXEC_SERVICE_VERB "--provision prepare-upgrade" "Prepare ${BATCAVE_SERVICE_NAME} upgrade"
    Goto service_upgrade_prepared

missing_owned_service_binary:
    !insertmacro BATCAVE_ABORT "The owned collector service binary is missing; refusing an unsafe upgrade."
service_upgrade_prepared:
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro BATCAVE_REQUIRE_FIXED_INSTALL_DIR
  IfFileExists "$INSTDIR\${BATCAVE_SERVICE_BINARY}" 0 missing_new_service_binary
  !insertmacro BATCAVE_EXEC_SERVICE_VERB "--provision install" "Install ${BATCAVE_SERVICE_NAME}"
  Goto service_install_complete

missing_new_service_binary:
  !insertmacro BATCAVE_ABORT "The packaged collector service binary is missing."
service_install_complete:
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro BATCAVE_REQUIRE_FIXED_INSTALL_DIR
  !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"
  Call un.BatCaveAssertServiceOwnedOrMissing
  IfFileExists "$INSTDIR\${BATCAVE_SERVICE_BINARY}" 0 missing_uninstall_service_binary
  !insertmacro BATCAVE_EXEC_SERVICE_VERB "--provision uninstall" "Uninstall ${BATCAVE_SERVICE_NAME}"
  Goto service_uninstall_complete

missing_uninstall_service_binary:
  !insertmacro BATCAVE_ABORT "The collector service binary is missing; refusing unsafe service cleanup."
service_uninstall_complete:
!macroend
