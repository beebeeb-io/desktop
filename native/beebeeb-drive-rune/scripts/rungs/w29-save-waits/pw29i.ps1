# w28 offline save x3: the tunnel to the server is cut by the host at READY-FOR-OFFLINE, restored at READY-FOR-ONLINE
. C:\bb\w28\hdr28.ps1
Remove-Item C:\bb\rec\chapters.log -ea 0
Head28; Install28; SignIn28
foreach ($i in 1, 2, 3, 4) { P "off$i" {
  $name = "W29 8off$i"; $f = "$name.docx"; ToRoot
  $c = CreateDoc $name; N "off$i created: '$($c.status)' in $($c.secs) s"; Send '{ESC}'; Beat 1.5; Clear-Editor; ToRoot
  $b = VH $f "off$i-before"; N "off$i BEFORE: $($b.hdr)"
  Open-Row $f 6; N "off$i opened: '$(Status)'"; Type-Slow "Typed while offline $i. "; Beat 1.2
  N "READY-FOR-OFFLINE off$i"; $mt = Wait-Marker "offline-off$i" 60
  if (-not $mt) { Res "W29 off$i the host cut the server" $false 'no marker'; return }
  Beat 12
  $sv = Save-Fast "off$i" 30
  N "off$i Ctrl+S offline: result=$($sv.result) in $($sv.secs) s; seen: $($sv.seen); app log: $($sv.log)"
  Beat 3; $st = Status; S "off$i-3s"; N "off$i +3 s: '$st'"
  Res "W29 off$i offline: the window says 'Saved on this PC, uploads when online' (no 'saved as version')" (($sv.result -eq 'kept-on-pc') -and ($st -notmatch 'saved as version')) "result=$($sv.result) '$($sv.status)' later '$st'"
  N "READY-FOR-ONLINE off$i"; $mo = Wait-Marker "online-off$i" 60
  $null = Leave2; Beat 45; ToRoot; Beat 1.5; $a = VH $f "off$i-after"; N "off$i AFTER online (+45 s): $($a.hdr) | newest: $($a.top)"
  Res "W29 off$i after the server is back the kept save uploads (history $($b.n) -> $($a.n))" ($a.n -eq $b.n + 1) "BEFORE $($b.n) AFTER $($a.n)"
} }
P 'end' { ToRoot; Beat 1 }
Get-Content C:\bb\rec\chapters.log
