# crosslay: the idea

## The pitch

A crosshair overlay that costs nothing while you play. It draws the crosshair once, hands it to
the operating system's compositor, and then does nothing at all until you press one of its
hotkeys or change the crosshair.

Designing crosshairs is a separate program. You only run it when you want to change something,
and closing it gives all of its memory back. The two programs never talk directly. The editor
writes a file and the overlay reads it.

## Target platform

**Windows 10 and 11 (x64 and ARM64).** The first version of this repo was already Windows-first,
and that's where the games are. The overlay uses Win32 APIs directly. The editor is built on a
cross-platform GUI toolkit, so it also runs on Linux and macOS (handy for designing and for CI),
but the overlay there is only a stub that says it isn't supported yet.

## The original idea, and what was wrong with it

> A crosshair app in Rust, as light as possible, drawn once and never removed. A separate tool
> creates custom crosshairs and passes them to the app through a settings file.

The core is right. These are the gaps it leaves, and how this design closes them.

| Flaw | Why it matters | Fix |
|---|---|---|
| **"Settings file" means the overlay must parse settings and draw shapes.** | A TOML parser, a PNG decoder and a rasteriser all end up in the always-running process. That's code, memory and startup work that the overlay doesn't need. | The editor **bakes** the crosshair into a tiny binary file of ready-to-show pixels (`overlay.bin`). The overlay copies those pixels to the screen and has no parser, no decoder and no drawing code. |
| **"Drawn once, never removed"** leaves no way to hide it, switch it or close it. | You need to hide it in menus, use a different crosshair for a sniper, and quit it without Task Manager. | Global hotkeys: show/hide, next/previous crosshair, open editor, quit. They cost nothing until pressed (the OS delivers a message). All profiles are baked into the same file. |
| **How does the overlay notice a new file?** Polling on a timer wakes the CPU forever; reading only at startup means restarting after each edit. | Either wastes power or makes editing painful. | The overlay asks Windows to wake it when the folder changes (`FindFirstChangeNotification`) and sleeps on that and its message queue together. **No timers, zero idle wake-ups**, and edits show up instantly. |
| **Other "always on top" windows and games can cover it.** The usual fix is a timer that re-asserts top-most every second or so. | That timer is the only thing a "drawn once" overlay would still do while idle. | The overlay re-asserts top-most only when the foreground window changes (an out-of-context `SetWinEventHook`), which is exactly when a game could have jumped above it. |
| **Half-written files.** The overlay could read the file while the editor is writing it. | A torn read shows garbage, or nothing. | The editor writes to a temp file and renames it into place, which is atomic. The overlay validates the header and sizes and keeps the old crosshair if anything is off. |
| **Missing or broken file on first run.** | A blank screen with no hint of what's wrong. | The overlay has a small built-in crosshair it shows until a valid file appears. |
| **Clicks and focus.** A normal top-most window steals clicks and focus from the game. | Unplayable. | A layered, click-through (`WS_EX_TRANSPARENT`), non-activating tool window. It never gets focus and isn't in Alt+Tab. |
| **Blurry or wrongly sized on scaled displays.** | A 1 px line drawn at 150 % scaling becomes a smeared 1.5 px line. | The overlay is per-monitor DPI aware and works in physical pixels. |
| **Two overlays running at once.** | Double crosshair, and the second can't register hotkeys. | A named mutex makes the overlay single-instance. A second launch just exits. |
| **Anti-cheat.** | Injecting into a game or hooking its rendering can get accounts banned. | The overlay is an ordinary desktop window. It never touches a game process. The trade-off is honest: it works over **windowed and borderless** games, not exclusive fullscreen. |
| **Hand-editing the config.** If the overlay only reads baked pixels, editing the text config won't do anything by itself. | Power users like text files. | The text config (`config.toml`) stays the source of truth. `crosslay-editor --bake` turns it into `overlay.bin` without opening a window, and the editor re-bakes on start when the text file is newer. |

## How it fits together

```
 ┌──────────────────────┐   writes (atomic)    ┌────────────────────┐
 │ crosslay-editor.exe  │ ───────────────────▶ │ config.toml        │  human-readable source
 │  (only while open)   │ ───────────────────▶ │ overlay.bin        │  baked pixels + settings
 └──────────────────────┘                      │ images/*.png       │  imported PNG crosshairs
                                               └─────────┬──────────┘
                                                         │ folder change wakes the overlay
                                               ┌─────────▼──────────┐
                                               │ crosslay.exe       │  reads overlay.bin,
                                               │  (always running)  │  shows one frame, sleeps
                                               └────────────────────┘
```

Both programs find the same folder the same way. If a `config.toml` sits next to the exe, that
folder is used (portable mode). Otherwise it's `%APPDATA%\crosslay`.

### The overlay (`crosslay.exe`)

- Depends only on the Windows API bindings. There's no GUI toolkit, GPU context, parser or image
  decoder.
- At startup and on each change: read `overlay.bin`, pick the active frame, copy its pixels into
  a DIB section, call `UpdateLayeredWindow` once, then drop everything. The file isn't kept in
  memory; the compositor holds the only copy of the image.
