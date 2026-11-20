# w25 gate helpers (not product code): Version history read BEFORE/AFTER, verified save, New document
function global:To-Reports { $null = Front; Clear-Editor; $null = Click-Name 'Files' 'Button' 5; Beat 0.8; Back-To-Files; Beat 0.8; $null = DClick-Name 'Project Alpha' 'Text' 8; Beat 1.2; $null = DClick-Name 'reports' 'Text' 8; Beat 1.2 }
function global:Status { Wait-Text '^Page \d+ of' 3 }
# opens Version history for the row (select it first), returns @{n=<versions>; top=<newest row>; hdr=<header>}; screenshot per read
function global:VH([string]$name, [string]$tag) {
  $null = Click-Name $name 'Text' 8; Beat 1
  $r = Click-Rx '^Version history$' 'Button' 4; Beat 2.5
  $hdr = ''; $n = -1; $top = ''
  for ($i = 0; $i -lt 8 -and $n -lt 0; $i++) {
    $lines = @(Dump '.')
    $h = @($lines | ? { $_ -match '^Text\|[^|]*\b(\d+) versions?\|' })[0]
    if ($h) { $hdr = ($h -split '\|')[1]; $null = $hdr -match '(\d+) versions?$'; $n = [int]$matches[1]; $top = (@($lines | ? { $_ -match '^ListItem\|' }) | Select-Object -First 1); $top = ($top -split '\|')[1] } else { Pump 700 }
  }
  S "vh-$tag"; $null = Click-Rx '^Done$' 'Button' 3; Beat 0.8
  @{ n = $n; top = $top; hdr = $hdr; click = $r }
}
# Ctrl+S and wait until the status says 'saved as version N' without 'unsaved changes'; returns @{ok; status; secs; ver}
function global:Save-Verified([string]$tag) {
  $t = Get-Date; Send '^s'; $st = ''; $ok = $false; $ver = -1
  while (((Get-Date) - $t).TotalSeconds -lt 45) { Pump 600; $st = Status; if ($st -match 'saved as version (\d+)' -and $st -notmatch 'unsaved changes' -and $st -notmatch 'Saving') { $ok = $true; $ver = [int]$matches[1]; break } }
  Beat 2; $st2 = Status; S "saved-$tag"
  @{ ok = $ok; status = $st; status2 = $st2; secs = [Math]::Round(((Get-Date) - $t).TotalSeconds, 1); ver = $ver }
}
function global:Leave2 { Send '{ESC}'; Beat 1.2; $asked = [bool](Find-El 'Discard changes' 'Button'); if ($asked) { N 'FAIL SIGNAL: leaving the editor after Ctrl+S asked about unsaved changes (Discard changes pressed)'; $null = Click-Name 'Discard changes' 'Button' 2; Beat 1 }; $asked }
function global:Gate([string]$label, [int]$before, $after, $sv, [bool]$asked) {
  $ok = ($after.n -eq $before + 1) -and $sv.ok -and -not $asked
  Res "GATE $label" $ok ("BEFORE=$before AFTER=$($after.n) (need $($before + 1)); status='$($sv.status)' status-ver=$($sv.ver) saved-in=$($sv.secs)s; discard-prompt=$asked; newest row='$($after.top)'")
}
# the existing document, one run: open from the reports folder, type, Ctrl+S, leave, AFTER
function global:ExistingRun([string]$run, [string]$file = 'Proposal.docx') {
  To-Reports; $b = VH $file "$run-before"; N "$run BEFORE: $($b.hdr) | newest: $($b.top)"
  Open-Row $file 6; $o = Status; N "$run opened: '$o'"
  Type-Slow "R$run "; Beat 1.2; $ty = Status; N "$run typed: '$ty'"; if ($ty -notmatch 'unsaved changes') { Res "$run typing marks the document unsaved" $false $ty }
  $sv = Save-Verified $run; N "$run Ctrl+S: ok=$($sv.ok) '$($sv.status)' after 2 s: '$($sv.status2)' in $($sv.secs) s"
  $asked = Leave2; To-Reports; $a = VH $file "$run-after"; N "$run AFTER: $($a.hdr) | newest: $($a.top)"
  Gate "existing $file run $run" $b.n $a $sv $asked
}
function global:CreateDoc([string]$name) {
  Back-To-Files; Beat 0.8; $null = Click-Name 'New' 'Button' 5; Beat 1.2; $null = Click-Rx '^New document' 'MenuItem' 4; Beat 2
  $null = Click-Rx '^Letter' 'RadioButton' 3; Beat 1; $null = TypeNew 'Name' $name; Beat 1; $t = Get-Date; $null = Click-Name 'Create' 'Button' 3
  $st = ''; while (((Get-Date) - $t).TotalSeconds -lt 90) { Pump 500; $st = Status; if ($st) { break }; if (Has-Text "Couldn't read the copy on this PC") { break } }
  $secs = [Math]::Round(((Get-Date) - $t).TotalSeconds, 1); $err = [bool](Has-Text "Couldn't")
  S "create-$name"
  @{ status = $st; secs = $secs; err = $err }
}
# New document, variant A: type in the editor that Create opened, Ctrl+S, leave, AFTER (BEFORE of a made document is 1: the file's first version; checked in variant B)
function global:NewRunA([string]$run, [string]$name) {
  $c = CreateDoc $name; N "$run created '$name': opened in $($c.secs) s, status '$($c.status)' error=$($c.err)"; Res "$run New document opens (variant A)" ($c.status -ne '') "$($c.secs) s '$($c.status)'"
  if (-not $c.status) { N "texts: $((Texts2) -join ' ; ')"; Send '{ESC}'; Beat 1; Clear-Editor; return }
  Type-Slow 'Dear team, '; Beat 1.2; $ty = Status; N "$run typed: '$ty'"; if ($ty -notmatch 'unsaved changes') { Res "$run typing marks the new document unsaved" $false $ty }
  $sv = Save-Verified $run; N "$run Ctrl+S: ok=$($sv.ok) '$($sv.status)' after 2 s '$($sv.status2)' in $($sv.secs) s"
  $asked = Leave2; Back-To-Files; Beat 1.5; $a = VH "$name.docx" "$run-after"; N "$run AFTER: $($a.hdr) | newest: $($a.top)"
  Gate "new document $name (A: typed in the editor Create opened; BEFORE=1 = first version)" 1 $a $sv $asked
}
# New document, variant B: BEFORE read from Version history before typing (leave, read, reopen)
function global:NewRunB([string]$run, [string]$name) {
  $c = CreateDoc $name; N "$run created '$name': opened in $($c.secs) s, status '$($c.status)' error=$($c.err)"; Res "$run New document opens (variant B)" ($c.status -ne '') "$($c.secs) s '$($c.status)'"
  if (-not $c.status) { N "texts: $((Texts2) -join ' ; ')"; Send '{ESC}'; Beat 1; Clear-Editor; return }
  Send '{ESC}'; Beat 1.5; $null = Clear-Editor; Back-To-Files; Beat 2; $b = VH "$name.docx" "$run-before"; N "$run BEFORE: $($b.hdr) | newest: $($b.top)"
  Open-Row "$name.docx" 6; $o = Status; N "$run reopened: '$o'"
  Type-Slow 'Dear team, '; Beat 1.2; $ty = Status; N "$run typed: '$ty'"; if ($ty -notmatch 'unsaved changes') { Res "$run typing marks the new document unsaved" $false $ty }
  $sv = Save-Verified $run; N "$run Ctrl+S: ok=$($sv.ok) '$($sv.status)' after 2 s '$($sv.status2)' in $($sv.secs) s"
  $asked = Leave2; Back-To-Files; Beat 1.5; $a = VH "$name.docx" "$run-after"; N "$run AFTER: $($a.hdr) | newest: $($a.top)"
  Gate "new document $name (B: BEFORE read from Version history)" $b.n $a $sv $asked
}
function global:Helpers { @(Get-Process beebeeb-office-render -ea 0).Count }
# helper killed mid-edit, notice, Ctrl+S, AFTER = BEFORE + 1
function global:KillRun([string]$run, [string]$file = 'Proposal.docx') {
  To-Reports; $b = VH $file "$run-before"; N "$run BEFORE: $($b.hdr) | newest: $($b.top)"
  Open-Row $file 5; N "$run opened: '$(Status)' helpers=$(Helpers)"; Type-Slow "K$run "; Beat 3.5; N "$run typed: '$(Status)' helpers=$(Helpers)"
  $clk = Get-Date; Get-Process beebeeb-office-render -ea 0 | Stop-Process -Force
  $s = Wait-Text '^The editor stopped' 20; N "$run killed at $($clk.ToString('HH:mm:ss')); notice: '$s'; helpers now $(Helpers)"; S "kill-$run-notice"; Res "$run helper killed: the editor says the changes are restored" ($s -ne '') $s
  Beat 3; N "$run after kill: '$(Status)'"
  $sv = Save-Verified "kill$run"; N "$run Ctrl+S: ok=$($sv.ok) '$($sv.status)' after 2 s '$($sv.status2)' in $($sv.secs) s"
  $asked = Leave2; To-Reports; $a = VH $file "$run-after"; N "$run AFTER: $($a.hdr) | newest: $($a.top)"
  Gate "helper kill then Ctrl+S, $file run $run" $b.n $a $sv $asked
}
