# Known Bugs

- Keyboard input is not working. Can not enter project names, enter param values, escape fields when I click into them, etc.
- Controller UI state is shared across tracks that are using the same engine.
  - If I have a tape engine loaded in track 1 and track 2, all control changes I made in track 1 are reflected in track 2 when I switch to that track and vice versa.
  - This is UI only. Does not change the audio/DSP of the other tracks.
- Address all the warnings that have accumulated.
- App crashes when master filter knob is brought above middle position.
