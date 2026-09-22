# Horyzond resume point

Last completed phase: **P1 — First-run bootstrap, session logs, and crash diagnostics**.

Last local commit: `feat(p1): bootstrap configuration and diagnostics`.

## Resume next

Start **P2 — Modular Lua configuration and safe live reload**. Add `wm-script` and extend `wm-config` with sandboxed Lua 5.4, the documented `source()` API, dependency graph and import-cycle detection, schema validation, candidate generations, file watching, debounce, and atomic reload commits.

Keep P1's current first-run files as the one source of shipped defaults. Do not introduce a second set of hardcoded Rust configuration values. Reload must preserve the prior valid generation and must exclude logs, crashes, runtime state, and locks from watched paths.

## Phase checklist

| Phase | Status | Remaining work |
| --- | --- | --- |
| P0 | Complete | No functional work. Revisit contracts only when real P6 frame ownership proves an adjustment necessary. |
| P1 | Complete after the pending verification and commit | P1 has bootstrap, an exclusive runtime lock, structured `latest.log`, previous-log archival, explicit crash reports, and a process panic hook. A durable unclean-session marker and bounded background log writer are deferred to P10 hardening. |
| P2 | Not started | `mlua` sandbox, `source()` dependency graph, schema validation, file watching, debounce, candidate reload generations, and recovery. |
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
