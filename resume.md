# Horyzond resume point

Last completed phase: **P0 — Architecture agreement and minimal workspace foundation**.

Last local commit: `feat(p0): establish modular foundation`.

## Resume next

Start **P1 — First-run bootstrap, session logs, and crash diagnostics**. Implement `wm-diagnostics` and `wm-config` only as far as bootstrap requires. Use `~/.config/horyzond/` by default and provide an explicit test override. Install only absent default files through atomic/exclusive writes, never overwrite existing user files, create `latest.log`, archive a previous session log, and add session/crash markers.

Before implementation, add the actual shipped `config/` templates that P1 installs. Do not add Lua execution or filesystem watching until P2.

## Phase checklist

| Phase | Status | Remaining work |
| --- | --- | --- |
| P0 | Complete after the pending verification and commit | No functional work. Revisit contracts only when real P6 frame ownership proves an adjustment necessary. |
| P1 | Not started | Config-path resolution, instance lock, safe default installation, structured session logs, archival, crash reports, stderr fallback, and tests using temporary directories. |
| P2 | Not started | `mlua` sandbox, `source()` dependency graph, schema validation, file watching, debounce, candidate reload generations, and recovery. |
| P3 | Not started | `wm-core`, deterministic lifecycle/workspace state, camera/scene transforms, damage, and recording-renderer scenarios. |
| P4 | Not started | Lua provider contract; SPATIAL, STACKING, TILING, and SCROLLING implementations; state migration and fallback placement. |
| P5 | Not started | Modal input, rules, hooks, versioned IPC, and `wmctl`. |
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
- User configuration creation is intentionally absent until P1; no test should modify the real home directory.
