Installing RunReCap
===================

1. Drag RunReCap onto the Applications folder.
2. Open RunReCap from Applications.

If macOS says RunReCap "can't be opened" or "Apple could not verify" it:

This build isn't notarized by Apple yet, so macOS asks you to confirm once.

  macOS 15 and later:
    1. Try to open RunReCap once and click "Done" on the warning.
    2. Open System Settings → Privacy & Security.
    3. Scroll down and click "Open Anyway" next to RunReCap, then confirm.

  macOS 13–14:
    Right-click (Control-click) RunReCap in Applications → Open → Open.

  Or, in Terminal:
    xattr -dr com.apple.quarantine /Applications/RunReCap.app

You only need to do this once.
