# Horyzond resume point

Last completed phase: **P4 — Provider-defined Lua layouts**.

Last completed step: `feat(p5): control cameras through IPC`.

## Resume next

Continue **P5 — Modal interaction, rules, hooks, IPC, and CLI**. P4 is complete: the core stores opaque `LayoutId` values and calls a neutral `LayoutEngine` contract; Lua providers are independent, bounded, reloadable, and versioned. They return validated geometry, optional complete back-to-front paint order, JSON-compatible state, and optional opaque interaction results. Configuration and the runtime select arbitrary IDs without knowing layout names. P5 now has `wm-input`, a backend-neutral modal state machine with configured bindings, transitions, consumed press/release tracking, and safe replacement. It also has deterministic rules, bounded hook policy, a versioned length-prefixed JSON protocol, bounded subscriptions, and an owner-only Unix-domain transport. The transport accepts at most one request per non-blocking poll, applies short I/O deadlines, preserves request IDs, and removes only its exact stale socket path. The composition root derives a distinct private socket path for every configuration root and routes status, configuration status, deterministic window IDs, workspace snapshots, configured opaque layout selection, and validated camera transactions. `horyctl status`, `horyctl windows`, `horyctl workspaces`, `horyctl layout select ID`, and `horyctl camera set X Y ZOOM` exercise those endpoints. Next, add configuration reload, pending spawn, and configuration-backed rules/hooks without making the CLI a runtime dependency.

## Phase checklist

| Phase | Status | Remaining work |
| --- | --- | --- |
| P0 | Complete | No functional work. Revisit contracts only when real P6 frame ownership proves an adjustment necessary. |
| P1 | Complete | P1 has bootstrap, an exclusive runtime lock, structured `latest.log`, previous-log archival, explicit crash reports, a process panic hook, and an unclean-session marker. A bounded background log writer is deferred to P10 hardening. |
| P2 | Complete | Lua 5.4 is restricted to table/string/math/UTF-8 libraries; imports stay beneath the config root, are cached per candidate, and are polled for changes. OS notification debounce and event-loop integration remain future work. |
| P3 | Complete | `wm-core` manages deterministic map/unmap, focus, world geometry, and outputs. `wm-scene` handles camera projection/inversion, picking, snapshots, and coalesced damage. Desired-versus-committed configure state and real event-loop dispatch remain for later protocol work. |
| P4 | Complete | Arbitrary providers have bounded calculation and interaction callbacks, validated output, isolated last-good reload handling, generic recovery, data-only state migration, cross-provider history, and independent per-workspace selection. |
| P5 | In progress | Modal bindings, deterministic rules, bounded hook policy, IPC framing, bounded subscriptions, owner-only Unix IPC transport, private per-configuration sockets, status/config status routing, window/workspace queries, configured layout selection, camera transactions, offline `horyctl config check`, and online `horyctl status|windows|workspaces|layout select ID|camera set X Y ZOOM` exist. Add configuration reload, pending spawn, and configuration-backed rules/hooks. |
| P6 | Not started | Nested Wayland adapter, minimal xdg-shell lifecycle, OpenGL renderer, configure/commit and buffer-release correctness. |
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
