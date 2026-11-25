Unicode true
RequestExecutionLevel user

!include "MUI2.nsh"
!include "FileFunc.nsh"

!ifndef NDM_BIN_DIR
  !error "NDM_BIN_DIR must contain ndm-desktop.exe and ndm-host.exe"
!endif
!ifndef NDM_OUTPUT
  !error "NDM_OUTPUT must be the output installer path"
!endif
!ifndef NDM_NOTICES
  !error "NDM_NOTICES must be the generated third-party licenses HTML"
!endif
!ifndef NDM_FFMPEG_DIR
  !error "NDM_FFMPEG_DIR must contain the audited FFmpeg build and corresponding source"
!endif
!ifndef NDM_VERSION
  !define NDM_VERSION "0.1.0"
!endif
!ifndef NDM_VERSION_QUAD
  !define NDM_VERSION_QUAD "0.1.0.0"
!endif
!ifndef NDM_HOST_NAME
  !define NDM_HOST_NAME "com.ndmnext.host"
!endif
!ifndef NDM_UNINSTALL_KEY
  !define NDM_UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\NDM Next"
!endif
!ifndef NDM_CHROME_ID
  !define NDM_CHROME_ID "caegebfdojcdlkkiaiokeoadefdooamd"
!endif
!define NDM_UNPACKED_ID "cpapmfpgmpbpnjgibdgkfkdeblamiodm"
!ifndef NDM_EDGE_ID
  !ifndef NDM_TEST_INSTALL
    !define NDM_EDGE_ID "hiblommiheggidolemmnnlocmocebckp"
  !endif
!endif
!ifdef NDM_TEST_DELETE_DATA
  !ifndef NDM_TEST_INSTALL
    !error "Automatic data removal is only allowed in isolated installer tests"
  !endif
!endif
!ifdef NDM_TEST_DATA_DIR
  !ifndef NDM_TEST_INSTALL
    !error "A custom data directory is only allowed in isolated installer tests"
  !endif
  !define NDM_DATA_DIR "${NDM_TEST_DATA_DIR}"
!else
  !define NDM_DATA_DIR "$LOCALAPPDATA\NDM Next"
!endif

Name "OnesDM"
OutFile "${NDM_OUTPUT}"
InstallDir "$LOCALAPPDATA\Programs\OnesDM"
InstallDirRegKey HKCU "${NDM_UNINSTALL_KEY}" "InstallLocation"
Var InstallationGuard
Var OldDesktop
Var UpdateMode
SetCompressor /SOLID lzma
VIProductVersion "${NDM_VERSION_QUAD}"
VIAddVersionKey "ProductName" "OnesDM"
VIAddVersionKey "CompanyName" "OnesDM contributors"
VIAddVersionKey "FileDescription" "OnesDM installer"
VIAddVersionKey "FileVersion" "${NDM_VERSION}"
VIAddVersionKey "ProductVersion" "${NDM_VERSION}"
VIAddVersionKey "LegalCopyright" "OnesDM contributors"
Icon "ndm-next.ico"
UninstallIcon "ndm-next.ico"

!define MUI_ABORTWARNING
!define MUI_ICON "ndm-next.ico"
!define MUI_UNICON "ndm-next.ico"
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_FUNCTION LaunchDesktop
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"
LangString CloseApplication ${LANG_SIMPCHINESE} "OnesDM 未能完成安全退出。请等待下载停止后重试。"
LangString CloseApplication ${LANG_ENGLISH} "OnesDM could not finish shutting down safely. Wait for downloads to stop and retry."
LangString InstallationBusy ${LANG_SIMPCHINESE} "另一个 OnesDM 安装程序或启动过程正在运行，请稍后重试。"
LangString InstallationBusy ${LANG_ENGLISH} "Another OnesDM installation or startup is in progress. Retry when it finishes."
LangString DeleteUserData ${LANG_SIMPCHINESE} "是否同时删除下载记录和设置？已下载的文件不会删除。"
LangString DeleteUserData ${LANG_ENGLISH} "Also delete download history and settings? Downloaded files will be kept."
LangString DeleteDataFailed ${LANG_SIMPCHINESE} "无法完全删除下载记录和设置；请检查数据目录。"
LangString DeleteDataFailed ${LANG_ENGLISH} "Some download history or settings could not be removed; check the data directory."

