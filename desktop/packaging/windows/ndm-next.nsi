Unicode true
RequestExecutionLevel user

!include "MUI2.nsh"

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
!ifndef NDM_CHROME_ID
  !define NDM_CHROME_ID "caegebfdojcdlkkiaiokeoadefdooamd"
!endif
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

Name "NDM Next"
OutFile "${NDM_OUTPUT}"
InstallDir "$LOCALAPPDATA\Programs\NDM Next"
SetCompressor /SOLID lzma
VIProductVersion "${NDM_VERSION_QUAD}"
VIAddVersionKey "ProductName" "NDM Next"
VIAddVersionKey "CompanyName" "NDM Next contributors"
VIAddVersionKey "FileDescription" "NDM Next installer"
VIAddVersionKey "FileVersion" "${NDM_VERSION}"
VIAddVersionKey "LegalCopyright" "NDM Next contributors"
Icon "ndm-next.ico"
UninstallIcon "ndm-next.ico"

!define MUI_ABORTWARNING
!define MUI_ICON "ndm-next.ico"
!define MUI_UNICON "ndm-next.ico"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"
LangString CloseApplication ${LANG_SIMPCHINESE} "NDM Next 正在运行，或无法检查其状态。请从托盘退出后重试。"
LangString CloseApplication ${LANG_ENGLISH} "NDM Next is running or its state could not be checked. Quit it from the tray and retry."
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

Section "Install"
  SetShellVarContext current
  InitPluginsDir
  File /oname=$PLUGINSDIR\ndm-check.exe "${NDM_BIN_DIR}\ndm-host.exe"
  install_check_running:
  nsExec::ExecToStack '"$PLUGINSDIR\ndm-check.exe" --desktop-running'
  Pop $0
  Pop $1
  StrCmp $0 "0" install_not_running
  IfSilent install_running_abort
  MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$(CloseApplication)" IDRETRY install_check_running
  install_running_abort:
  Abort
  install_not_running:
  SetOutPath "$INSTDIR"
  File "${NDM_BIN_DIR}\ndm-desktop.exe"
  File "${NDM_BIN_DIR}\ndm-host.exe"
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
  !ifdef NDM_EDGE_ID
    nsExec::ExecToStack '"$INSTDIR\ndm-host.exe" --write-manifest "${NDM_HOST_NAME}" "${NDM_CHROME_ID}" "${NDM_EDGE_ID}"'
  !else
    nsExec::ExecToStack '"$INSTDIR\ndm-host.exe" --write-manifest "${NDM_HOST_NAME}" "${NDM_CHROME_ID}"'
  !endif
  Pop $0
  Pop $1
  StrCmp $0 "0" 0 install_error

  WriteRegStr HKCU "Software\Google\Chrome\NativeMessagingHosts\${NDM_HOST_NAME}" "" "$INSTDIR\native-messaging-host.json"
  WriteRegStr HKCU "Software\Chromium\NativeMessagingHosts\${NDM_HOST_NAME}" "" "$INSTDIR\native-messaging-host.json"
  WriteRegStr HKCU "Software\Microsoft\Edge\NativeMessagingHosts\${NDM_HOST_NAME}" "" "$INSTDIR\native-messaging-host.json"

  !ifndef NDM_TEST_INSTALL
    CreateDirectory "$SMPROGRAMS\NDM Next"
    CreateShortcut "$SMPROGRAMS\NDM Next\NDM Next.lnk" "$INSTDIR\ndm-desktop.exe"
    CreateShortcut "$SMPROGRAMS\NDM Next\Uninstall NDM Next.lnk" "$INSTDIR\Uninstall.exe"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\NDM Next" "DisplayName" "NDM Next"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\NDM Next" "DisplayVersion" "${NDM_VERSION}"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\NDM Next" "DisplayIcon" "$INSTDIR\ndm-desktop.exe"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\NDM Next" "UninstallString" '"$INSTDIR\Uninstall.exe"'
    WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\NDM Next" "NoModify" 1
    WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\NDM Next" "NoRepair" 1
  !endif
  Goto install_done
install_error:
  MessageBox MB_ICONSTOP "Could not create the browser integration manifest."
  Abort
install_done:
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  InitPluginsDir
  File /oname=$PLUGINSDIR\ndm-check.exe "${NDM_BIN_DIR}\ndm-host.exe"
  uninstall_check_running:
  nsExec::ExecToStack '"$PLUGINSDIR\ndm-check.exe" --desktop-running'
  Pop $0
  Pop $1
  StrCmp $0 "0" uninstall_not_running
  IfSilent uninstall_running_abort
  MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$(CloseApplication)" IDRETRY uninstall_check_running
  uninstall_running_abort:
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
    ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\NDM Next" "UninstallString"
    StrCmp $0 '$\"$INSTDIR\Uninstall.exe$\"' 0 skip_shortcuts
    DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\NDM Next"
    Delete "$SMPROGRAMS\NDM Next\NDM Next.lnk"
    Delete "$SMPROGRAMS\NDM Next\Uninstall NDM Next.lnk"
    RMDir "$SMPROGRAMS\NDM Next"
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
  Delete "$INSTDIR\ndm-host.exe"
  Delete "$INSTDIR\ndm-desktop.exe"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
SectionEnd
