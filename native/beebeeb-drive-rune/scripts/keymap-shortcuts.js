// keymap-shortcuts.js: the gate keys of "Commands and keyboard shortcuts" (the board's `.wn-dlg`
// that holds a `.wn-cmds` grid: DW-1232, DW-1233, DW-1234). Runs after keymap.js, in the same
// scope (`win`, `api`, `q`, `qa`). The window behind the dialog keeps the keys keymap.js gives it;
// the dialog's one button is `btn-close` there already (a `.kit-btn` by its words).
//
// Keys are class names and running numbers in document order: `sc-1-4-name` is the command of the
// 5th row of the 2nd list, `sc-1-4-home` its key or where it lives, `sc-1-4-line` the hairline under it. Never positions or strings.
// The scrim is the shell's (`dialog-scrim`), so it has no key here; the pixels around the card and
// the frame tiles hold it to the board.
const cmds = q('.wn-dlg .wn-cmds');
if (cmds) {
  const dlg = cmds.closest('.wn-dlg');
  const css = (el) => {
    const cs = getComputedStyle(el);
    const out = {};
    if (cs.borderRadius !== '0px') out.radius = cs.borderRadius;
    if (cs.boxShadow !== 'none') out.shadow = cs.boxShadow;
    if (cs.padding !== '0px') out.pad = cs.padding;
    if (cs.gap !== 'normal') out.gap = cs.gap;
    if (parseFloat(cs.borderTopWidth) || parseFloat(cs.borderBottomWidth)) out.border = `${cs.borderTopWidth} ${cs.borderBottomWidth} ${cs.borderBottomColor}`;
    return { css: out };
  };
  const wr = win.getBoundingClientRect();
  const oy = wr.y + win.clientTop;
  const text = (key, el) => { if (el) api.text(key, el, { lh: getComputedStyle(el).lineHeight, align: getComputedStyle(el).textAlign }); };
  api.box('sc-dlg', dlg, css(dlg));
  text('sc-title', q('.wn-dlg-head .t-title', dlg));
  text('sc-sub', q('.wn-dlg-head .t-sub', dlg));
  api.box('sc-foot', q('.wn-dlg-foot', dlg), css(q('.wn-dlg-foot', dlg)));
  qa(':scope > *', cmds).forEach((list, c) => {
    text(`sc-${c}-head`, q('.wn-cmdhead', list));
    qa('.wn-cmd', list).forEach((cmd, r) => {
      // The row's one drawn thing is the hairline under it (`border-bottom: .5px`): keyed as its own
      // strip. The row's whole box is not a key: its words sit on both of its edges, and the gate's
      // edge check on such a box measures the anti-aliasing of the words, not a border.
      const cr = cmd.getBoundingClientRect();
      const line = parseFloat(getComputedStyle(cmd).borderBottomWidth);
      api.box(`sc-${c}-${r}-line`, cmd, { y: Math.round((cr.bottom - line - oy) * 100) / 100, h: line, ...css(cmd) });
      text(`sc-${c}-${r}-name`, q(':scope > span:not(.h)', cmd));
      text(`sc-${c}-${r}-home`, q(':scope > .h', cmd));
    });
  });
}