!macro DeleteDataFile NAME LABEL
  IfFileExists "${NDM_DATA_DIR}\${NAME}" 0 data_skip_${LABEL}
  ClearErrors
  Delete "${NDM_DATA_DIR}\${NAME}"
  IfErrors data_delete_error
  data_skip_${LABEL}:
!macroend

; Object existence is the guard. Hold its handle until every file/registry write
; finishes; do not wait on an existing mutex or replace files alongside its owner.
!macro InstallationFunctions PREFIX
Function ${PREFIX}ReleaseInstallationGuard
  StrCmp $InstallationGuard "" guard_released
  System::Call 'kernel32::CloseHandle(p $InstallationGuard)'
  StrCpy $InstallationGuard ""
  guard_released:
FunctionEnd

Function ${PREFIX}AcquireInstallationGuard
  InitPluginsDir
  File /oname=$PLUGINSDIR\ndm-check.exe "${NDM_BIN_DIR}\ndm-host.exe"
  nsExec::ExecToStack '"$PLUGINSDIR\ndm-check.exe" --installation-namespace --data-dir "${NDM_DATA_DIR}"'
  Pop $0
  Pop $1
  StrCmp $0 "0" 0 guard_failed
  StrCmp $1 "" guard_failed
  System::Call 'kernel32::CreateMutexW(p 0, i 0, w r1) p .r0 ?e'
  Pop $2
  StrCmp $0 "0" guard_failed
  StrCmp $2 "183" guard_existing
  StrCpy $InstallationGuard $0
  Return
  guard_existing:
  System::Call 'kernel32::CloseHandle(p r0)'
  guard_failed:
  IfSilent guard_abort
  MessageBox MB_OK|MB_ICONEXCLAMATION "$(InstallationBusy)"
  guard_abort:
  SetErrorLevel 2
  Abort
FunctionEnd
!macroend
!insertmacro InstallationFunctions ""
!insertmacro InstallationFunctions "un."

Function .onInit
  SetShellVarContext current
  Call AcquireInstallationGuard
  StrCpy $UpdateMode "0"
  ${GetParameters} $0
  ClearErrors
  ${GetOptions} $0 "/UPDATE" $1
  IfErrors +2
  StrCpy $UpdateMode "1"
  ; Previous installers recorded DisplayIcon even for custom install locations.
  ReadRegStr $OldDesktop HKCU "${NDM_UNINSTALL_KEY}" "DisplayIcon"
  StrCpy $0 $OldDesktop 1
  StrCmp $0 '$\"' old_icon_quoted
  StrCpy $0 $OldDesktop 6 -6
  StrCmp $0 ".exe,0" 0 old_icon_parsed
  StrCpy $OldDesktop $OldDesktop -2
  Goto old_icon_parsed
  old_icon_quoted:
  StrCpy $1 1
  old_icon_quote_loop:
  StrCpy $0 $OldDesktop 1 $1
  StrCmp $0 "" old_icon_invalid
  StrCmp $0 '$\"' old_icon_quote_end
  IntOp $1 $1 + 1
  Goto old_icon_quote_loop
  old_icon_quote_end:
  IntOp $1 $1 - 1
  StrCpy $OldDesktop $OldDesktop $1 1
  Goto old_icon_parsed
  old_icon_invalid:
  StrCpy $OldDesktop ""
  old_icon_parsed:
  ; Preserve a legacy custom location when no different directory was selected.
  StrCmp $INSTDIR "$LOCALAPPDATA\Programs\OnesDM" 0 old_location_done
  StrCmp $OldDesktop "" old_location_done
  IfFileExists "$OldDesktop" 0 old_location_done
  ${GetParent} "$OldDesktop" $INSTDIR
  old_location_done:
FunctionEnd

Function LaunchDesktop
  Call ReleaseInstallationGuard
  Exec '"$INSTDIR\ndm-desktop.exe"'
FunctionEnd
Function .onInstSuccess
  Call ReleaseInstallationGuard
  IfSilent 0 launch_done
  StrCmp $UpdateMode "1" 0 launch_done
  Call LaunchDesktop
  launch_done:
FunctionEnd
Function .onInstFailed
  Call ReleaseInstallationGuard
FunctionEnd
Function .onGUIEnd
  Call ReleaseInstallationGuard
FunctionEnd
Function un.onInit
  SetShellVarContext current
  Call un.AcquireInstallationGuard