- The thread then blocks in `MsgWaitForMultipleObjects` on the folder change handle and its
  message queue. Nothing runs until a hotkey, a file change, a display change or a foreground
  change.
- After drawing it trims its working set, so Task Manager shows the real (tiny) footprint.
- The release build is size-optimised, with LTO, `panic = "abort"` and stripped symbols, and
  links the C runtime statically so no redistributable is needed.

### The editor (`crosslay-editor.exe`)

- An egui app with live preview at real size or zoomed, on several backgrounds.
- Crosshairs are stacks of shape layers (cross, line, rectangle, circle, ring, arc,
  triangle), each with its own size, position, rotation, colour, opacity, outline, repeat
  around the centre and mirroring. The editor rasterises them on the CPU and bakes the result,
  so a crosshair with twenty shapes costs the overlay exactly what a dot does. Valorant
  crosshair codes are imported by turning them into cross and dot layers that draw the same
  pixels as the game, and crosshairs made only of such layers export back to a code. PNG
  crosshairs work too. PNGs are copied into `images/` so they keep working if the
  original moves.
- Several profiles, which you can reorder, plus settings for the monitor and the hotkeys
  (validated before saving).
- **Live apply:** each change is saved and baked after a short pause, so the running overlay
  updates while you drag a slider.
- Command line: `--bake`, `--update`, `--preview out.png`, `--print-config`, `--dir <folder>`.

### Updates

The overlay must stay offline and idle, so it never checks for updates itself. If `auto_update`
is on, it starts `crosslay-editor --update` once at startup. That separate process asks GitHub
Releases for the latest version and, if it's newer, downloads both exes. It then stops the overlay
(a "please quit" window message), renames the running exes to `*.old` (Windows allows renaming a
running exe but not overwriting it), moves the new ones into place, and restarts the overlay. The
`*.old` files are deleted the next time the editor starts. Two guards stop an update loop: the
release build fails if the tag doesn't match the version in `Cargo.toml`, and the updater
remembers the last tag it installed.

### Right-click crosshair

Each profile can have a second crosshair for while the right mouse button is held (aim down
sights), or toggled by each right click. The constraint is the same as everywhere else: nothing
may run while nothing happens, and nothing in the overlay may slow the game down.

- **Pre-drawn, swapped on the event.** The overlay has two layered windows. Each redraw uploads
  the normal crosshair to one and the right-click crosshair to the other. A button press only
  shows one and hides the other in a single `DeferWindowPos` batch. DWM already holds both
  images, so a swap reads no file, copies no pixels and allocates nothing.
- **Raw input, not a mouse hook.** A low-level mouse hook (`WH_MOUSE_LL`) sits in the input path:
  every mouse report waits for the hook to return, and a busy hook adds latency to the game's
  aim. Raw input (`RegisterRawInputDevices` with `RIDEV_INPUTSINK`) is posted to the overlay's
  queue after the fact, so it can't delay anything. Neither API can deliver button presses
  alone, so a moving mouse keeps waking the overlay. To keep that down, after reading input the
  overlay waits 4 ms on a high-resolution waitable timer before it looks at its queue again,
  then reads everything that piled up. A 1000 to 8000 Hz mouse costs at most 250 wake-ups a
  second, and a swap is at most 4 ms late. CI measures the cost per mouse report.
- **Opt-in by construction.** Raw input is registered only while the shown profile has a
  right-click crosshair and the overlay is visible, and removed otherwise. A setup without one
  never hears from the mouse.
- **No animation.** A swap is instant. Animating (say, lines spreading while firing) would need
  a timer and a redraw per step, which this design avoids. A "while firing" crosshair could reuse
  the same swap on the left button if it's ever wanted.

### `overlay.bin` (format v2, little-endian)

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | magic `CLAY` |
| 4 | 2 | version (1) |
| 6 | 2 | flags (bit 0 = start visible, bit 1 = auto update) |
| 8 | 2 | monitor (0 = primary) |
| 10 | 2 | startup profile |
| 12 | 20 | 5 hotkeys × (modifiers u16, virtual key u16): toggle, editor, next, previous, quit. vk 0 = disabled |
| 32 | 2 | profile count |
| 34 | 2 | right-click mode (0 = while held, 1 = toggle) |
| 36 | … | profiles: a frame, then its right-click frame if the frame's flags bit 0 is set |

A frame is: offset x i32, offset y i32, width u16, height u16, anchor x u16, anchor y u16,
flags u16 (bit 0 = a right-click frame follows), reserved u16, then width × height premultiplied
BGRA pixels. Version 1 files (crosslay 0.1) are still read: their frame headers end after the
anchor and they have no right-click frames.

The pixel layout is exactly what a 32-bit top-down DIB expects, so the overlay copies it with no
per-pixel work.

## Not in scope (yet)

- Exclusive fullscreen games. That needs injection or a graphics hook, which is exactly what this
  design avoids.
- A Linux or macOS overlay. The architecture allows one (a shaped, input-transparent X11/Wayland
  layer or an `NSWindow`), but it isn't built.
- Animated crosshairs. They'd need a timer, which breaks the "costs nothing" promise. Swapping
  to a pre-drawn crosshair on a mouse button (see above) covers the common case.
