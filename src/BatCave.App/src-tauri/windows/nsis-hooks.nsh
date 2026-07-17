!include "LogicLib.nsh"

!define BATCAVE_SERVICE_NAME "BatCaveCollector"
!define BATCAVE_SERVICE_REGISTRY_KEY "SYSTEM\CurrentControlSet\Services\${BATCAVE_SERVICE_NAME}"
!define BATCAVE_SERVICE_OWNER_VALUE "BatCaveInstallerOwner"
!define BATCAVE_SERVICE_OWNER_MARKER "dev.batcave.monitor/service-v1"
!define BATCAVE_SERVICE_BINARY "batcave-collector-service.exe"
!define BATCAVE_INSTALL_DIR "$PROGRAMFILES64\BatCave Monitor"

Var BatCaveServiceUpgrade

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

!macro BATCAVE_EXEC_SERVICE_IMAGE_VERB IMAGE VERB DESCRIPTION
  nsExec::ExecToStack `"${IMAGE}" ${VERB}`
  Pop $R0
  Pop $R1
  ${If} $R0 != 0
    DetailPrint "${DESCRIPTION} failed with exit code $R0: $R1"
    SetErrorLevel 1
    Abort
  ${EndIf}
!macroend

!macro BATCAVE_EXEC_SERVICE_VERB VERB DESCRIPTION
  !insertmacro BATCAVE_EXEC_SERVICE_IMAGE_VERB "$INSTDIR\${BATCAVE_SERVICE_BINARY}" "${VERB}" "${DESCRIPTION}"
!macroend

!macro BATCAVE_RETIRE_SHARED_SHORTCUTS IMAGE
  !insertmacro BATCAVE_EXEC_SERVICE_IMAGE_VERB "${IMAGE}" "--provision retire-installer-shortcuts" "Retire legacy shared BatCave shortcuts"
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
  ; Tauri CLI 2.11.4 uses this installer-owned switch for both the Common
  ; Programs and Public Desktop shortcuts. Its Wix migration path bypasses
  ; NoShortcutMode, but the prior Wix uninstall is complete before PREINSTALL,
  ; so clear that shortcut-only bypass as well. BatCave owns neither surface.
  StrCpy $NoShortcutMode 1
  StrCpy $WixMode 0
  !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"
  Call BatCaveAssertServiceOwnedOrMissing
  StrCpy $BatCaveServiceUpgrade $R9
  ${If} $R9 == "1"
    SetOutPath "$INSTDIR"
    File /a "/oname=batcave-collector-service.recovery.exe" "..\..\batcave-collector-service.exe"
    ; Preflight while the prior verified service generation is running. The
    ; provisioner repeats this after recovering any interrupted transaction.
    !insertmacro BATCAVE_RETIRE_SHARED_SHORTCUTS "$INSTDIR\batcave-collector-service.recovery.exe"
    !insertmacro BATCAVE_EXEC_SERVICE_IMAGE_VERB "$INSTDIR\batcave-collector-service.recovery.exe" "--provision prepare-upgrade-staged" "Prepare ${BATCAVE_SERVICE_NAME} upgrade"
    Goto service_upgrade_prepared
service_upgrade_prepared:
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro BATCAVE_REQUIRE_FIXED_INSTALL_DIR
  IfFileExists "$INSTDIR\${BATCAVE_SERVICE_BINARY}" 0 missing_new_service_binary
  ${If} $BatCaveServiceUpgrade == "1"
    IfFileExists "$INSTDIR\batcave-collector-service.recovery.exe" 0 missing_staged_service_binary
    !insertmacro BATCAVE_EXEC_SERVICE_IMAGE_VERB "$INSTDIR\batcave-collector-service.recovery.exe" "--provision commit-upgrade-staged" "Commit ${BATCAVE_SERVICE_NAME} upgrade"
    ; Retain the verified recovery controller through the rollback-coupled
    ; final gate. The stable install verb repeats it before finalization.
    !insertmacro BATCAVE_RETIRE_SHARED_SHORTCUTS "$INSTDIR\batcave-collector-service.recovery.exe"
  ${Else}
    ; A fresh install retires before service creation. The install verb repeats
    ; this gate and rolls a new service back if a link is recreated.
    !insertmacro BATCAVE_RETIRE_SHARED_SHORTCUTS "$INSTDIR\${BATCAVE_SERVICE_BINARY}"
  ${EndIf}
  !insertmacro BATCAVE_EXEC_SERVICE_VERB "--provision install" "Install ${BATCAVE_SERVICE_NAME}"
  Goto service_install_complete

