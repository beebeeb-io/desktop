# w28 Ctrl+S gate again (final): existing x3 + new x3 on whatever account the previous run left signed in
. C:\bb\w28\hdr28.ps1
Remove-Item C:\bb\rec\chapters.log -ea 0
Head28; Ensure-In
N "signed in as: $(Who)"
GateSet "9K"
P 'end' { ToRoot; Beat 1 }
Get-Content C:\bb\rec\chapters.log
