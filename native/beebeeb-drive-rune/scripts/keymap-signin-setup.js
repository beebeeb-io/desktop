// keymap-signin-setup.js: the gate keys of the first run on Windows (DW-0201 to DW-0209) and the
// first-time guide overlay (DW-0380, DW-0301 to DW-0305), on top of what keymap-signin.js keys in
// any `.au-mac` window (lockup, heading, buttons, banners, lists, rows).
// Same scope as keymap.js (`win`, `api`, `q`, `qa`); a block of its own, so no name collides.
{
  const mac = q('.au-mac');
  const steps = mac && q('.au-mac-side ol', mac);
  if (steps) {
    const css = (el) => {
      const cs = getComputedStyle(el); const out = {};
      if (cs.borderRadius !== '0px') out.radius = cs.borderRadius;
      if (cs.boxShadow !== 'none') out.shadow = cs.boxShadow;
      if (parseFloat(cs.borderTopWidth)) out.border = `${cs.borderTopWidth} ${cs.borderTopColor}`;
      if (cs.padding !== '0px') out.pad = cs.padding;
      if (cs.gap !== 'normal') out.gap = cs.gap;
      if (cs.opacity !== '1') out.opacity = cs.opacity;
      return { css: out };
    };
    const shown = (el) => !!el && el.getClientRects().length > 0;
    const box = (key, el) => { if (shown(el)) api.box(key, el, css(el)); };
    const text = (key, el) => { if (shown(el) && el.textContent.trim()) api.text(key, el, { lh: getComputedStyle(el).lineHeight }); };
    const node = (key, el) => { if (shown(el) && [...el.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim())) api.textNode(key, el); };
    const icon = (key, el) => { if (shown(el)) api.icon(key, el); };
    // A bare text node measured without the spaces at its ends (the board's string is compared trimmed, so
    // its box is too): the words go in a span of their own, which changes nothing in the line.
    const trimmed = (key, parent) => {
      const n = parent && [...parent.childNodes].find((c) => c.nodeType === 3 && c.textContent.trim());
      if (!n) return;
      const [, lead, words, trail] = n.textContent.match(/^(\s*)([\s\S]*?)(\s*)$/);
      const span = document.createElement('span'); span.textContent = words;
      n.replaceWith(document.createTextNode(lead), span, document.createTextNode(trail));
      api.text(key, span);
    };

    // ---- The side panel: the seven steps, and what File Explorer is doing ----
    qa(':scope > li', steps).forEach((li, i) => {
      const dot = q(':scope > span', li);
      box(`step-${i}-dot`, dot);
      icon(`step-${i}-check`, q('svg', dot));
      node(`step-${i}-n`, dot);
      node(`step-${i}`, li);
    });
    const status = q('.au-mac-side > .mt-auto', mac);
    if (status) { icon('side-status-icon', q('svg', status)); node('side-status', status); }

    // ---- The main part ----
    const main = q('.au-mac-main', mac);
    text('skip', q(':scope > .absolute.t-sub', main));
    const col = q('.au-mac-col', main);

    // The preview card of step 4: the sample, its size and the original's.
    const card = q(':scope > .kit-card:not(.kit-os)', col);
    if (card) {
      box('prev-card', card);
      box('prev-thumb', q('.mk-ph', card));
      const words = q('.flex-1', card);
      const strong = q(':scope > span', words);
      trimmed('prev-size', strong);
      text('prev-size-bytes', q('.font-mono', strong));
      trimmed('prev-orig', words);
      text('prev-orig-bytes', q(':scope > .font-mono', words));
    }
    qa(':scope > .t-label', col).forEach((l, i) => text(`label-${i}`, l));
    qa('.kit-seg', col).forEach((seg, i) => {
      box(`seg-${i}`, seg);
      qa(':scope > span', seg).forEach((tab, j) => {
        if (tab.classList.contains('on')) box(`seg-${i}-on`, tab);
        api.textNode(`seg-${i}-${j}`, tab);
      });
    });
    qa('.kit-item', col).forEach((it, i) => {
      const pop = q('.mk-pop2', it);
      if (pop) { box(`item-${i}-pop`, pop); api.textNode(`item-${i}-pop-label`, pop); icon(`item-${i}-pop-caret`, q('svg', pop)); }
      const sw = q('[role="switch"] > span', it);
      if (sw) { box(`item-${i}-switch`, sw); box(`item-${i}-knob`, q('span', sw)); }
    });

    // Step 5: File Explorer's navigation pane, as a picture.
    const nav = q(':scope > .kit-os', col);
    if (nav) {
      box('nav', nav);
      qa('.wn-fdlg-ni', nav).forEach((ni, i) => {
        if (ni.classList.contains('on')) box('nav-on', ni);
        // The b is a `<use>` of the board's symbol: no path to compare, so it is a box (place and pixels).
        const svg = q('svg', ni);
        if (svg && q('use', svg)) box(`nav-${i}-b`, svg); else icon(`nav-${i}-icon`, svg);
        text(`nav-${i}`, q(':scope > span:last-child', ni));
      });
      box('nav-sep', q('.wn-fdlg-sep', nav));
    }
    const bar = q(':scope > .mk-bar', col);
    if (bar) { box('bar', bar); box('bar-fill', q('i', bar)); }
    const tile = q(':scope > .grid > .kit-tile', col);
    if (tile) { box('tile', tile); icon('tile-icon', q('svg', tile)); }
    text('banner-0-code', q('.kit-banner-text > span > .font-mono', col));

    // ---- DW-0209: the compare view, a sheet over the window ----

    const sheet = q('.mk-sheet', mac);
    if (sheet) {
      box('scrim', q('.mk-scrim', mac));
      box('sheet', sheet);
      box('cmp', q(':scope > .mk', sheet));
      const [head, stage, foot] = qa(':scope > .mk > div', sheet);
      const done = q('.kit-btn', head);
      box('cmp-done', done); text('cmp-done-label', q('span', done));
      const seg = q('.kit-seg', head);
      box('cmp-seg', seg);
      qa(':scope > span', seg).forEach((tab, j) => { if (tab.classList.contains('on')) box('cmp-seg-on', tab); api.textNode(`cmp-seg-${j}`, tab); });
      text('cmp-hint', q('.ml-auto', head));
      const photo = q('.mk-ph', stage);
      box('cmp-photo', photo);
      box('cmp-wipe', q(':scope > span:not(.t-cap)', photo));
      qa(':scope > .t-cap', photo).forEach((tag, j) => { box(`cmp-tag-${j}-box`, tag); api.textNode(`cmp-tag-${j}`, tag); });
      qa(':scope > span.flex-col', foot).forEach((opt, j) => {
        if (getComputedStyle(opt).backgroundColor !== 'rgba(0, 0, 0, 0)') box(`cmp-opt-${j}-on`, opt);
        text(`cmp-opt-${j}`, q('b', opt));
        text(`cmp-opt-${j}-bytes`, q('span', opt));
      });
      box('cmp-sep', q('.mk-sep', foot));
      const choose = q(':scope > .kit-btn', foot);
      box('cmp-choose', choose); text('cmp-choose-label', q('span', choose));
    }
  }
}

