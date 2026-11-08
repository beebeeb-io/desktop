// The command table behind "Commands and keyboard shortcuts" (DW-1232 to DW-1234): every command of the other
// desktop's app menus (DM-1232 to DM-1234) with its Windows key, or where it lives. No Ctrl+Alt combination
// (AltGr is Ctrl+Alt on European keyboards). Windows' own keys are kept where Windows has one.

export interface CommandRow { name: string; where: string }
export interface CommandList { title: string; rows: CommandRow[] }

const r = (name: string, where: string): CommandRow => ({ name, where })

export const FILES_COMMANDS: CommandList[] = [
  { title: 'Beebeeb', rows: [
    r('About Beebeeb', 'Settings › General › About'),
    r('Settings…', 'Ctrl+,'),
    r('Check for updates…', 'Settings › General › About'),
    r('Lock Beebeeb', 'Settings › Security'),
    r('Hide Beebeeb', 'Minimize, or close the window: the b stays'),
    r('Show all', 'Windows has none'),
    r('Quit Beebeeb', "the b's menu"),
  ] },
  { title: 'File', rows: [
    r('New folder', 'Ctrl+Shift+N'),
    r('New file…', 'Ctrl+N'),
    r('New document…', 'New ▾'),
    r('Upload files…', 'Ctrl+O'),
    r('Upload a folder…', 'New ▾'),
    r('Open', 'Enter'),
    r('Open With', 'right-click › Open with'),
    r('Preview', 'Space'),
    r('Edit', 'Ctrl+E'),
    r('Save', 'Ctrl+S'),
    r('Share…', 'right-click, details panel'),
    r('Copy link', 'right-click'),
    r('Make available offline', 'right-click, details panel'),
    r('Show in File Explorer', 'right-click'),
    r('Details', 'Alt+Enter'),
    r('Rename', 'F2'),
    r('Move…', 'right-click'),
    r('Duplicate', 'Ctrl+D'),
    r('Version history…', 'right-click, details panel'),
    r('Move to Trash', 'Delete'),
    r('Close window', 'Alt+F4'),
  ] },
  { title: 'Edit', rows: [
    r('Undo', 'Ctrl+Z'),
    r('Redo', 'Ctrl+Y'),
    r('Cut', 'Ctrl+X'),
    r('Copy', 'Ctrl+C'),
    r('Paste', 'Ctrl+V'),
    r('Select all', 'Ctrl+A'),
    r('Search', 'Ctrl+F'),
  ] },
]

export const NAVIGATION_COMMANDS: CommandList[] = [
  { title: 'View', rows: [
    r('as List', 'toolbar: view switch'),
    r('as Grid', 'toolbar: view switch'),
    r('Sort by', 'toolbar: Sort'),
    r('Folders first', 'toolbar: Sort'),
    r('Show details panel', 'Alt+Shift+P'),
    r('Hide sidebar', 'Ctrl+Shift+B'),
    r('Photos across', 'Photos toolbar'),
    r('Enter full screen', 'F11'),
  ] },
  { title: 'Go', rows: [
    r('Back', 'Alt+Left'),
    r('Forward', 'Alt+Right'),
    r('Enclosing folder', 'Alt+Up'),
    r('Files', 'Ctrl+1'),
    r('Shared', 'Ctrl+2'),
    r('Photos', 'Ctrl+3'),
    r('Backups', 'Ctrl+4'),
    r('Trash', 'Ctrl+5'),
    r('Sync history', 'Ctrl+6'),
    r('Available offline', 'Ctrl+7'),
  ] },
  { title: 'Window', rows: [
    r('Minimize', 'the – caption button (Win+↓)'),
    r('Zoom', 'the □ caption button (Win+↑)'),
    r('Bring all to front', 'Windows has none: one window'),
    r('Beebeeb Drive', 'the taskbar, Alt+Tab'),
  ] },
  { title: 'Help', rows: [
    r('Search', 'F1: Help has its own search'),
    r('Beebeeb Help', 'F1'),
    r('Show the guide again', 'Settings › General › Help'),
    r('Report a problem…', 'Settings › General › Help'),
    r("What's new in 0.9", 'Settings › General › About'),
    r('Privacy policy', 'Settings › General › About'),
  ] },
]

export const EDITOR_COMMANDS: CommandList[] = [
  { title: 'Edit', rows: [
    r('Undo', 'Ctrl+Z'),
    r('Redo', 'Ctrl+Y'),
    r('Cut', 'Ctrl+X'),
    r('Copy', 'Ctrl+C'),
    r('Paste', 'Ctrl+V'),
    r('Paste and match style', 'Ctrl+Shift+V'),
    r('Select all', 'Ctrl+A'),
    r('Find', 'Ctrl+F'),
    r('Find next', 'F3'),
  ] },
  { title: 'View', rows: [
    r('Zoom in', 'Ctrl+Plus'),
    r('Zoom out', 'Ctrl+Minus'),
    r('Actual size', 'Ctrl+0'),
    r('Show outline', 'ribbon: Layout'),
    r('Show ribbon', 'Ctrl+F1'),
    r('Present', 'F5'),
    r('Enter full screen', 'F11'),
  ] },
  { title: 'Format', rows: [
    r('Bold', 'Ctrl+B'),
    r('Italic', 'Ctrl+I'),
    r('Underline', 'Ctrl+U'),
    r('Strikethrough', 'ribbon: Home'),
    r('Normal text', 'ribbon: Home'),
    r('Heading 1', 'ribbon: Home'),
    r('Heading 2', 'ribbon: Home'),
    r('Heading 3', 'ribbon: Home'),
    r('Align left', 'Ctrl+L'),
    r('Align center', 'Ctrl+E'),
    r('Align right', 'Ctrl+R'),
    r('Justify', 'Ctrl+J'),
    r('Bulleted list', 'ribbon: Home'),
    r('Numbered list', 'ribbon: Home'),
    r('Increase indent', 'ribbon: Home'),
    r('Decrease indent', 'ribbon: Home'),
  ] },
  { title: 'Insert', rows: [
    r('Table…', 'ribbon: Insert'),
    r('Image…', 'ribbon: Insert'),
    r('Link…', 'ribbon: Insert'),
    r('Page break', 'ribbon: Insert'),
    r('New slide', 'Ctrl+M'),
    r('Duplicate slide', 'right-click on a slide'),
  ] },
]

/** The AltGr rule as a check: no command key may be a Ctrl+Alt combination. */
export function ctrlAltCommands(lists: CommandList[]): string[] {
  return lists.flatMap((l) => l.rows).filter((x) => /ctrl\+alt|alt\+ctrl/i.test(x.where)).map((x) => x.name)
}
