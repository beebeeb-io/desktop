<#
 uia-walk.ps1 - UI Automation walk: lists interactive elements without an accessible name.
 Windows PowerShell 5.1 + UIAutomationClient. See README.md.

 Walk (default): attaches to every top-level window of the process(es) matching -Process, walks all
 descendants and reports each element that is keyboard-focusable OR supports Invoke/Toggle/SelectionItem/
 ExpandCollapse, AND has an empty Name or ControlType Custom/Pane. Prints JSON, then one count line
 per window and a final "UIA-WALK: <n> unnamed controls". Exit 1 if any, 0 if none, 2 on setup error.
 -Keys: afterwards sends Tab/F6/arrows to each window (SendInput) and records the focused element.
 -SelfTest: checks the classifier and JSON path on synthetic elements, and that UIA loads. No app needed.
#>
param(
  [string]$Process = 'beebeeb-drive',
  [string]$Window = '',            # optional title substring, e.g. Files / Photos / Settings
  [string]$Out = '',               # write JSON here too
  [switch]$Keys,
  [int]$KeyRepeat = 12,
  [switch]$SelfTest
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes

$PatternProps = @(
  [System.Windows.Automation.AutomationElement]::IsInvokePatternAvailableProperty,
  [System.Windows.Automation.AutomationElement]::IsTogglePatternAvailableProperty,
  [System.Windows.Automation.AutomationElement]::IsSelectionItemPatternAvailableProperty,
  [System.Windows.Automation.AutomationElement]::IsExpandCollapsePatternAvailableProperty)
$PatternNames = 'Invoke', 'Toggle', 'SelectionItem', 'ExpandCollapse'

# Pure classifier: $Info has Name, ControlType (e.g. 'Button'), Focusable (bool), Patterns (string[]).
function Test-Unnamed($Info) {
  $interactive = $Info.Focusable -or (@($Info.Patterns).Count -gt 0)
  if (-not $interactive) { return $false }
  $emptyName = [string]::IsNullOrWhiteSpace($Info.Name)
  $vague = $Info.ControlType -in @('Custom', 'Pane')
  return ($emptyName -or $vague)
}

function Get-Info($El) {
  $c = $El.Current
  $pats = @()
  for ($i = 0; $i -lt $PatternProps.Count; $i++) {
    try { if ($El.GetCurrentPropertyValue($PatternProps[$i])) { $pats += $PatternNames[$i] } } catch {}
  }
  [pscustomobject]@{
    Name = $c.Name; ControlType = ($c.ControlType.ProgrammaticName -replace '^ControlType\.', '')
    AutomationId = $c.AutomationId; ClassName = $c.ClassName
    Focusable = [bool]$c.IsKeyboardFocusable; Patterns = $pats
    Rect = $c.BoundingRectangle.ToString()
  }
}

function Get-TopWindows([string]$Proc, [string]$Title) {
  $pids = @(Get-Process -Name $Proc -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
  if (-not $pids) { return @() }
  $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Window)
  $all = [System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children, $cond)
  foreach ($w in $all) {
    if ($pids -contains $w.Current.ProcessId -and ($Title -eq '' -or $w.Current.Name -like "*$Title*")) { $w }
  }
}

function Walk-Window($Win) {
  $found = New-Object System.Collections.ArrayList
  $total = 0
  $els = $Win.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
  foreach ($e in $els) {
    $total++
    try { $info = Get-Info $e } catch { continue }
    if (Test-Unnamed $info) { [void]$found.Add($info) }
  }
  [pscustomobject]@{ Window = $Win.Current.Name; Elements = $total; Unnamed = $found.Count; Controls = @($found) }
}

function Send-Key([string]$K) {
  Add-Type -AssemblyName System.Windows.Forms
  [System.Windows.Forms.SendKeys]::SendWait($K)   # wraps SendInput
}

function Key-Pass($Win, [int]$Repeat) {
  $log = @()
  try { $Win.SetFocus() } catch {}
  foreach ($k in @('{TAB}', '{F6}', '{DOWN}', '{RIGHT}')) {
    for ($i = 0; $i -lt $Repeat; $i++) {
      Send-Key $k; Start-Sleep -Milliseconds 120
      $f = [System.Windows.Automation.AutomationElement]::FocusedElement
      $d = if ($f) { Get-Info $f } else { $null }
      $log += [pscustomobject]@{ Key = $k; Focused = $d; Unnamed = [bool]($d -and (Test-Unnamed $d)) }
    }
  }
  $log
}

if ($SelfTest) {
  $fail = 0
  function Check($label, $got, $want) { if ($got -ne $want) { Write-Host "FAIL $label (got $got)"; $script:fail++ } }
  function Mk($n, $t, $f, $p) { [pscustomobject]@{ Name = $n; ControlType = $t; Focusable = $f; Patterns = $p } }
  Check 'named button'        (Test-Unnamed (Mk 'Save' 'Button' $true @('Invoke'))) $false
  Check 'empty-name button'   (Test-Unnamed (Mk '' 'Button' $true @('Invoke'))) $true
  Check 'whitespace name'     (Test-Unnamed (Mk '  ' 'ListItem' $false @('SelectionItem'))) $true
  Check 'custom named'        (Test-Unnamed (Mk 'Files' 'Custom' $true @())) $true
  Check 'pane named'          (Test-Unnamed (Mk 'Main' 'Pane' $false @('Invoke'))) $true
  Check 'static unnamed text' (Test-Unnamed (Mk '' 'Text' $false @())) $false
  Check 'focusable unnamed'   (Test-Unnamed (Mk '' 'Edit' $true @())) $true
  Check 'toggle named'        (Test-Unnamed (Mk 'Dark' 'CheckBox' $false @('Toggle'))) $false
  $json = ConvertTo-Json @([pscustomobject]@{ Window = 'x'; Elements = 1; Unnamed = 0; Controls = @() }) -Depth 6
  Check 'json roundtrip' ((ConvertFrom-Json $json)[0].Window) 'x'
  try { $r = [System.Windows.Automation.AutomationElement]::RootElement; Check 'uia root' ($null -ne $r) $true }
  catch { Write-Host "FAIL uia root: $($_.Exception.Message)"; $fail++ }
  if ($fail) { Write-Host "SELFTEST: $fail failed"; exit 1 }
  Write-Host 'SELFTEST: ok'; exit 0
}

$wins = @(Get-TopWindows $Process $Window)
if ($wins.Count -eq 0) { Write-Host "UIA-WALK: no window for process '$Process' (title filter '$Window')"; exit 2 }
$results = @(); $sum = 0
foreach ($w in $wins) {
  $r = Walk-Window $w
  if ($Keys) { $r | Add-Member -NotePropertyName KeyPass -NotePropertyValue @(Key-Pass $w $KeyRepeat) }
  $results += $r; $sum += $r.Unnamed
}
$json = ConvertTo-Json $results -Depth 8
if ($Out) { Set-Content -Path $Out -Value $json -Encoding UTF8 }
Write-Host $json
foreach ($r in $results) { Write-Host ("WINDOW '{0}': {1} elements, {2} unnamed" -f $r.Window, $r.Elements, $r.Unnamed) }
Write-Host "UIA-WALK: $sum unnamed controls"
if ($sum -gt 0) { exit 1 } else { exit 0 }
