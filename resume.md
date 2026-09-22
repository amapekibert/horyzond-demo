# Horyzond resume point

Last completed phase: **P2 — Modular Lua configuration and safe live reload**.

Last local commit: `feat(p2): add safe Lua configuration reload`.

## Resume next

Start **P3 — Headless runtime, workspace state, camera, and scene**. Add `wm-core` and `wm-scene`, deterministic command/event processing, workspace and focus lifecycle, desired versus committed geometry, camera forward/inverse transforms, picking, scene snapshots, and baseline damage tracking.

P2 currently supplies a bounded Lua 5.4 loader, relative root-confined `source()` imports, cycle detection, memory/instruction limits, candidate generations, and dependency polling. Integrate it with a real event-loop watcher and configuration schema validation when P3 introduces a coordinator; preserve its transactional last-valid-candidate behavior.

## Phase checklist

| Phase | Status | Remaining work |
| --- | --- | --- |
| P0 | Complete | No functional work. Revisit contracts only when real P6 frame ownership proves an adjustment necessary. |
| P1 | Complete | P1 has bootstrap, an exclusive runtime lock, structured `latest.log`, previous-log archival, explicit crash reports, a process panic hook, and an unclean-session marker. A bounded background log writer is deferred to P10 hardening. |
| P2 | Complete after the pending verification and commit | Lua 5.4 is restricted to table/string/math/UTF-8 libraries; imports stay beneath the config root, are cached per candidate, and are polled for changes. Schema validation, OS notification debounce, and event-loop integration await P3. |
| P3 | Not started | `wm-core`, deterministic lifecycle/workspace state, camera/scene transforms, damage, and recording-renderer scenarios. |
| P4 | Not started | Lua provider contract; SPATIAL, STACKING, TILING, and SCROLLING implementations; state migration and fallback placement. |
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
