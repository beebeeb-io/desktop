// The empty state for a drive with no files yet.
import { Icon } from '../../design'
import { BBButton } from '../../design'
import type { ReactNode } from 'react'

const honeycombBg = `url("data:image/svg+xml,%3Csvg width='28' height='49' viewBox='0 0 28 49' xmlns='http://www.w3.org/2000/svg'%3E%3Cg fill-rule='evenodd'%3E%3Cg fill='%23f5b800' fill-opacity='1'%3E%3Cpath d='M13.99 9.25l13 7.5v15l-13 7.5L1 31.75v-15l12.99-7.5zM3 17.9v12.7l10.99 6.34 11-6.35V17.9l-11-6.34L3 17.9z'/%3E%3C/g%3E%3C/g%3E%3C/svg%3E")`

interface EmptyDriveProps {
  userName?: string
  onUpload: () => void
  onCreateFolder: () => void
  /**
   * When false, this empty state is rendered inside a subfolder (the parent
   * vault may already contain files). The heading and primary CTA shift from
   * "first file" wording to neutral "this folder is empty" copy. Defaults to
   * true to preserve existing root-level behaviour.
   */
  isRoot?: boolean
  /** Drive's "+ New" menu (task 1582). When given, it replaces the plain
   *  "Create folder" button (the menu's first item is Folder). */
  newMenu?: ReactNode
  /** Shown above the heading (task 1918 round 4: "Restore everything from the erase"). */
  notice?: ReactNode
}

export function EmptyDrive({ userName, onUpload, onCreateFolder, isRoot = true, newMenu, notice }: EmptyDriveProps) {
  const heading = isRoot
    ? (userName ? `Welcome, ${userName}.` : 'Your encrypted vault is empty')
    : 'This folder is empty'
  const uploadLabel = isRoot ? 'Upload first file' : 'Upload files'

  return (
    <div className="flex-1 flex items-center justify-center relative py-12">
      {/* Honeycomb pattern */}
      <div
        className="absolute inset-0 opacity-[0.03]"
        style={{ backgroundImage: honeycombBg }}
      />

      <div className="relative max-w-[520px] text-center">
        {notice}
        {/* Lock icon */}
        <div
          className="w-[58px] h-[58px] mx-auto mb-[18px] rounded-2xl flex items-center justify-center -rotate-[4deg]"
          style={{
            background: 'var(--color-amber)',
            boxShadow:
              '0 8px 24px -6px oklch(0.82 0.17 84 / 0.5), inset 0 1px 0 rgba(255,255,255,0.4)',
          }}
        >
          <Icon name="lock" size={26} className="text-ink" />
        </div>

        <h1 className="text-xl font-semibold text-ink mb-2">
          {heading}
        </h1>
        <p className="text-[14px] text-ink-3 leading-relaxed mb-[22px]">
          Drop files here or click Upload to get started — everything is encrypted before it
          leaves your device. Keys live only on your devices.
        </p>

        {/* Action buttons */}
        <div className="flex gap-2 justify-center mb-6">
          <BBButton variant="amber" size="lg" onClick={onUpload} className="gap-1.5">
            <Icon name="upload" size={13} /> {uploadLabel}
          </BBButton>
          {newMenu ?? (
            <BBButton size="lg" variant="ghost" onClick={onCreateFolder} className="gap-1.5">
              <Icon name="folder" size={13} /> Create folder
            </BBButton>
          )}
        </div>

        {/* Feature grid */}
        <div
          className="grid grid-cols-3 gap-2.5 p-3.5 bg-paper border border-line rounded-lg mb-4"
          style={{ boxShadow: 'var(--shadow-1)' }}
        >
          {(
            [
              ['shield', 'Encrypted', 'Before it leaves your device'],
              ['users', 'Share safely', 'Keys separate from links'],
              ['cloud', 'Stored in the EU', 'Europe · more locations soon'],
            ] as const
          ).map(([ico, title, sub]) => (
            <div key={ico} className="flex gap-2 items-start text-left">
              <div
                className="w-6 h-6 rounded-md shrink-0 flex items-center justify-center"
                style={{
                  background: 'var(--color-amber-bg)',
                  color: 'var(--color-amber-deep)',
                }}
              >
                <Icon name={ico} size={12} />
              </div>
              <div>
                <div className="text-xs font-semibold">{title}</div>
                <div className="text-[10px] text-ink-3">{sub}</div>
              </div>
            </div>
          ))}
        </div>

        {/* iOS app CTA */}
        <a
          href="https://apps.apple.com/app/id6766666400"
          target="_blank"
          rel="noopener noreferrer"
          className="inline-flex items-center gap-2 px-3.5 py-2 rounded-lg bg-paper border border-line text-[12px] text-ink-2 hover:bg-paper-2 hover:border-line-2 transition-colors"
          style={{ boxShadow: 'var(--shadow-1)' }}
        >
          <Icon name="image" size={13} className="text-amber-deep" />
          Get the iOS app to back up your camera roll automatically
        </a>

        {/* Encryption badge */}
        <div className="mt-4 inline-flex items-center gap-1.5 font-mono text-[10.5px] text-ink-3">
          <Icon name="lock" size={11} className="text-amber-deep" />
          AES-256-GCM &middot; encrypted on your device
        </div>
      </div>
    </div>
  )
}
