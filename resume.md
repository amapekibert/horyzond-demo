# Horyzond resume point

Last completed phase: **P5 — Modal interaction, rules, hooks, IPC, and CLI**.

Last completed step: `feat(p6): add feature-gated Wayland adapter`.

## Resume next

Continue **P6 — First visible Wayland session with OpenGL**. P5 is complete: configured Normal, Insert, Passthrough, Move, Resize, Layout Select, and pending-spawn workflows have transitions, cancellation or recovery bindings, consumed press/release tracking, and visible IPC status. Rules are deterministic and non-recursive; their actions safely create pending launches, select layouts, cancel pending launches, or remain provider-opaque. Lifecycle hooks run after their committed boundaries under quotas. The Unix IPC transport is versioned, framed, bounded, same-user only, and exercised by `horyctl`. The automated headless end-to-end test starts the compositor, selects all four shipped layout profiles through modal input, validates recovery from every configured mode, and checks pending-spawn cancellation. P6 now has `wm-backend-wayland`, an optional Smithay 0.7 adapter boundary whose `nested` feature selects only `backend_winit`, `renderer_gl`, and `wayland_frontend`; the default workspace still has no active graphics dependency. Next, implement protocol-neutral configure transaction tracking and then build the nested xdg-shell lifecycle and GL frame path inside the feature-gated adapter.

## Phase checklist

| Phase | Status | Remaining work |
| --- | --- | --- |
| P0 | Complete | No functional work. Revisit contracts only when real P6 frame ownership proves an adjustment necessary. |
| P1 | Complete | P1 has bootstrap, an exclusive runtime lock, structured `latest.log`, previous-log archival, explicit crash reports, a process panic hook, and an unclean-session marker. A bounded background log writer is deferred to P10 hardening. |
| P2 | Complete | Lua 5.4 is restricted to table/string/math/UTF-8 libraries; imports stay beneath the config root, are cached per candidate, and are polled for changes. OS notification debounce and event-loop integration remain future work. |
| P3 | Complete | `wm-core` manages deterministic map/unmap, focus, world geometry, and outputs. `wm-scene` handles camera projection/inversion, picking, snapshots, and coalesced damage. Desired-versus-committed configure state and real event-loop dispatch remain for later protocol work. |
| P4 | Complete | Arbitrary providers have bounded calculation and interaction callbacks, validated output, isolated last-good reload handling, generic recovery, data-only state migration, cross-provider history, and independent per-workspace selection. |
| P5 | Complete | Modal workflows, deterministic rules and action dispatch, lifecycle hooks, bounded same-user IPC, offline and online `horyctl` commands, and an automated headless scenario covering every shipped layout profile and modal recovery path. |
| P6 | In progress | `wm-backend-wayland` provides an optional Smithay 0.7 nested adapter boundary and compiles both disabled and enabled. Implement configure transactions, minimal xdg-shell lifecycle, GLES rendering, frame/buffer completion ordering, and composition-root selection; retain the independent headless path. |
| P7 | Not started | DRM/KMS, session/seat, GBM/EGL, libinput, hotplug, and suspend/resume. |
| P8 | Not started | Layer shell, desktop protocols, clipboard, drag-and-drop, lock policy, and safe pending-spawn association. |
| P9 | Not started | Shader assets/reload, damage correctness, scaling, frame pacing, and safe direct scanout. |
| P10 | Not started | Persistent state, soak/failure testing, packaging, documentation, and release hardening. |
| P11 | Not started | Xwayland, native X11 management, then optional X11 compositing as a separate capability. |
| P12 | Not started | Vulkan device/pipeline/synchronization implementation and GL/Vulkan parity scenarios. |

## Repository state notes

- `examples/` contains local upstream reference repositories. Do not modify, compile, package, or include them in Horyzond commits unless the user explicitly requests it.
- P0 has no external crate dependencies. The first dependencies must be selected and license-checked at the phase that needs them.
- The `horyzond` binary intentionally wires `HeadlessBackend` and `RecordingRenderer` only. It must keep compiling without display or graphics system libraries.
- `horyzond --config-dir PATH` is the isolated execution path for tests and nested instances. Running without that option creates and uses `~/.config/horyzond/` as requested.
