/**
 * TemplatesGallery (task 1567) — shown for "New document". Five cards,
 * matching design/office-editor.html screen 05. Selecting one calls
 * `applyTemplate` (../../lib/office/templates) against a freshly opened
 * blank Writer document — see that module's header for why templates are
 * engine-generated rather than pre-baked binary files.
 */

import { Icon } from '../../design'
import { TEMPLATE_DEFS, type TemplateDef } from '../lib/office/templates'

export interface TemplatesGalleryProps {
  onSelect: (id: TemplateDef['id']) => void
  onBack: () => void
}

export function TemplatesGallery({ onSelect, onBack }: TemplatesGalleryProps) {
  return (
    <div className="flex h-full w-full flex-col bg-paper" data-testid="office-templates-gallery">
      <div className="flex h-16 shrink-0 items-center gap-3.5 border-b border-line bg-paper-2 px-6">
        <button type="button" onClick={onBack} className="flex items-center gap-1.5 text-[13px] text-ink-3 hover:text-ink">
          <Icon name="chevron-right" size={12} className="rotate-180" />
          Back
        </button>
        <h1 className="text-[17px] font-semibold text-ink">New document</h1>
      </div>
      <div className="flex flex-1 flex-col items-center gap-6 overflow-auto px-6 py-11">
        <p className="text-[13px] text-ink-3">Everything you create here is encrypted before it leaves this device.</p>
        <div className="flex flex-wrap justify-center gap-5">
          {TEMPLATE_DEFS.map((tpl) => (
            <button
              key={tpl.id}
              type="button"
              data-testid={`template-${tpl.id}`}
              onClick={() => onSelect(tpl.id)}
              className="flex w-[190px] flex-col gap-2.5 text-left"
            >
              <div className="flex h-[230px] w-full items-start rounded-lg border border-line bg-white p-4 shadow-1">
                <TemplateThumb id={tpl.id} />
              </div>
              <b className="text-[13px] text-ink">{tpl.label}</b>
              <span className="text-[11.5px] text-ink-3">{tpl.description}</span>
            </button>
          ))}
        </div>
      </div>
    </div>
  )
}

function TemplateThumb({ id }: { id: TemplateDef['id'] }) {
  const bar = (w: string, h = '5px', extra = '') => (
    <span className={`block rounded-[1px] bg-[#DAD4C6] ${extra}`} style={{ width: w, height: h }} />
  )
  switch (id) {
    case 'letter':
      return (
        <div className="flex w-full flex-col gap-2.5">
          {bar('40%', '8px')}
          <div className="mt-2 flex flex-col gap-1.5">
            {bar('60%')}
            {bar('90%')}
            {bar('85%')}
            {bar('70%')}
          </div>
        </div>
      )
    case 'invoice':
      return (
        <div className="flex w-full flex-col gap-2">
          {bar('50%', '8px')}
          <div className="mt-1.5 h-px w-full bg-[#DAD4C6]" />
          <div className="mt-1.5 flex flex-col gap-1.5">
            {bar('100%')}
            {bar('100%')}
            {bar('100%')}
          </div>
          <div className="mt-1.5 self-end">{bar('40%', '6px')}</div>
        </div>
      )
    case 'meeting-notes':
      return (
        <div className="flex w-full flex-col gap-2.5">
          {bar('55%', '8px')}
          <div className="mt-2 flex flex-col gap-1.5">
            {bar('35%')}
            {bar('80%')}
            {bar('30%')}
            {bar('75%')}
          </div>
        </div>
      )
    case 'report':
      return (
        <div className="flex w-full flex-col items-center gap-2.5 pt-6">
          {bar('70%', '9px')}
          {bar('45%')}
        </div>
      )
    case 'blank':
    default:
      return <span className="h-full w-full" />
  }
}