FunctionEnd
Function un.onUninstSuccess
  Call un.ReleaseInstallationGuard
FunctionEnd
Function un.onUninstFailed
  Call un.ReleaseInstallationGuard
FunctionEnd
Function un.onGUIEnd
  Call un.ReleaseInstallationGuard
FunctionEnd

Section "Install"
  SetShellVarContext current
  install_check_running:
  StrCmp $OldDesktop "" install_check_target
  StrCmp $OldDesktop "$INSTDIR\ndm-desktop.exe" install_check_target
  nsExec::ExecToStack '"$PLUGINSDIR\ndm-check.exe" --shutdown-desktop "$OldDesktop" --data-dir "${NDM_DATA_DIR}"'
  Pop $0
  Pop $1
  ; If the profile moved, the current target may own its event instead.
  ; Only that specific mismatch permits fallback. Timeout/other errors abort.
  StrCmp $0 "0" install_check_target
  StrCmp $0 "3" install_check_target
  Goto install_shutdown_failed
  install_check_target:
  nsExec::ExecToStack '"$PLUGINSDIR\ndm-check.exe" --shutdown-desktop "$INSTDIR\ndm-desktop.exe" --data-dir "${NDM_DATA_DIR}"'
  Pop $0
  Pop $1
  StrCmp $0 "0" install_not_running
  install_shutdown_failed:
  StrCmp $0 "4" install_shutdown_timeout
  IfSilent install_running_abort
  MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$(CloseApplication)" IDRETRY install_check_running
  install_running_abort:
  Call ReleaseInstallationGuard
  SetErrorLevel 2
  Abort
  install_shutdown_timeout:
  IfSilent install_timeout_abort
  MessageBox MB_OK|MB_ICONEXCLAMATION "$(CloseApplication)"
  install_timeout_abort:
  Call ReleaseInstallationGuard
  SetErrorLevel 4
  Abort
  install_not_running:
  SetOutPath "$INSTDIR"
  File "${NDM_BIN_DIR}\ndm-desktop.exe"
  File "${NDM_BIN_DIR}\ndm-host.exe"
  File /oname=onesdm.exe "${NDM_BIN_DIR}\ndm-host.exe"
  File /oname=LICENSE.txt "..\..\..\LICENSE"
  File /oname=THIRD-PARTY-LICENSES.html "${NDM_NOTICES}"
  File /oname=LICENSE-LUCIDE.txt "licenses\LUCIDE.txt"
  SetOutPath "$INSTDIR\ffmpeg"
  File "${NDM_FFMPEG_DIR}\bin\*.dll"
  File "${NDM_FFMPEG_DIR}\bin\ffmpeg.exe"
  File "${NDM_FFMPEG_DIR}\bin\ffprobe.exe"
  File "${NDM_FFMPEG_DIR}\SHA256SUMS"
  SetOutPath "$INSTDIR\ffmpeg\source"
  File "${NDM_FFMPEG_DIR}\source\*.*"
  SetOutPath "$INSTDIR"
  WriteUninstaller "$INSTDIR\Uninstall.exe"

  ; The host writes UTF-8 JSON, including escaped non-ASCII install paths.
  StrCmp $OldDesktop "" manifest_without_migration
  StrCmp $OldDesktop "$INSTDIR\ndm-desktop.exe" manifest_without_migration
  ; Migration is explicitly tied to the registered previous desktop image.
  !ifdef NDM_EDGE_ID
    nsExec::ExecToStack '"$INSTDIR\ndm-host.exe" --write-manifest "${NDM_HOST_NAME}" --preserve-origins --migrate-origins-from "$OldDesktop" "${NDM_CHROME_ID}" "${NDM_EDGE_ID}" "${NDM_UNPACKED_ID}"'
  !else
    nsExec::ExecToStack '"$INSTDIR\ndm-host.exe" --write-manifest "${NDM_HOST_NAME}" --preserve-origins --migrate-origins-from "$OldDesktop" "${NDM_CHROME_ID}" "${NDM_UNPACKED_ID}"'
  !endif
  Goto manifest_result
  manifest_without_migration:
  !ifdef NDM_EDGE_ID
    nsExec::ExecToStack '"$INSTDIR\ndm-host.exe" --write-manifest "${NDM_HOST_NAME}" --preserve-origins "${NDM_CHROME_ID}" "${NDM_EDGE_ID}" "${NDM_UNPACKED_ID}"'
  !else
    nsExec::ExecToStack '"$INSTDIR\ndm-host.exe" --write-manifest "${NDM_HOST_NAME}" --preserve-origins "${NDM_CHROME_ID}" "${NDM_UNPACKED_ID}"'
  !endif
  manifest_result:
  Pop $0
  Pop $1
  StrCmp $0 "0" 0 install_error

  WriteRegStr HKCU "Software\Google\Chrome\NativeMessagingHosts\${NDM_HOST_NAME}" "" "$INSTDIR\native-messaging-host.json"
  WriteRegStr HKCU "Software\Chromium\NativeMessagingHosts\${NDM_HOST_NAME}" "" "$INSTDIR\native-messaging-host.json"
  WriteRegStr HKCU "Software\Microsoft\Edge\NativeMessagingHosts\${NDM_HOST_NAME}" "" "$INSTDIR\native-messaging-host.json"

  !ifndef NDM_TEST_INSTALL
    Delete "$SMPROGRAMS\NDM Next\NDM Next.lnk"
    Delete "$SMPROGRAMS\NDM Next\Uninstall NDM Next.lnk"
    RMDir "$SMPROGRAMS\NDM Next"
    CreateDirectory "$SMPROGRAMS\OnesDM"
    CreateShortcut "$SMPROGRAMS\OnesDM\OnesDM.lnk" "$INSTDIR\ndm-desktop.exe"
    CreateShortcut "$SMPROGRAMS\OnesDM\Uninstall OnesDM.lnk" "$INSTDIR\Uninstall.exe"
    WriteRegStr HKCU "${NDM_UNINSTALL_KEY}" "DisplayName" "OnesDM"
    WriteRegStr HKCU "${NDM_UNINSTALL_KEY}" "DisplayVersion" "${NDM_VERSION}"
    WriteRegStr HKCU "${NDM_UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\ndm-desktop.exe"
    WriteRegStr HKCU "${NDM_UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
    WriteRegStr HKCU "${NDM_UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
    WriteRegDWORD HKCU "${NDM_UNINSTALL_KEY}" "NoModify" 1
    WriteRegDWORD HKCU "${NDM_UNINSTALL_KEY}" "NoRepair" 1
  !endif
  Goto install_done
