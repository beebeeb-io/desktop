// keymap-office.js: the gate keys of the office editor in the window (DW-1507 to DW-1511, DW-1550 to
// DW-1561) and of the dialog over it (DW-1234). Runs after keymap.js, in the same scope (`win`, `api`,
// `q`, `qa`). Scoped to these frames by the board's own frame id, so no other golden gains a key.
// The window's caption, sidebar and toolbar keep the keys every window has (keymap.js). Keys here are
// by the editor's structure, numbered in document order; the native side counts the same way.
{
  const ofFrame = (win.closest('[id^="DW-"]') || {}).id || '';
  if (/^DW-(15(0[7-9]|1[01]|5\d|6[01])|1234)$/.test(ofFrame)) {
    const shown = (el) => !!el && el.getClientRects().length > 0;
    const box = (key, el, extra = {}) => { if (shown(el)) api.box(key, el, extra); };
    const text = (key, el, extra = {}) => { if (shown(el) && el.textContent.trim()) api.text(key, el, extra); };
    const icon = (key, el) => { if (shown(el)) api.icon(key, el); };
    const dim = (el) => ({ opacity: getComputedStyle(el).opacity });
    // A `.mk-tbtn`: its box, then its glyph or its letter.
    const tool = (key, b) => {
      box(key, b, { on: b.classList.contains('on'), off: b.classList.contains('is-off') });
      icon(`${key}-icon`, q(':scope > svg', b));
      text(`${key}-letter`, q(':scope > b', b));
      const word = q(':scope > span', b);
      if (word) text(`${key}-label`, word);
    };
    // A `.kit-btn`: box, glyph, word, and the mono shortcut after the word.
    const button = (key, b) => {
      if (!shown(b)) return;
      box(key, b);
      icon(`${key}-icon`, q(':scope > svg', b));
      const word = q(':scope > span', b);
      const hint = word && q(':scope > .font-mono', word);
      if (hint) { api.textNode(`${key}-label`, word); text(`${key}-hint`, hint, dim(hint)); } else text(`${key}-label`, word);
    };
    const main = q('.mk-main');
    const ribbon = main && q(':scope > .flex-none.flex.flex-col', main);
    if (ribbon) {
      box('rib', ribbon);
      const [tabs, tools] = qa(':scope > div', ribbon);
      // The tabs' row: the tabs, the command search, Share, Save.
      qa(':scope > span.relative', tabs).forEach((tab, i) => {
        api.textNode(`rib-tab-${i}`, tab);
        box('rib-tab-on', q(':scope > span.absolute', tab));
      });
      const find = q('.mk-pop2', tabs);
      if (find) {
        box('rib-find', find);
        icon('rib-find-icon', q(':scope > svg', find));
        text('rib-find-text', q(':scope > .t-cap', find));
        box('rib-find-kbd-box', q('kbd', find));
        text('rib-find-kbd', q('kbd', find));
      }
      qa('.kit-btn', tabs).forEach((b, i) => button(`rib-btn-${i}`, b));
      // The tools' row, in order: tool buttons, pop-up buttons, hairlines.
      if (tools) {
        box('rib-tools', tools);
        let nt = 0, np = 0, ns = 0;
        for (const c of tools.children) {
          const el = c.classList.contains('kit-go') ? c.firstElementChild : c;
          if (!el) continue;
          if (el.classList.contains('mk-tbtn')) tool(`rt-${nt++}`, el);
          else if (el.classList.contains('mk-pop2')) {
            const k = `rp-${np++}`;
            box(k, el); api.textNode(`${k}-label`, el); icon(`${k}-caret`, q(':scope > svg', el));
          } else if (el.classList.contains('mk-sep')) box(`rsep-${ns++}`, el);
        }
      }
    }
    // Word: the outline pane, and the page on its stage.
    const body = main && q('.mk-body', main);
    const outline = body && q('.flex.flex-col.gap-1.py-3.px-2', body);
    if (outline) {
      box('ol', outline);
      text('ol-title', q('.t-label', outline));
      const close = q('.mk-tbtn', outline);
      if (close) tool('ol-close', close);
      qa(':scope > span.t-sub', outline).forEach((line, i) => {
        box(`ol-${i}-row`, line, { on: getComputedStyle(line).backgroundColor !== 'rgba(0, 0, 0, 0)' });
        api.textNode(`ol-${i}`, line);
      });
    }
    const stage = body && q('.flex-1.min-w-0.relative.flex.justify-center', body);
    if (stage) {
      box('stage', stage);
      const page = q(':scope > .flex.flex-col', stage);
      if (page) {
        box('page', page);
        qa(':scope > div, :scope > p', page).forEach((line, i) => text(`pg-${i}`, line, dim(line)));
      }
      const float = q(':scope > .glass', stage);
      if (float) {
        box('float', float);
        qa('.mk-tbtn', float).forEach((b, i) => tool(`ft-${i}`, b));
      }
    }
    // The status line: what the document is, then how it is kept, zoom, About.
    const foot = main && q(':scope > .mk-foot', main);
    if (foot) {
      let n = 0, ns = 0;
      for (const c of foot.children) {
        const el = c.classList.contains('kit-go') ? c.firstElementChild : c;
        if (!el || !shown(el)) continue;
        if (el.classList.contains('mk-sep')) { box(`st-sep-${ns++}`, el); continue; }
        if (el.classList.contains('flex-1')) continue;
        const k = `st-${n++}`;
        const svgs = qa(':scope > svg', el);
        const mono = q(':scope > .font-mono', el);
        if (svgs.length === 2 && mono) { icon(`${k}-less`, svgs[0]); text(`${k}-zoom`, mono); icon(`${k}-more`, svgs[1]); }
        else if (svgs.length === 1) { icon(`${k}-icon`, svgs[0]); api.textNode(k, el); }
        else text(k, el);
      }
    }
  }
}
