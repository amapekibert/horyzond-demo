# Horyzond resume point

Last completed phase: **P3 — Headless runtime, workspace state, camera, and scene**. P4 is actively in progress.

Last completed step: `feat(p4): separate stacking paint order`.

## Resume next

Continue **P4 — Four independent Lua layout profiles**. `wm-layout` has independent native recovery engines and `LuaLayout` executes versioned profile contracts with memory and instruction limits, validates their rectangles, and falls back safely; `wm-core` has multiple independent workspaces and applies the selected provider through `LayoutProviders`. Layout state is versioned JSON containing only active profile and profile geometry; it rejects unknown versions, discards unmapped windows during restore, and can reapply a new provider while preserving compatible data. STACKING now has an explicit raise operation whose paint order remains independent from focus. Next, implement the missing profile-specific state and behavior: SPATIAL navigation/camera intent, STACKING transient layers, TILING gaps/ratios and split trees, and SCROLLING columns/rows/reveal policy.

## Phase checklist

| Phase | Status | Remaining work |
| --- | --- | --- |
| P0 | Complete | No functional work. Revisit contracts only when real P6 frame ownership proves an adjustment necessary. |
| P1 | Complete | P1 has bootstrap, an exclusive runtime lock, structured `latest.log`, previous-log archival, explicit crash reports, a process panic hook, and an unclean-session marker. A bounded background log writer is deferred to P10 hardening. |
| P2 | Complete | Lua 5.4 is restricted to table/string/math/UTF-8 libraries; imports stay beneath the config root, are cached per candidate, and are polled for changes. OS notification debounce and event-loop integration remain future work. |
| P3 | Complete | `wm-core` manages deterministic map/unmap, focus, world geometry, and outputs. `wm-scene` handles camera projection/inversion, picking, snapshots, and coalesced damage. Desired-versus-committed configure state and real event-loop dispatch remain for later protocol work. |
| P4 | In progress | Native fallback engines, versioned Lua contracts, and a restricted validated `LuaLayout` runtime cover SPATIAL, SCROLLING, TILING, and STACKING; active-profile application, per-profile geometry retention, independent multi-workspace scenes, and STACKING's explicit paint order are wired into core. Configuration resolves and watches canonical provider paths, and the composition root selects a configured provider or independent native fallback. Versioned data-only JSON state and compatible provider migration are implemented. Profile-specific interaction and placement state remain. |
| P5 | Not started | Modal input, rules, hooks, versioned IPC, and `horyctl`. |
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
