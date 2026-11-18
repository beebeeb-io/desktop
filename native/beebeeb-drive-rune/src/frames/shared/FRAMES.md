# Lane f-shared-search-trash: the Windows frames of Shared, Search and Trash

The checklist for `src/frames/{shared,search,trash}`. Source: `design/desktop-v2/FRAMES.md` and `frames.json`
(Windows tab). Status is `built` once the frame's gate (`scripts/compare.sh <id>`, light and dark) shows
0 failing on all six counts.

The dialogs (DW-0504, DW-0511 to DW-0515, DW-0749) are `dialogs/`: their state lives on `SharedUi`, the
group's window draws them over the section they belong to, and their scenes register through
`scenes::FRAMES`. "Scene only" means the live app has no way to open the dialog yet: it shows crypto
material the engine has to give, or its action has no engine call, so it opens in its board scene alone.

## Shared (17)
| Frame | What it is | Status |
| --- | --- | --- |
| DW-0501 | Shared, With me: the "Waiting for you" card (Autumn shoot, Decline / Accept) and the Shared with you list | built |
| DW-0502 | Shared, By me: With people (3) and Links (3) | built, 1 pixel check red (item-0-2-pill-dot edge 11 > 9; see Rune limitations in the PR) |
| DW-0503 | Shared, Requests: 4 file requests, New request in the toolbar | built |
| DW-0504 | Dialog New request: Title, Note, four menus, Create link (amber) / Cancel | built (the four settings open their menus and Create link sends the request: w17-shared-requests) |
| DW-0505 | Menu on someone else's file: Open, Preview, Make available offline, Show in File Explorer, Details, Leave… | built |
| DW-0506 | Menu on a link: Copy link, Change expiry, Password…, Open, People, Stop sharing | built, 1 pixel check red (the same pill dot as DW-0502) |
| DW-0507 | Menu on a file request: Copy link, View files, Change expiry, Close request | built |
| DW-0508 | Empty With me: "Nothing shared with you" | built |
| DW-0509 | Empty By me: "You haven't shared anything" | built |
| DW-0510 | Empty Requests: "No file requests", New request | built |
| DW-0511 | Dialog People: Lisbon trip · 212 items, three people, Invite someone… / Done | built (opens from By me: Enter, People in the menu, a click on the people) |
| DW-0512 | Dialog one person's access: Email, Access (Can view / Can edit), Safety number, Remove access | built (Can view / Can edit change at once; Safety number asks for DW-0513, the security-reviewed wiring) |
| DW-0512b | Alert "Remove Jonas's access?": Remove access (red) / Cancel | built |
| DW-0513 | Dialog Safety number: twelve groups of five digits, They don't match / They match | built (the real number: the engine works it out from both keys, `core` `share_flow::safety`; They match / They don't match are kept as local marks: w18-people-safety-number) |
| DW-0513b | Dialog "Sharing with Jonas is stopped": Stops / Stays, the twelve groups, Compare again / Remove Jonas's access | built (as DW-0513: the real number and the marks) |
| DW-0514 | Dialog Invite someone: Email, Access, the first-share note, Send invite / Cancel | built |
| DW-0515 | Alert "Jonas's key has changed" over People: Compare safety numbers / Share anyway / Cancel | built (the engine reports the changed key; Share anyway shares only on that press: w18-people-safety-number) |

## Search (4)
| Frame | What it is | Status |
| --- | --- | --- |
| DW-0801 | Search opened: Recent (contract, lisbon, invoice 2026), Clear, what search covers | built (see `src/frames/search/FRAMES.md`, the group's own checklist) |
| DW-0802 | Results for "Contr": pills All / Folders / Photos / Documents, three rows with Where | built (see `src/frames/search/FRAMES.md`, the group's own checklist) |
| DW-0802b | Results during a first sign-in: the "Still getting your files ready" note with the live count | built (see `src/frames/search/FRAMES.md`, the group's own checklist) |
| DW-0803 | No results for "harbr" | built (see `src/frames/search/FRAMES.md`, the group's own checklist) |

## Trash (5)
| Frame | What it is | Status |
| --- | --- | --- |
| DW-0747 | Trash: 12 items · 240 MB, Empty Trash…, six rows with Deleted and Where it was | built |
| DW-0748 | Menu on Contract old.pdf: Restore, Details, Delete permanently… | built |
| DW-0749 | Dialog "Empty Trash?": the warning and the six-box authenticator code | built (Trash asks for it with `Intent::Frame("DW-0749")`; the code and the account password are proved and `engine_host::trash::purge` deletes: one item on its own route, more than one in batches of 500) |
| DW-0749b | Emptying: the progress note with its bar, Empty Trash… dimmed | built |
| DW-0117 | Empty Trash: "Trash is empty", no Empty Trash…, no sidebar count | built |

## Wired since (waves 16 to 18, 2026-11)

Shared's items act on the engine: Open, Show in File Explorer, a link's People, Leave, Copy link, Change expiry and
Password (links and requests), each with a confirmation where it removes access (w17-shared-items, w17-shared-links);
New request and Create link (w17-shared-requests); the real safety number and Share anyway in People (w18); Sort, the
search filter and the sync status line in Shared and Trash (w18-shared-trash-chrome). Shared's New opens Files' New menu.
The Search section above is stale by design: `src/frames/search/FRAMES.md` is the checklist.
