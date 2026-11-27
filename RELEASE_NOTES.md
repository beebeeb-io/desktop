# Beebeeb Desktop 0.9.0 - Windows: the new native app

0.9.0 is a new Beebeeb for Windows, rebuilt from the ground up as a native app. It starts faster, uses less memory and no longer needs a web view. This release is for Windows and Linux; macOS follows in a later version.

**Known limitations:**
- **Linux:** the Linux app cannot sign in yet, because it has no support for a system password store.
- **Office files:** you can open and view Word, Excel and PowerPoint files. Editing them follows in a later version.

### What's New

- **A new native app.** Files, Photos, Shared, Trash and Search are built into one fast window.
- **Photos, PDFs, Office files, video and audio open inside Beebeeb.** Video and audio play in the app.
- **Sharing.** Create a link, set when it expires, add a password, or ask others to send you files.
- **Backups.** Back up your Documents, Desktop and Pictures folders.
- **Settings.** Choose your theme, storage location, notifications and speed limit, and lock Beebeeb again after a set time.
- **Sign-in.** Sign in with a password, a passkey or Windows Hello. If you quit while Windows Hello is asking, the prompt closes.

### Bug Fixes / Hardening

- **Sign-out:** signing out clears what the app kept for your account on this PC.
- **Window size:** the window stays inside your screen the first time you open it.
- **Shared view:** the footer shows your own account and storage use.
- **First load:** your file list appears faster when you open the app.
- **Video:** video streams and plays in the app without a full download first.
- **Video:** video plays in the app while it downloads, with a buffering indicator.
- **Backup:** backup no longer lists your whole tree every 10 minutes.
- **First load:** the app downloads your file list once at start.
- **Loading:** a loading state shows while your files load, instead of "Nothing here yet".
- **Paused sync:** when sync is paused, the app shows Resume.
- **Sign-in:** a sign-in problem found on real accounts is fixed.
- **Dates:** file dates show correctly.
- **System files:** system files are hidden from your lists.
- **Search, Trash and folders:** fixes to search, to Trash and to folders.
- **Settings:** Settings fills wide windows.
- **Trash:** the Trash row opens Trash.
- **Sidebar:** the sidebar sizes include Backups.
- **Thumbnails:** missing thumbnails are made when you open a folder, and Settings > Previews can repair them.
- **Search:** search no longer floods the server after an error.
- **Retry:** Beebeeb waits more gently before retrying after "too many requests".
- **Photos:** RAW photo types are recognised.

### Verification

- Real hardware smoke test: not run on this release yet.
- Not verified: Linux on real hardware; macOS is not in this release.

### Install / Update

This release is published to the alpha channel first. Windows installs following that channel receive it through the in-app updater; stable installs receive it only once it is promoted to stable.

macOS: not in this release; it follows in a later version.

For a fresh Windows install, download the NSIS `setup.exe` from the GitHub release assets.

<!-- changelog
title: Desktop 0.9.0: the new native app for Windows
date: 2026-11-21
tags: desktop
release: https://github.com/beebeeb-io/desktop/releases/tag/desktop-v0.9.0
summary: Beebeeb for Windows is now a native app that starts faster and opens your files inside Beebeeb.
new:
- A new native app with Files, Photos, Shared, Trash and Search in one window
- Photos, PDFs, Word, Excel and PowerPoint files open inside Beebeeb, and video and audio play there
- Create share links with an expiry date and a password
- Back up your Documents, Desktop and Pictures folders
- Sign in with a password, a passkey or Windows Hello
improved:
- The window stays inside your screen the first time you open it
fixed:
- The Shared view now shows your own account and storage use
- Signing out clears what the app kept for your account on this PC
security:
note: Windows only for now: macOS follows in a later version.
-->
