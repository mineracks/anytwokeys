# Profiles

One app, several sealed data trees, one open at a time. Built 2026-10-01 from
`mineracks-infrastructure/roadmap/anytwokeys/PROFILES-SPEC.md`.

## What a profile is

Everything the app keeps lives in a folder. A profile **is** such a folder, one level down:

```
~/Library/Application Support/Any Two Keys/
  profiles.json          unsealed index: id, name, class, picture, timestamps, open-on-start, discreet — nothing else
  profiles/<id>/         exactly the tree the app always kept: store.enc, slots.json, chain.db, attachments/, …
```

`lockbox::dir()` answers "the open profile's folder" (`profiles::dir()`); while none is open it answers the root, as
it always did, so every test and harness that points `A2K_DATA_DIR` at a folder keeps working. `A2K_PROFILE=<id>`
opens a profile before the window asks for anything.

Each profile has **its own store key** in its own Keychain item: service `com.anytwokeys.app.store-key`, account
`profile <id>`. The profile the first launch made out of the old folder keeps the account the app always used
(`store key`), so nothing is re-keyed. `slots.json` (Touch ID, idle lock, card slots) is per profile too.

### Classes

| class | meant for | opens with | on screen |
|---|---|---|---|
| `private` | the real data | Touch ID or a card, always asked, never on start | `PRIVATE` tag; the guard below |
| `work` | demo wallets for working on the app | a click (slots are made with `touch: false`) | `DEMO` tag |
| `fiction` | a made-up person for an audience | a click | `DEMO · fictional` tag |

## The screens

- **Picker** (`#profiles`, the welcome screen's own shape): one card per profile, last opened first, each with a
  round picture (`.ic.s64`, ring red for private / gold for a demo), the name, the class tag, one line, and Open
  (Open… for private, which leads into the lock screen). A dashed **New profile** card opens a narrow form: name,
  class as a `.pick`, one line of consequence. A work or fiction profile marked **Open on start** skips the picker.
- **Chip** (`#profile-chip`, top bar, left of the network pill): the picture at 22px, the name, the tag. It drops a
  menu of every profile plus **All profiles…**. Switching = `profile_open` (which locks first) then `location.reload()`.
- **App settings › This profile**: picture (the app's marks or a chosen picture, kept in the clear like the name),
  name, what the class means, Open on start (not for private), Discreet when it opens, Switch profile…, Delete.
- **Lock screen**: a `Choose another profile` link under the keys.

## The private guard

1. `guard_sharing(none: true)` sets the main `NSWindow.sharingType` to `NSWindowSharingNone` while a private profile
   is open: the window is black in screenshots, recordings and screen shares. Back to ReadOnly for the others.
2. `guard_displays()` reads `CGGetActiveDisplayList` / `CGDisplayIsInMirrorSet`; a thread in `run()` emits
   `displays-changed` whenever the count or mirroring changes. With a private profile open: more than one display, or
   a mirror, drops the **curtain** (`#curtain`) over the window. **Show anyway for 10 minutes** needs Touch ID again
   (`guard_override`). A display *change* while private re-locks (`lockNow`), override or not.
3. **Discreet mode** (`body.discreet`): `.content`, the vault tabs and the title are blurred; holding the page for
   350 ms peeks (`body.peek`); the chip toggles it and `profile_update { discreet }` remembers it. On by default for
   private. What a private profile copies is cleared from the clipboard after 60 s if still there.

What this does not stop: a camera pointed at the laptop.

## Commands

`profiles_list` (also runs the one-time legacy move and `A2K_PROFILE`), `profile_open`, `profile_close`,
`profile_create` (Touch ID first for private), `profile_update`, `profile_delete` (Touch ID; Keychain item first,
then the folder; never the open one), `profile_picture_pick`, `guard_displays`, `guard_sharing`, `guard_override`.

## Not yet

Moving a vault between profiles (a sealed vault bundle), exporting a profile as one archive, and the fiction
persona generator: see the spec's P3 and P4.
