!define CLIPAGO_HOOK_DIR "${__FILEDIR__}"

!macro NSIS_HOOK_POSTINSTALL
  Push $0
  InitPluginsDir
  SetOutPath "$PLUGINSDIR"
  File "/oname=install-ffmpeg.ps1" "${CLIPAGO_HOOK_DIR}\install-ffmpeg.ps1"
ffmpeg_retry:
  DetailPrint "Checking FFmpeg and FFprobe. If missing, downloading the required files..."
  nsExec::ExecToLog '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\install-ffmpeg.ps1" -InstallDir "$INSTDIR"'
  Pop $0
  StrCmp $0 "0" ffmpeg_done
  MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "FFmpeg could not be installed. Check your internet connection and retry, or continue and run this installer again later." /SD IDCANCEL IDRETRY ffmpeg_retry
ffmpeg_done:
  SetOutPath "$INSTDIR"
  Pop $0
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode <> 1
    Delete "$INSTDIR\bin\ffmpeg\ffmpeg.exe"
    Delete "$INSTDIR\bin\ffmpeg\ffprobe.exe"
    RMDir "$INSTDIR\bin\ffmpeg"
    RMDir "$INSTDIR\bin"
  ${EndIf}
!macroend
