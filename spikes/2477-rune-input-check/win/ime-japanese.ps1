# Adds or removes Japanese (with the Microsoft IME) in THIS account's language list, for the composition leg of
# win\run.ps1. User scope only (HKCU, Set-WinUserLanguageList): no language pack, no capability, no system
# locale, no region and no display-language change. The first language and the default input method stay as
# they are. Set-WinUserLanguageList also resets the account's regional FORMAT to the first language (seen: en-GB
# became en-US); the format is read before and written back after. Run it inside the desktop session whose input it changes:
#
#   powershell -NoProfile -File $env:BB_RUN_IN_DESKTOP -Script win\ime-japanese.ps1           # status, changes nothing
#   powershell -NoProfile -File $env:BB_RUN_IN_DESKTOP -Script win\ime-japanese-add.ps1
#   powershell -NoProfile -File $env:BB_RUN_IN_DESKTOP -Script win\ime-japanese-remove.ps1    # the revert
#
# (The desktop-session runner passes a script no arguments and no environment: hence the two one-line wrappers.)
param([ValidateSet('status', 'add', 'remove')][string]$Action = 'status')
$ErrorActionPreference = 'Stop'
$action = $Action
function Show([string]$When) {
  [Console]::Out.WriteLine("$When language list:")
  $now = Get-WinUserLanguageList   # a List: held in a variable first, so that foreach walks its items
  foreach ($l in $now) { [Console]::Out.WriteLine("  $($l.LanguageTag) tips=$($l.InputMethodTips -join ',')") }
  [Console]::Out.WriteLine("  regional format: $((Get-ItemProperty 'HKCU:\Control Panel\International').LocaleName)")
  [Console]::Out.WriteLine("  default input method override: '$((Get-WinDefaultInputMethodOverride).InputMethodTip)'")
}
Show 'before'
$format = (Get-ItemProperty 'HKCU:\Control Panel\International').LocaleName
$list = Get-WinUserLanguageList
$has = @($list | Where-Object { $_.LanguageTag -like 'ja*' }).Count -gt 0
switch ($action) {
  'add' {
    if ($has) { [Console]::Out.WriteLine('Japanese is already in the list: nothing changed') }
    else { $list.Add('ja-JP'); Set-WinUserLanguageList -LanguageList $list -Force; Set-Culture $format; Show 'after' }
  }
  'remove' {
    if (-not $has) { [Console]::Out.WriteLine('Japanese is not in the list: nothing changed') }
    else {
      $keep = New-WinUserLanguageList -Language $list[0].LanguageTag
      $keep.Clear()
      foreach ($l in $list) { if ($l.LanguageTag -notlike 'ja*') { $keep.Add($l) } }
      Set-WinUserLanguageList -LanguageList $keep -Force; Set-Culture $format; Show 'after'
    }
  }
  'status' { }
}