// ---- The first-time guide overlay (DW-0380 and DW-0301 to DW-0305) ----
// Runs for every frame; the guard (`mk-coach` present) limits it to guide frames.
{
  const coach = q('.mk-coach');
  if (coach) {
    api.box('coach', coach);

    // DW-0380 only: the heading "Beebeeb lives in the notification area"
    const title = q('.t-head', coach);
    if (title) api.text('coach-title', title);

    // The body paragraph: t-body in tips 1-5, t-sub in DW-0380
    const body = q('p', coach);
    if (body) api.text('coach-text', body);

    // Progress dots and the active dot (tips 1-5 only)
    const dots = q('.mk-pdots', coach);
    if (dots) {
      api.box('coach-dots', dots);
      [...dots.querySelectorAll('i')].forEach((dot, i) => api.box(`coach-dot-${i}`, dot));
    }

    // "1 of 5" / "5 of 5" counter (tips 1-5 only)
    const counter = q('.t-cap', coach);
    if (counter) api.text('coach-counter', counter);

    // Action buttons: Skip (ghost) and Next / Done / Got it (outlined or amber)
    [...coach.querySelectorAll('.kit-btn-sm')].forEach((btn) => {
      const label = btn.querySelector('span');
      const t = label ? label.textContent.trim() : '';
      if (t === 'Skip') {
        api.box('coach-skip', btn);
        api.text('coach-skip-label', label);
      } else if (t === 'Next' || t === 'Done' || t === 'Got it') {
        api.box('coach-next', btn);
        api.text('coach-next-label', label);
      }
    });
  }

  // The highlight ring around the target element (sibling of coach, DW-0301 to DW-0305)
  const ring = q('.mk-ring');
  if (ring) api.box('coach-ring', ring);
}
