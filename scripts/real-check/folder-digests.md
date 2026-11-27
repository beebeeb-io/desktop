# folder-digests real check on VM 401

The epic's real check (`board/epics/folder-digests.md`, Setup > Real check) as steps. Run every command under
`scripts/coord/with-lock.sh vm401 -- <command>` (single seat; one lead at a time). Keyboard and mouse go over the
console; the same lock covers them. Write each count in the evidence file as "n/N" next to the step number.
Nothing here is a secret: the test account's password and the database URL are made on the VM and stay there.
Never point any step at production: the stack is `http://127.0.0.1:3001` on VM 401, the only host used.

## Where the numbers are
- Backup pass line (task 12), once per pass per category, message `backup: digest pass`; fields `category`,
  `digest_requests`, `listing_requests`, `folders_changed`, `folders_listed`, `folders_from_cache`, `trusted`.
  Requests of one pass = `digest_requests + listing_requests`. Read the line of the category that holds the tree
  (Pictures) and ignore the others.
- Drift check line (task 13), once per check, message `sync drift check`; fields `requests`, `changed`, `in_flux`,
  `disagreeing`, `stored`, `action`. `action` shows whether a resnapshot was requested.
- Both are `info` lines of the TEST build's log. This file does not know the log's exact path on Windows. Find it
  once (the TEST build uses the names `Beebeeb Test` / `beebeeb-test`), and call it $log below:
  `Get-ChildItem $env:LOCALAPPDATA,$env:APPDATA -Recurse -Filter *.log -ErrorAction SilentlyContinue | Where-Object { $_.FullName -match 'Beebeeb Test|beebeeb-test' } | Sort-Object LastWriteTime -Descending | Select-Object -First 5 FullName`
  If the `info` lines are not in a file, start the TEST build from a console with `RUST_LOG=info`, redirect its
  output to a file and use that file as $log.
- Read lines with `Select-String -Path $log -Pattern 'backup: digest pass' | Select-Object -Last 5` (or
  `'sync drift check'`). Before each step note how many lines match, and read only the lines after that number.

## Steps
1. **Stack.** On VM 401 start the local stack with server `folder-digests/integ` (a release build of `repos/server`
   at that branch tip, `BLOB_STORE=local` on a VM folder, an empty database; the dev compose file starts Postgres on
   :5434). Check `curl.exe -s http://127.0.0.1:3001/health` answers. Write down the server SHA. The fold and backfill
   jobs must run: digests are ready when `GET /api/v1/folder-digests` stops answering 503 `digests_not_ready`.
2. **Build.** Install the TEST build (`io.beebeeb.app.test`, never the real app) of the `native/folder-digests` tip,
   built on the win VM; write down the SHA. Start it with `BB_API_BASE=http://127.0.0.1:3001`.
3. **Account.** In the app make a fresh account on the local stack (a new address on a local test domain, a password
   made for this run and kept only on the console). Sign in. Nothing from an earlier run is reused.
4. **Tree.** In the Pictures folder of the VM user:
   `powershell -NoProfile -File scripts/real-check/make-backup-tree.ps1 -Root "$env:USERPROFILE\Pictures\bb-tree" -Files 41000 -Folders 2700 -Depth 7 -Seed 1 -Verify`
   Expect `files=41000`, `folders=2700`, exit 0. Write down the `sha256=` line (the same command with another empty
   `-Root` prints the same one). Then turn Pictures backup on in the app.
5. **First backup.** Wait until the app shows Pictures 41,000 of 41,000 backed up (expected 41,000/41,000; write the
   counts shown). In the web app the device's backup tree holds 2,700 folders.
6. **Idle passes (3/3).** Touch nothing. Take the next 3 `backup: digest pass` lines of the Pictures category, one per
   pass (about 10 minutes apart; "back up now" in the app starts one sooner). Per pass `digest_requests +
   listing_requests` must be at most 5. Write the three sums; pass = 3/3. Also write `folders_changed` (expect 0)
   and `trusted`.
7. **Deep add (1/1).**
   `powershell -NoProfile -File scripts/real-check/make-backup-tree.ps1 -Root "$env:USERPROFILE\Pictures\bb-tree" -Folders 2700 -Depth 7 -Seed 1 -AddDeep`
   prints the new file's relative path (7 folders deep). The next pass must have the sum at most 25 and the file must
   show as backed up (41,001/41,001). Write the sum and `folders_changed` (expect 7 or 8). The tree now has 41,001
   files (`-Files 41001 -Verify` agrees).
8. **Web rename (1/1).** In the web app (`http://127.0.0.1:5173`, same account) rename one folder of the backup
   device tree, at a deep level. The next pass must show `folders_changed` of at least 1 and end without error, and
   the backup status stays 41,001/41,001 (no re-upload). Write the line's counts.
9. **Idle drift checks (0/3).** Wait for 3 `sync drift check` lines (at most one per 10 minutes). Expected on each:
   `disagreeing=0`, `action` is not a resnapshot, `requests` at most 2. Pass = 0 of 3 started a resnapshot.
10. **Drift by direct SQL (1/1).** Without a sync op. First upload a small file `drift-probe.txt` to the account's
    root in the web app and wait for 1 `sync drift check` line after it (the file is in the mirror and its folder
    agrees: `disagreeing=0`). Then, against the local stack database ONLY (VM 401, port 5434; the database URL comes
    from the stack's own env on the VM, set as `$env:BB_PG_URL` for the shell, never written down), with `psql`:
    ```sql
    UPDATE files
       SET size_bytes = size_bytes + 1
     WHERE parent_id IS NULL AND NOT is_folder AND NOT is_trashed AND NOT is_uploading
       AND created_at = (SELECT max(created_at) FROM files
                          WHERE parent_id IS NULL AND NOT is_folder AND NOT is_trashed)
    RETURNING id, size_bytes;
    ```
    It must return exactly 1 row (the probe is the newest root file on a fresh stack with one account; with 0 or 2
    rows, undo it and stop). The statement fires the digest trigger and writes no sync op. Write the time. Expected:
    within 2 `sync drift check` lines after it, one shows `disagreeing=1` and the next one a resnapshot as its
    `action` (1/1); the app's mirror then shows the new size. Then 3 more idle checks start no resnapshot (0/3).
11. **Evidence.** One file with the server SHA, the app SHA, the tree `sha256=`, the counts of steps 5-10 as n/N and
    the lines read (counts only; the lines hold no id or name). A step without its count is red; say which.
