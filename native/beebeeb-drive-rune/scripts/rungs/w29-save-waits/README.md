w29 the Windows test machine proof (build native/w29-save-waits; harness = w28 run11 scripts: run28.sh, watch28.sh, lib28.ps1, api-up.sh).
pw29i = clean install + 4 offline Ctrl+S cycles; pw28g = Ctrl+S gate (existing x3, New document x3, tag edited per run);
lib29-createdoc-fine.ps1 = CreateDoc with a 30 ms poll, appended to lib28.ps1 (the old one polled every 0.5-3 s).
logs/: A (first gate, old CreateDoc, aborted by a host timeout), G (gate on 297882cf's predecessor build b252efd8), G2 (gate, final), I (offline, final).
