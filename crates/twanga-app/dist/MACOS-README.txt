TWANGA — macOS first launch
============================

Apps aren't notarised yet — an Apple Developer ID ($99/yr) isn't
wired into the build. On first launch macOS Gatekeeper will say:

  "TWANGA cannot be opened because Apple cannot check it for
   malicious software."

Two ways to allow it (both are one-time per install — you won't
see the warning again):


METHOD 1 — System Settings (GUI)
---------------------------------

  1. Try to open TWANGA.app — macOS will block it. Click "Done".
  2. Open System Settings → Privacy & Security.
  3. Scroll down to the "Security" section.
  4. You'll see: "TWANGA.app was blocked..."  → click "Open Anyway".
  5. Confirm in the dialog that follows.


METHOD 2 — Terminal (faster)
-----------------------------

  Move TWANGA.app to /Applications (or wherever you keep apps),
  then run in Terminal:

    xattr -d com.apple.quarantine /Applications/TWANGA.app

  Open the .app normally. Gatekeeper will not block it again.


Why this happens
-----------------

TWANGA is an open-source project run by a solo dev. Notarising
each release requires an Apple Developer Program membership
($99/yr) — when that's wired into the build pipeline, this
warning will go away. The app is ad-hoc-signed so macOS can
verify its integrity hasn't changed since build; it just can't
verify the developer's identity without notarisation.

Source: https://github.com/andrewiankidd/TWANGA
