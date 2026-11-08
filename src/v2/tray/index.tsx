// Owned by batch B09: the notification-area flyout (DW-1201 to DW-1222), fixed 340 x 520, reusing the state
// precedence and polling of src/WindowsTray.tsx (move that logic into v2/tray/model.ts, keep its tests).
// Mounted for ?window=tray only once ./enabled.ts says so; until then today's flyout runs.
export default function TrayApp() {
  return <div className="grid h-screen place-items-center bg-paper font-sans text-[13px] text-ink-3">The v2 flyout is not built yet.</div>
}
