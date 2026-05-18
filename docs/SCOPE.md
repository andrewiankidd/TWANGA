# Scope (what TWANGA isn't)

- **Not a tab library.** Tabs are a legal grey zone. The app ships empty. Users bring their own `.gp5` / `.gpx` / `.xml` files. Community sharing happens off-platform, like emulator ROMs.
- **Not a custom-content player for proprietary game formats.** That's [slopsmith](https://github.com/byrongamatos/slopsmith)'s niche.
- **No free polyphonic transcription in v1.** Polyphonic transcription remains an open problem in the open-source world. v1 covers monophonic transcription (record-to-tab) and polyphonic *verification* (classify against a known chord set). Free polyphonic transcription is an explicit stretch goal, not v1.
- **No runtime AI.** Pitch detection is deterministic DSP. AI is used during *development* (Claude Code), not in the shipped binary.
- **Mobile is v2.** Desktop tuner first; Android Oboe / AAudio quirks deferred.