missing_staged_service_binary:
  !insertmacro BATCAVE_ABORT "The staged collector service controller is missing."
missing_new_service_binary:
  !insertmacro BATCAVE_ABORT "The packaged collector service binary is missing."
service_install_complete:
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro BATCAVE_REQUIRE_FIXED_INSTALL_DIR
  ; The native retirement gate owns the only shared-shortcut deletion. Keep the
  ; stock uninstaller from reopening either shared path after that gate.
  StrCpy $NoShortcutMode 1
  !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"
  Call un.BatCaveAssertServiceOwnedOrMissing
  IfFileExists "$INSTDIR\batcave-collector-service.recovery.exe" use_recovery_uninstall_service_binary 0
  IfFileExists "$INSTDIR\batcave-collector-service.${VERSION}.staged.exe" use_legacy_staged_uninstall_service_binary 0
  Goto use_stable_uninstall_service_binary

use_recovery_uninstall_service_binary:
  !insertmacro BATCAVE_RETIRE_SHARED_SHORTCUTS "$INSTDIR\batcave-collector-service.recovery.exe"
  !insertmacro BATCAVE_EXEC_SERVICE_IMAGE_VERB "$INSTDIR\batcave-collector-service.recovery.exe" "--provision uninstall-staged" "Uninstall ${BATCAVE_SERVICE_NAME} through the recovery controller"
  Delete "$INSTDIR\batcave-collector-service.recovery.exe"
  IfFileExists "$INSTDIR\batcave-collector-service.recovery.exe" staged_uninstall_service_binary_present 0
  Goto service_uninstall_complete

use_legacy_staged_uninstall_service_binary:
  !insertmacro BATCAVE_RETIRE_SHARED_SHORTCUTS "$INSTDIR\batcave-collector-service.${VERSION}.staged.exe"
  !insertmacro BATCAVE_EXEC_SERVICE_IMAGE_VERB "$INSTDIR\batcave-collector-service.${VERSION}.staged.exe" "--provision uninstall-staged" "Uninstall ${BATCAVE_SERVICE_NAME} through the compatibility controller"
  Delete "$INSTDIR\batcave-collector-service.${VERSION}.staged.exe"
  IfFileExists "$INSTDIR\batcave-collector-service.${VERSION}.staged.exe" staged_uninstall_service_binary_present 0
  Goto service_uninstall_complete

use_stable_uninstall_service_binary:
  IfFileExists "$INSTDIR\${BATCAVE_SERVICE_BINARY}" 0 missing_uninstall_service_binary
  !insertmacro BATCAVE_RETIRE_SHARED_SHORTCUTS "$INSTDIR\${BATCAVE_SERVICE_BINARY}"
  !insertmacro BATCAVE_EXEC_SERVICE_VERB "--provision uninstall" "Uninstall ${BATCAVE_SERVICE_NAME}"
  Goto service_uninstall_complete

staged_uninstall_service_binary_present:
  !insertmacro BATCAVE_ABORT "The staged collector service controller could not be removed."
missing_uninstall_service_binary:
  !insertmacro BATCAVE_ABORT "The collector service binary is missing; refusing unsafe service cleanup."
service_uninstall_complete:
  DeleteRegKey HKLM "Software\batcave\BatCave Monitor"
  System::Call 'advapi32::RegOpenKeyExW(p 0x80000002, w "Software\batcave\BatCave Monitor", i 0, i 0x20019, *p .R8) i.R7'
  ${If} $R7 == 0
    System::Call 'advapi32::RegCloseKey(p R8)'
    !insertmacro BATCAVE_ABORT "The BatCave product registry key is still present."
  ${ElseIf} $R7 != 2
    !insertmacro BATCAVE_ABORT "The BatCave product registry key removal could not be verified."
  ${EndIf}
  DeleteRegKey /ifempty HKLM "Software\batcave"
!macroend