install_error:
  Call ReleaseInstallationGuard
  SetErrorLevel 2
  IfSilent +2
  MessageBox MB_ICONSTOP "Could not create the browser integration manifest."
  Abort
install_done:
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  uninstall_check_running:
  nsExec::ExecToStack '"$PLUGINSDIR\ndm-check.exe" --shutdown-desktop "$INSTDIR\ndm-desktop.exe" --data-dir "${NDM_DATA_DIR}"'
  Pop $0
  Pop $1
  StrCmp $0 "0" uninstall_not_running
  StrCmp $0 "4" uninstall_shutdown_timeout
  IfSilent uninstall_running_abort
  MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$(CloseApplication)" IDRETRY uninstall_check_running
  uninstall_running_abort:
  Call un.ReleaseInstallationGuard
  SetErrorLevel 2
  Abort
  uninstall_shutdown_timeout:
  IfSilent uninstall_timeout_abort
  MessageBox MB_OK|MB_ICONEXCLAMATION "$(CloseApplication)"
  uninstall_timeout_abort:
  Call un.ReleaseInstallationGuard
  SetErrorLevel 4
  Abort
  uninstall_not_running:
  ; Do not remove a registry entry if another installation replaced it.
  ReadRegStr $0 HKCU "Software\Google\Chrome\NativeMessagingHosts\${NDM_HOST_NAME}" ""
  StrCmp $0 "$INSTDIR\native-messaging-host.json" 0 +2
  DeleteRegKey HKCU "Software\Google\Chrome\NativeMessagingHosts\${NDM_HOST_NAME}"
  ReadRegStr $0 HKCU "Software\Chromium\NativeMessagingHosts\${NDM_HOST_NAME}" ""
  StrCmp $0 "$INSTDIR\native-messaging-host.json" 0 +2
  DeleteRegKey HKCU "Software\Chromium\NativeMessagingHosts\${NDM_HOST_NAME}"
  ReadRegStr $0 HKCU "Software\Microsoft\Edge\NativeMessagingHosts\${NDM_HOST_NAME}" ""
  StrCmp $0 "$INSTDIR\native-messaging-host.json" 0 +2
  DeleteRegKey HKCU "Software\Microsoft\Edge\NativeMessagingHosts\${NDM_HOST_NAME}"
  ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "NDM Next"
  StrCmp $0 '$\"$INSTDIR\ndm-desktop.exe$\" --background' 0 +2
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "NDM Next"
  !ifndef NDM_TEST_INSTALL
    ReadRegStr $0 HKCU "${NDM_UNINSTALL_KEY}" "UninstallString"
    StrCmp $0 '$\"$INSTDIR\Uninstall.exe$\"' 0 skip_shortcuts
    DeleteRegKey HKCU "${NDM_UNINSTALL_KEY}"
    Delete "$SMPROGRAMS\OnesDM\OnesDM.lnk"
    Delete "$SMPROGRAMS\OnesDM\Uninstall OnesDM.lnk"
    RMDir "$SMPROGRAMS\OnesDM"
  skip_shortcuts:
  !endif
  Delete "$INSTDIR\native-messaging-host.json"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\THIRD-PARTY-LICENSES.html"
  Delete "$INSTDIR\LICENSE-LUCIDE.txt"
  Delete "$INSTDIR\ffmpeg\source\BUILD-INFO.txt"
  Delete "$INSTDIR\ffmpeg\source\build.sh"
  Delete "$INSTDIR\ffmpeg\source\COPYING.LGPLv2.1"
  Delete "$INSTDIR\ffmpeg\source\COPYING.GCC-RUNTIME"
  Delete "$INSTDIR\ffmpeg\source\COPYING.GPL-3"
  Delete "$INSTDIR\ffmpeg\source\LICENSE.md"
  Delete "$INSTDIR\ffmpeg\source\ffmpeg-9.0.2.tar.xz"
  Delete "$INSTDIR\ffmpeg\source\ffmpeg-9.0.2.tar.xz.asc"
  Delete "$INSTDIR\ffmpeg\source\ffmpeg-devel.asc"
  RMDir "$INSTDIR\ffmpeg\source"
  Delete "$INSTDIR\ffmpeg\SHA256SUMS"
  Delete "$INSTDIR\ffmpeg\ffmpeg.exe"
  Delete "$INSTDIR\ffmpeg\ffprobe.exe"
  Delete "$INSTDIR\ffmpeg\avcodec-63.dll"
  Delete "$INSTDIR\ffmpeg\avdevice-63.dll"
  Delete "$INSTDIR\ffmpeg\avfilter-12.dll"
  Delete "$INSTDIR\ffmpeg\avformat-63.dll"
  Delete "$INSTDIR\ffmpeg\avutil-61.dll"
  Delete "$INSTDIR\ffmpeg\swresample-7.dll"
  Delete "$INSTDIR\ffmpeg\swscale-10.dll"
  RMDir "$INSTDIR\ffmpeg"
  !ifdef NDM_TEST_DELETE_DATA
    Goto data_delete_selected
  !else
    IfSilent data_keep
    MessageBox MB_YESNO|MB_ICONQUESTION|MB_DEFBUTTON2 "$(DeleteUserData)" IDNO data_keep
    Goto data_delete_selected
  !endif
  data_delete_selected:
  ; Do not follow a junction or symlink if the data directory was replaced.
  System::Call 'kernel32::GetFileAttributesW(w "${NDM_DATA_DIR}") i .r0'
  StrCmp $0 "-1" data_keep
  IntOp $1 $0 & 0x400
  IntCmp $1 0 data_delete_safe data_delete_error data_delete_error
  data_delete_safe:
  !insertmacro DeleteDataFile "tasks.db" "db"
  !insertmacro DeleteDataFile "tasks.db-wal" "wal"
  !insertmacro DeleteDataFile "tasks.db-shm" "shm"
  !insertmacro DeleteDataFile "preferences.json" "prefs"
  RMDir "${NDM_DATA_DIR}"
  Goto data_keep
  data_delete_error:
  IfSilent data_delete_error_silent
  MessageBox MB_OK|MB_ICONEXCLAMATION "$(DeleteDataFailed)"
  Goto data_keep
  data_delete_error_silent:
  SetErrorLevel 1
  data_keep:
  Delete "$INSTDIR\onesdm.exe"
  Delete "$INSTDIR\ndm-host.exe"
  Delete "$INSTDIR\ndm-desktop.exe"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
SectionEnd
