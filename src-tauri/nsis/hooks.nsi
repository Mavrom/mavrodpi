!macro customInstall
  CreateShortcut "$DESKTOP\MavroDPI.lnk" "$INSTDIR\MavroDPI.exe" "" "$INSTDIR\MavroDPI.exe" 0
!macroend

!macro customUnInstall
  Delete "$DESKTOP\MavroDPI.lnk"
!macroend
