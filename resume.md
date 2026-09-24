# Horyzond resume point

Last completed phase: **P5 — Modal interaction, rules, hooks, IPC, and CLI**.

Last completed step: `fix(p6): handle nested null-buffer unmaps`.

## Resume next

Continue **P6 — First visible Wayland session with OpenGL**. The optional `nested-wayland` feature builds a Smithay/Winit/GLES compositor while the default workspace remains graphics-free. It publishes compositor, shared-memory, seat, xdg-shell, and output globals; handles xdg toplevel and popup configure/acknowledge/commit ordering; preserves adapter-owned geometry when acknowledging native serial-only resize callbacks; maps toplevels only after Smithay reports an attached client buffer; handles null-buffer unmaps and issues a fresh configure before remapping; renders toplevel, subsurface, popup, and client-cursor trees; sends toplevel and popup frame callbacks; uses native popup grabs; forwards keyboard and pointer input; reports metadata to rules; synchronizes layout geometry and client resize configures; retries stalled configures; paces full redraws at 60 Hz; tracks neutral frame ownership through presentation; and shuts down clients in protocol order. The root runtime routes nested lifecycle and metadata through core, rules, hooks, IPC, and live configuration reload.

Popup placement now uses Smithay's protocol positioner calculation, including gravity and constraint-adjustment handling against the logical output bounds, and refreshes when core scene geometry changes. Toplevel surface trees now render above a two-pixel GLES border with active/inactive focus colors, alongside the existing compositor background. Pointer input and client cursor surfaces use the current output scale. Initial maps and pointer-button presses transfer keyboard focus to the appropriate client and emit normalized focus events so core focus and paint order remain synchronized; releases continue to reach the selected client or popup grab, and nested popup parents resolve back to their owning toplevel.

The remaining P6 work is real-client validation of buffer release, texture lifetime, mapping, input, resize, close, invalid-config retention, and normal shutdown. This host cannot link a feature test executable because the system `libxkbcommon` development files are absent, and it has no installed Wayland sample client. `cargo check --features nested-wayland` and feature clippy are the available enabled-feature verification here.

## Phase checklist

| Phase | Status | Remaining work |
| --- | --- | --- |
| P0 | Complete | Revisit only if proven native contracts require adjustments. |
| P1 | Complete | Background log writer and release hardening defer to P10. |
| P2 | Complete | OS notification debounce and native event-loop watch integration defer to later platform work. |
| P3 | Complete | Real protocol dispatch is supplied by P6; desired-versus-committed state evolves with native adapters. |
| P4 | Complete | Maintain provider isolation and recovery while integrating native sessions. |
| P5 | Complete | Keep headless end-to-end coverage intact as native paths evolve. |
| P6 | In progress | Complete real-client validation and confirm buffer-release/texture-lifetime behavior with `libxkbcommon` development files and a Wayland client available. |
| P7 | Not started | DRM/KMS, session/seat, GBM/EGL, libinput, hotplug, VT switching, suspend/resume, and multi-output policy. |
| P8 | Not started | Layer shell, desktop protocols, clipboard, drag-and-drop, lock policy, and safe pending-spawn association. |
| P9 | Not started | Shader assets/reload, damage optimization, scaling, measured pacing, and direct scanout. |
| P10 | Not started | Persistent state, soak/failure testing, packaging, documentation, and release hardening. |
| P11 | Not started | Xwayland, native X11 management, and optional X11 compositing. |
| P12 | Not started | Vulkan device/pipeline/synchronization implementation and GL/Vulkan parity scenarios. |

## Repository state notes

- `examples/` contains local upstream reference repositories. Do not modify, compile, package, or commit it unless explicitly requested.
- The default `horyzond` build must continue to compile without display or graphics system libraries.
- Use `horyzond --config-dir PATH` for isolated test and nested instances.
