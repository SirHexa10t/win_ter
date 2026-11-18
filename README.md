# win_ter - proton sandboxed

Easily run a Windows app through your terminal. Uses Proton (no Steam needed) inside a sandbox.

```sh
win_ter game.exe                      # the newest Proton, on Wayland, no network
win_ter --network --xwayland game.exe
win_ter --prefix-continue game.exe    # carry on with the saves Steam has for it
win_ter --take-ownership game.exe     # copy those over to the game's own folder first
win_ter --help
```

## What the sandbox does

- **Built on bubblewrap**, the sandbox Flatpak uses.
- **Your files stay out of reach.** The game sees the system read-only and nothing of your home,
  except its own folder and a folder of its own for its saves
  (`~/Games/manual_run_proton_prefixes/<game>`). It refuses to start if either would be your home,
  your Downloads or the like, or a whole drive.
- **No network**, unless you allow it (`--network`).
- **The kernel calls a game never needs are refused**: the same filter Flatpak gives every app, and
  the game can't set up sandboxes of its own.
- **Proton itself stays untouched**: whatever the game changes there is thrown away when it exits.
- **Wayland by default**, which keeps the game from seeing or typing into your other windows.

## What it doesn't stop

- The display, sound (your microphone included), the GPU and controllers reach the game, as games
  need them.
- On X11 (`--xwayland`), the game can see and type into other X11 windows.
- It runs as you: a flaw in the kernel or the GPU driver is still a way out.

## Needs

Linux, bubblewrap (0.8 or later for all of the above), python3, and a Proton: Steam's, or a packaged
one such as proton-cachyos or GE-Proton.
