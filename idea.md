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
- Shape crosshairs (arms, gap, thickness, T-style, dot, circle, outline, colours, opacity,
  offset) and PNG crosshairs. PNGs are copied into `images/` so they keep working if the
  original moves.
- Several profiles, which you can reorder, plus settings for the monitor and the hotkeys
  (validated before saving).
- **Live apply:** each change is saved and baked after a short pause, so the running overlay
  updates while you drag a slider.
- Command line: `--bake`, `--preview out.png`, `--print-config`, `--dir <folder>`.

### `overlay.bin` (format v1, little-endian)

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | magic `CLAY` |
| 4 | 2 | version (1) |
| 6 | 2 | flags (bit 0 = start visible) |
| 8 | 2 | monitor (0 = primary) |
| 10 | 2 | startup profile |
| 12 | 20 | 5 hotkeys × (modifiers u16, virtual key u16): toggle, editor, next, previous, quit. vk 0 = disabled |
| 32 | 2 | frame count |
| 34 | 2 | reserved |
| 36 | … | frames: offset x i32, offset y i32, width u16, height u16, anchor x u16, anchor y u16, then width × height premultiplied BGRA pixels |

The pixel layout is exactly what a 32-bit top-down DIB expects, so the overlay copies it with no
per-pixel work.

## Not in scope (yet)

- Exclusive fullscreen games. That needs injection or a graphics hook, which is exactly what this
  design avoids.
- A Linux or macOS overlay. The architecture allows one (a shaped, input-transparent X11/Wayland
  layer or an `NSWindow`), but it isn't built.
- Animated crosshairs. They'd need a timer, which breaks the "costs nothing" promise.
