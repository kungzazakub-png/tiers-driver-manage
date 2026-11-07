' Hidden launcher: wscript has no console, so nothing flashes (a .cmd shows a cmd window, powershell.exe shows a console for a moment).
' Waits for PowerShell so the one-file setup's temp folder is not deleted early.
Set sh = CreateObject("WScript.Shell")
Set fso = CreateObject("Scripting.FileSystemObject")
d = fso.GetParentFolderName(WScript.ScriptFullName)
ps = sh.ExpandEnvironmentStrings("%windir%") & "\sysnative\WindowsPowerShell\v1.0\powershell.exe"
If Not fso.FileExists(ps) Then ps = sh.ExpandEnvironmentStrings("%windir%") & "\System32\WindowsPowerShell\v1.0\powershell.exe"
a = ""
For Each x In WScript.Arguments
  a = a & " " & x
Next
sh.Run """" & ps & """ -WindowStyle Hidden -NoProfile -ExecutionPolicy Bypass -File """ & d & "\DriverScan.ps1""" & a, 0, True
