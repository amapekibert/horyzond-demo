# Horyzond implementation plan

Status: planning only; implementation has not started.

This plan develops [horyzondRust-architecture.md](horyzondRust-architecture.md) into ordered, testable milestones. P0, P1, and subsequent numbers indicate implementation order and dependency gates, not severity labels. The architecture document remains the design input; the explicit refinements below resolve ambiguities before coding.

## 1. Product requirements

- Implement a Rust window manager and compositor with KISS design and independently testable modules.
- Support independent, runtime-selectable Lua layout providers. Ship SPATIAL, SCROLLING, TILING, and STACKING only as editable examples; the WM core must not recognize their names or behavior.
- Deliver Wayland and OpenGL first. Preserve explicit extension points for native X11 and Vulkan from P0, without pretending that every backend has identical capabilities.
- Keep user policy in editable Lua files: settings, bindings, rules, layout algorithms, hooks, and theme declarations. Keep shader source in external shader files referenced by configuration.
- Use `~/.config/horyzond/` as the default user directory, exactly as requested. Resolve `~` from the user's home directory, independently of the working directory. A deliberate `--config-dir` override may support testing and additional instances; do not silently redirect the default through `XDG_CONFIG_HOME`.
- Create the directory and all shipped default configuration files automatically when absent. Never overwrite user edits during normal startup or reload.
- Support relative `source("file.lua")` imports, nested imports, automatic reload, explicit reload, validation, and rollback without terminating clients.
- Write a detailed log of each active session to `~/.config/horyzond/latest.log`, preserve earlier sessions, and create crash reports.
- Do What The Fuck You Want To Public License (`WTFPL`). Keep all code, configuration comments, documentation, diagnostic messages, and CLI output in English.

## 2. Architecture decisions to settle in P0

### 2.1 Independence without distributed-system complexity

Use a Cargo workspace with a small crate per meaningful responsibility. Run the compositor in one process with one primary event-loop thread. Independence means explicit contracts, private module state, a directed dependency graph, and replaceable implementations. It does not require a process, thread, global event bus, or dynamic plugin loader for every module.

The entrypoint is the composition root: it selects adapters and connects them to the coordinator. `wm-core` owns window/workspace lifecycle and coordinates typed operations. Configuration, layout state, scene caches, protocol objects, GPU resources, and logging remain owned by their respective modules. No module receives unrestricted access to another module's mutable state.

Use synchronous calls for inexpensive deterministic work. Use bounded workers only for work that would block the event loop, initially configuration loading and log writing. Avoid an ECS, dependency-injection framework, universal service registry, network services, and Rust dynamic-library ABI in the initial implementation.

### 2.2 Separate display protocols, rendering, and presentation

Refine the original `DisplayBackend::present(GraphicBuffer)` sketch into separate roles:

| Contract | Responsibility | Must not assume |
| --- | --- | --- |
| Window-system adapter | Client lifecycle, input/output events, configure/focus/close operations, capability reporting | Wayland objects, a local GPU, or compositor-owned presentation |
| Renderer | Import supported content, consume immutable scene snapshots, produce frame results | DRM ownership, Wayland surfaces, GL texture IDs in shared types |
| Presenter | Acquire render targets, submit frames, report completion, manage output/device timing | One specific graphics API or universal DMA-BUF support |
| Layout provider | Read normalized window data and produce validated layout updates | Protocol resources, renderer state, IPC transport, or other layout implementations |

These are conceptual contracts first, not a commitment to overly generic trait signatures. Prove them using headless adapters and a recording renderer before stabilizing interfaces.

Define capability negotiation for compositing, camera scaling, buffer imports, effects, synchronization, presentation, and optional protocols. Reject unsupported combinations clearly. Native non-compositing X11 can manage geometry and stacking without a renderer; it cannot promise compositor zoom or shader effects. Xwayland inside a Wayland session is a separate feature from running Horyzond as a native X11 WM. Running a nested Wayland compositor in an X11 host window is also a separate mode.

The initial renderer is OpenGL ES through an isolated adapter. Vulkan is a later implementation with potentially different pipelines, buffers, synchronization, and shader artifacts, not a renamed GL implementation.

### 2.3 Shared types and ownership

Keep the neutral vocabulary small: stable IDs, coordinate-space types, window metadata, commands, events, capabilities, immutable snapshots, and validated updates. Native resources stay in adapters behind opaque handles with explicit lifetime/release rules.

Separate desired geometry, protocol configure-in-flight state, and committed client content. A layout result is an intent; clients may acknowledge asynchronously or constrain their size. Adapter-specific serials remain inside adapters.

The coordinator enforces cross-module invariants and orders events; it does not absorb layout algorithms, protocol handlers, file watchers, shader compilation, or log formatting. Use bounded queues with documented overflow behavior and deterministic ordering. Give asynchronous results generation IDs so stale results cannot overwrite newer state.

### 2.4 Coordinate contract: apply the camera exactly once

The original Lua SPATIAL example returns camera-projected coordinates despite describing persistent world coordinates. Replace that ambiguity with this contract:

- Profiles return workspace/world geometry, never already-projected renderer rectangles.
- Scene preparation transforms world coordinates into output-local logical coordinates exactly once.
- Output scale and transform then convert logical coordinates into physical pixels.
- Input follows the inverse path through output, camera, and surface-local transforms, including input regions and subsurfaces.
- SPATIAL stores persistent positions and a free camera. SCROLLING stores ordered columns and a ribbon offset. TILING uses the work area with a canonical camera. STACKING uses manually controlled geometry with a canonical camera.
- Keep world positions in `f64`; subtract the camera origin before converting to GPU precision. An "infinite" canvas means no fixed desktop bounds, not arbitrary numerical precision.
- Validate finite positions, positive sizes, positive bounded zoom, and conversion limits. Defer camera rotation until an actual use case justifies it.

### 2.5 External configuration with reliable recovery

Ship readable defaults in `config/` and package the same files with the executable or installation. An embedded copy of those exact assets may provide first-run and recovery defaults. This is a distribution mechanism, not a second hidden set of Rust policy constants. User configuration remains authoritative; Rust owns schema validation, safety limits, and minimal emergency recovery behavior.

Do not maintain four feature-complete Lua algorithms plus four feature-complete Rust duplicates. Start with the four Lua providers and one small safe Rust placement fallback. A script failure preserves the last valid result where possible; a new window still receives usable placement. This deliberately simplifies the architecture document's full native fallback proposal.

### 2.6 Provider-defined layout rule

`wm-core` and `wm-layout` must never contain an enum, match arm, fallback branch, or policy switch for `spatial`, `scrolling`, `tiling`, `stacking`, or any other named layout. The core stores an opaque `LayoutId` and invokes a neutral layout-provider contract. The layout runtime discovers every provider name from `layouts` in the active configuration and validates only the provider protocol: API version, matching opaque ID, resource limits, complete geometry, ordering, and state schema.

The bundled Lua layouts are first-run examples. Users may remove them, rename them, replace them, or add layouts such as `monocle`, `bsp`, or `my_layout` without a Rust code change. The only Rust fallback is generic recovery placement that preserves known valid rectangles and gives new windows usable rectangles. It must not emulate a named layout.

```lua
settings = { default_layout = "my_layout" }
layouts = { my_layout = "layouts/my_layout.lua" }
```

Each provider declares `layout = { id = "my_layout", api_version = 1 }` and implements the documented callbacks. Provider-specific settings, navigation, state, and behavior remain inside that provider's Lua files. The WM validates and applies data; it does not interpret provider semantics.

### 2.7 Initial dependency policy

Prefer maintained building blocks over hand-writing a Wayland protocol stack. Evaluate and pin Smithay behind Wayland/GL adapters, `mlua` with Lua 5.4 for scripting, an event-loop library such as `calloop`, filesystem notifications, and structured tracing. Verify the selected versions and licenses during P0 rather than declaring untested version requirements here.

Smithay documents separate renderer abstractions and implementations; use those inside adapters without exporting Smithay types into neutral contracts. See [Smithay renderer documentation](https://docs.rs/smithay/latest/smithay/backend/renderer/index.html).

Lua instruction hooks and memory limits are available through `mlua`; verify limits cover the actual exposed execution paths. See [mlua Lua API documentation](https://docs.rs/mlua/latest/mlua/struct.Lua.html). Hooks alone do not bound a blocking native callback, so expose only bounded host operations and disallow arbitrary native modules.

## 3. Target module layout

Create crates when their implementation milestone begins; avoid a workspace full of speculative empty crates.

| Crate / location | Owns | Allowed relationships |
| --- | --- | --- |
| `wm-types` | IDs, geometry, capabilities, normalized commands/events | No platform, Lua, or graphics dependencies |
| `wm-core` | Lifecycle, workspaces, modes, command ordering, small policy coordinators | Neutral types and injected contracts; no concrete adapters |
| `wm-config` | Schema, imports, defaults, file watching, configuration transactions | Neutral data and Lua runtime; no backend or renderer |
| `wm-script` | Bounded Lua runtime and common host API construction | Shared by configuration/layout; no compositor globals |
| `wm-layout` | Generic provider contract, layout-state validation, generic recovery placement | Neutral types and scripting; no concrete platform/render APIs or named layout policies |
| `wm-scene` | Camera, scene snapshots, picking, damage, visual layers | Neutral types; no GL/Vulkan/Wayland objects |
| `wm-backend` | Window-system and presentation contracts, capability descriptors | Neutral types |
| `wm-backend-headless` | Virtual windows/outputs and deterministic event injection | Backend contracts |
| `wm-backend-wayland` | Wayland protocol objects, seat/device integration, nested and DRM hosts | Backend contracts; adapter-local framework integration |
| `wm-backend-x11` | Native X11 management, later optional compositing integration | Backend contracts; added in P11 |
| `wm-render` | Renderer contract, neutral frame descriptions, recording renderer | Neutral types and immutable scene descriptions |
| `wm-render-gl` | GL resources, imports, shader compilation | Render contracts; platform interop remains adapter-local |
| `wm-render-vulkan` | Vulkan resources, synchronization, pipeline compilation | Render contracts; added in P12 |
| `wm-ipc` | Versioned wire schema, socket transport, subscriptions | Neutral commands/events; never a dependency of layout algorithms |
| `wm-cli` | `horyctl` CLI | IPC schema/client; no compositor runtime dependency |
| `wm-diagnostics` | Session log writer, crash metadata, recovery markers | No dependency on compositor internals; receives snapshots |
| `src/main.rs` | Startup/shutdown ordering and composition | Concrete modules selected by build features/runtime options |
| `config/` | Shipped Lua files and renderer-specific shader assets | Plain editable files, not compiled policy code |
| `tests/`, `docs/` | Scenario harnesses, user documentation, short decision records | Organize by behavior and module contracts |

Keep rules, modal input, spawn tracking, and persistence as small internal modules initially. Extract another crate only if its boundary becomes useful. Prohibit cyclic crate dependencies and dependencies from neutral crates onto concrete adapters. Modules emit tracing events through the tracing facade; they do not initialize global logging individually.

## 4. User configuration contract

### 4.1 Shipped directory structure

```text
~/.config/horyzond/
├── config.lua
├── keybinds.lua
├── modes.lua
├── rules.lua
├── theme.lua
├── layouts/
│   ├── spatial.lua
│   ├── scrolling.lua
│   ├── tiling.lua
│   └── stacking.lua
├── rules.d/
│   ├── media.lua
│   └── dev.lua
├── hooks/
│   ├── on_window_open.lua
│   └── on_workspace_change.lua
├── shaders/
│   └── gl/
│       ├── window.vert
│       ├── window.frag
│       └── rounded_border.frag
├── state/
│   └── session.json
├── logs/
│   └── <previous-session-id>.log
├── crashes/
│   └── <session-id>-<timestamp>.json
└── latest.log
```

Runtime state/log/report files are created when needed, not shipped as default configuration. Add `shaders/vulkan/` only when supported. A custom profile is optional rather than an unused generated file. Keep the IPC socket and active-instance lock in a private runtime directory, not in the watched configuration tree.

### 4.2 Import example and semantics

Planned `config.lua` API:

```lua
-- Import other files.
source("keybinds.lua")
source("rules.lua")
source("theme.lua")

-- Default settings; users can edit these values.
settings = {
    default_layout = "spatial",
    reload = { enabled = true, debounce_ms = 150 },
    logging = { level = "trace", detail = "full" },
}

modes = source("modes.lua")
layouts = {
    spatial = "layouts/spatial.lua",
    scrolling = "layouts/scrolling.lua",
    tiling = "layouts/tiling.lua",
    stacking = "layouts/stacking.lua",
}
```

`keybinds.lua`, `rules.lua`, and `theme.lua` assign their named tables in the shared candidate configuration environment; return-value modules such as `modes.lua` can also be assigned explicitly. This is one documented convention supporting the user's bare `source(...)` example without an implicit merge system. Later assignments replace earlier values; table extensions must be explicit. Report conflicting bindings and invalid fields during validation.

- Resolve a relative import against the directory of the file containing `source`, not the process working directory.
- Initially allow imports within the configuration root, including subdirectories. Reject root escapes after path/symlink resolution; document this boundary.
- Execute each resolved file once per candidate load, cache its return value, and detect import cycles with the complete import chain.
- A missing import is a candidate error. Diagnostics include filename, line, import chain, and Lua traceback.
- Build a dependency graph for imported configuration, registered layout scripts, hooks, and referenced shaders.
- Evaluate configuration in a fresh sandbox. No `os`, arbitrary `io`, native module loading, unrestricted `require`, process spawning, or compositor mutations during evaluation. Start with no user-created coroutines unless quotas are enforced for all execution contexts.
- Launch commands are declarative action data, executed by the coordinator after a user action or a specific lifecycle hook. Reload must never replay autostart automatically.
- Configuration tables and layout state have explicit schema versions. No compiled-in bindings or application-specific rules beyond documented generic emergency recovery.

### 4.3 Bootstrap and transactional reload

1. Resolve the configuration directory and acquire an instance lock before touching session logs or defaults.
2. Create missing directories and install absent shipped defaults using exclusive/atomic writes. Existing files remain byte-for-byte unchanged. Complete an interrupted first-run installation on the next startup.
3. Initialize diagnostics before evaluating user scripts. Log every created path and initialization failure.
4. Read a candidate configuration and its dependency graph. Bound file sizes, import depth, Lua instructions, allocations, and host-call work.
5. Validate schema, bindings, rules, profile contracts, capabilities, and referenced assets. Prepare resources such as shaders before changing active state.
6. Commit one validated generation at a safe event-loop boundary. Preserve live clients, workspace membership, camera state, and compatible profile state. Release old resources only after active users/GPU work finish.
7. On error, keep the previous configuration and resources and report why. On invalid initial configuration, start with known-good packaged defaults in an explicitly reported recovery mode without replacing broken user files.
8. Watch dependencies and their parent directories to catch editor atomic-save renames, deletion, recreation, and newly referenced files. Debounce bursts; discard superseded candidate generations. Detect changes during a load and retry rather than commit a mixed candidate.
9. Exclude `latest.log`, `logs/`, `crashes/`, `state/`, lock files, and temporary files from reload triggers. Rewatch after configuration-directory replacement. A missing root/default file triggers the same non-destructive repair process; a missing custom import reports an error and preserves the active generation.
10. Support `horyctl config check`, `reload`, and `status`, exposing generation, last result, and diagnostics. Provide polling/manual reload fallback when filesystem notifications fail.

Bindings, modes, rules, profile settings/scripts, themes, hooks, shader assets, and logging filters are reloadable. Backend selection, graphics API, and process-level options may require restart: reject such a mixed candidate atomically with a clear restart-required diagnostic instead of partially applying it. Runtime output modes have their own tested apply/revert transaction once supported. Do not claim that a configuration transaction can roll back an already-spawned external process.

## 5. Diagnostics and crash contract

- Start each session with a unique ID, UTC timestamp, monotonic origin, executable/build identity, selected features, OS/device details where available, config path/generation, and negotiated capabilities.
- Archive the previous `latest.log` before opening the new session log. Keep the entire active session in `latest.log`; do not rotate away its beginning by default. Make retention of archived logs configurable.
- Permit one active instance per configuration directory so concurrent sessions cannot overwrite `latest.log`. Additional test/nested sessions use an explicit alternate configuration directory.
- Use structured single-line records with sequence number, severity, module, event/span ID, thread, duration, and relevant window/workspace/output/frame/config-generation IDs. Add CLI formatting/filtering later without changing the canonical data.
- Default to detailed TRACE instrumentation: initialization, client lifecycle, input routing decisions, mode transitions, rule matches, layout decisions, geometry changes, camera changes, configure/commit progress, buffer imports/releases, damage, frame timing, presentation, IPC, reload decisions, and shutdown.
- "Full detail" means full instrumented WM diagnostics. Do not silently log typed text, clipboard contents, passwords, environment secrets, or client pixel contents. Raw protocol/input payload capture is a separate explicit diagnostic option. Escape untrusted strings and bound record sizes.
- Use a dedicated writer with bounded buffering, periodic flush, flush on errors, and shutdown drain. Record saturation and dropped-record counters explicitly. A strict blocking lossless mode may be selectable for diagnosis, with the latency tradeoff documented. Default full-detail logging does not silently sample events, but cannot guarantee zero loss under arbitrary overload.
- Keep the last diagnostic events and a small prebuilt state snapshot available for crash reporting. Panic reports include payload/location, backtrace, thread/session/build IDs, active configuration generation, and recent operations. Write to `crashes/` and reference the report in `latest.log`.
- Prevent recursive logger failures. If the directory is unwritable or disk space is exhausted, emit a direct stderr diagnostic and continue with bounded emergency buffering where possible; never claim the log file exists when it could not be created.
- Use an unclean-session marker to report a likely crash on the next startup. For fatal signals, use a minimal signal-safe path and optional OS core dumps; do not allocate, lock, or run Lua inside a fatal signal handler.
- Explain the physical limit: SIGKILL, power loss, kernel failure, aborts, and storage failure can prevent final records or backtraces. A panic hook is not universal crash recovery. Do not continue normal compositor execution after arbitrary internal corruption.
- User-selected reduced logging is allowed and reflected in the session header. The active-log path remains the requested path under the selected configuration root.

## 6. Profile design based on the local examples

These are behavior and data-structure references, not code to transplant. Write original implementations for the project's `0BSD` code; retain reference repositories' own licenses and exclude them from release packages/build inputs.

| Profile | Local references inspected | Horyzond model and behavior |
| --- | --- | --- |
| SPATIAL | [hevel README](examples/SPATIAL/hevel/README.md), [scroll.c](examples/SPATIAL/hevel/src/scroll.c), [zoom.c](examples/SPATIAL/hevel/src/zoom.c), [vxwm infinite tags](examples/SPATIAL/vxwm-main/vxwm/modules/infinitetags/infinitetags.c) | Persistent per-window world rectangles; per-workspace camera; free pan/zoom, home/overview, directional focus, and rectangle-based spawning. Camera movement never rewrites every saved world rectangle. |
| SCROLLING | [niri layout model](examples/SCROLLING/niri/src/layout/mod.rs), [scrolling space](examples/SCROLLING/niri/src/layout/scrolling.rs) | Ordered columns, windows within columns, focused column/window, width policy, and camera offset. Focus reveals the target; inserting a column does not shrink all unrelated columns to fit one screen. Preserve workspaces across output changes. |
| TILING | [dwm tile](examples/TILING/dwm/dwm.c), [sway container model](examples/TILING/sway/include/sway/tree/container.h) | Independent master-stack and split-tree strategies; explicit membership, orientation, ratios, and focus. Start master-stack, then a binary split tree. Sway illustrates hierarchical containers, not a requirement to copy its entire layout system or call it BSP. |
| STACKING | [labwc placement](examples/STACKING/labwc/src/placement.c), [interactive movement](examples/STACKING/labwc/src/interactive.c), [Openbox stacking](examples/STACKING/openbox-3.6.1/openbox/stacking.c) | User rectangles, saved restore geometry, focus history, and explicit z-order/layers. Move/resize, raise/lower, maximize/minimize/restore, parent/transient ordering, and usable-area placement. |

Each workspace has one active profile and separate saved state for previously used profiles. No profile imports another. Common geometry helpers are acceptable; hidden shared policy is not. Switching profiles reconciles live window IDs, initializes newcomers, discards closed IDs, and preserves profile-specific state for a later return.

Refine the profile interface to include metadata/API version, initialization, bounded event handling, layout calculation, and serializable state. Inputs are immutable snapshots plus explicit profile settings. Outputs may request rectangles, ordering, camera intent, and profile state; the host validates all of them. A rectangle-only map cannot describe column membership or a split tree, so those live in versioned provider state.

Keep protocol mechanics, frame scheduling, validated state ownership, and numeric helpers in Rust. Keep shipped layout algorithms and configurable behavior in Lua. Returning to a broken profile retains its last valid provider/result or uses the safe fallback. A profile's failure must not disable other workspaces.

## 7. Ordered implementation milestones

### P0 — Architecture agreement and minimal workspace foundation

**Depends on:** this plan being reviewed before coding begins.

**Work:**

- Record short decisions for ownership, coordinate spaces, capabilities, Lua API, reload atomicity, logging guarantees, and native X11 versus Xwayland.
- Convert the starter package into a workspace with only the foundational crates needed now. Preserve existing user work and the reference architecture document.
- Set package names/binaries (`horyzond`, `horyctl`), Rust edition/MSRV, formatting/lint policy, and shared `license = "0BSD"`. Keep the existing BSD Zero Clause license text.
- Define neutral IDs, errors, event/command vocabulary, and contract sketches. Keep device buffer/sync details provisional until P6 evidence.
- Add feature boundaries for headless, Wayland, GL, and later X11/Vulkan. A headless build must not pull in GPU/display system libraries.
- Verify dependency choices and licensing; document that examples are references excluded from shipped artifacts.

**Exit gate:** foundational workspace builds; basic format/lint/test checks pass; dependency inspection confirms no platform APIs in neutral crates; each module's owner and failure behavior are documented.

### P1 — First-run bootstrap, session logs, and crash diagnostics

**Depends on:** P0.

**Work:**

- Implement configuration path resolution, instance locking, directory creation, and non-destructive installation of the complete shipped default set.
- Add the early structured logger, previous-session archival, retention policy, stderr fallback, session header/footer, and unclean-session markers.
- Add panic reporting, backtrace capture, emergency reporting, and writer shutdown behavior before complex runtime code exists.
- Package defaults from the same source files used by development installs. Record their template/schema version without overwriting user files on upgrades.

**Exit gate:** a fresh temporary home gets all defaults and `latest.log`; a second launch preserves edits and archives the earlier log; concurrent startup cannot clobber files; injected panic creates a correlated crash report; read-only/disk-write failures are reported without recursive crashes. No actual user home is modified by tests.

### P2 — Modular Lua configuration and safe live reload

**Depends on:** P1.

**Work:**

- Implement the documented `source()` API, named configuration tables, schema versioning, import graph, cycle detection, and actionable diagnostics.
- Add bounded Lua execution and validation of settings, bindings, rules, profile descriptors, and hooks.
- Implement file notification/debounce, candidate generations, prepare/commit/abort, startup recovery, and excluded runtime paths.
- Define a reload-participant contract now; wire scene/layout/renderer participants when those modules exist. Do not claim a shader is reloadable before a renderer can validate it.
- Add an offline configuration-check entrypoint before IPC exists; later reuse it in `horyctl config check`.

**Exit gate:** valid edits atomically change the active generation; invalid syntax, import cycles, missing imports, runaway scripts, memory limits, atomic editor saves, and directory recreation preserve a usable runtime. Log writes never cause reload loops. Reload causes no process spawn side effects.

### P3 — Headless runtime, workspace state, camera, and scene

**Depends on:** P0–P2.

**Work:**

- Implement deterministic command/event processing with injected clock and headless windows/outputs.
- Add lifecycle, workspace assignment, focus, output work areas, desired/committed geometry separation, and state snapshots.
- Implement world/logical/physical coordinate types, forward/inverse camera transforms, surface-local picking, scene layers, clipping, and baseline damage tracking.
- Begin with straightforward scene traversal and full redraw fallback. Add spatial indexing only after measurement.
- Add the recording renderer and a non-presenting backend path to exercise architectural independence without a GPU.

**Exit gate:** synthetic map/focus/move/unmap/output events produce deterministic snapshots; transform round trips and zoomed hit tests pass; closed windows cannot remain focused; two outputs with differing scales are handled; camera movement leaves world geometry intact; the runtime builds without Wayland/GL.

### P4 — Provider-defined Lua layouts

**Depends on:** P2–P3.

**Work:**

- Implement provider loading, state serialization, bounded callbacks, output validation, last-good behavior, and the minimal Rust recovery placement.
- Define opaque provider IDs, versioned metadata, bounded callbacks, validated output, generic recovery placement, and data-only state migration. Never store Lua closures in persistent state.
- Ship SPATIAL, STACKING, TILING, and SCROLLING as independent example providers, each with its own Lua state and behavior. Their names and behavior are not part of the Rust API.
- Allow users to add, remove, rename, and select providers through configuration without recompiling the WM.
- Implement per-workspace layout selection and compatible state migration during script reload.

**Exit gate:** arbitrary third-party provider IDs pass the same lifecycle/contract scenarios as shipped examples; switching layouts preserves clients and provider state history; malformed geometry/unknown IDs/script timeouts cannot corrupt the scene; one broken provider does not stop other providers.

### P5 — Modal interaction, rules, hooks, IPC, and CLI

**Depends on:** P2–P4.

**Work:**

- Implement Normal, Insert/Passthrough, Move, Resize, Layout Select, and pending-spawn modes with configured transitions, visible status, cancellation, and recovery bindings.
- Track consumed key/button presses and their releases consistently across mode/focus changes; avoid stuck modifiers and leaking modal commands to clients.
- Implement ordered window rules with explicit priority/stop behavior, metadata-change reevaluation, and no recursive rule-trigger loops. Use bounded matching and cache compiled patterns.
- Implement versioned JSON IPC with request IDs, framed messages, size limits, same-user access, bounded subscriptions, and deterministic errors.
- Implement `horyctl status`, window/workspace queries, profile selection, camera commands, configuration check/reload/status, and pending spawn.
- Represent process launch as executable plus argument array. Shell interpretation must be an explicit action, not an automatic concatenation of user input.
- Dispatch hooks after committed events with quotas and recursion limits; distinguish startup hooks from reload hooks.

**Exit gate:** headless end-to-end scenarios drive all four profiles through CLI commands and modal events; malformed/slow IPC clients cannot stall the coordinator; rules are deterministic; reload updates bindings without stuck input or duplicate autostart.

### P6 — First visible Wayland session with OpenGL

**Depends on:** P3–P5. Deliver backend and renderer together as one visible vertical slice.

**Work:**

- Integrate a nested Wayland host first so development does not require controlling a physical seat.
- Implement a minimal real client lifecycle: compositor/subcompositor surfaces, shared-memory content, `xdg-shell` toplevels/popups, configure/acknowledgment/commit ordering, seats, keyboard/pointer focus, and cursor display.
- Implement GL scene rendering, surface trees, clipping, configured borders/background, texture lifetime, and baseline output frame scheduling.
- Refine the neutral frame contract using real requirements: target acquisition, supported formats, content import, frame completion, ownership, and release. Keep native buffer/fence representations adapter-local.
- Honor frame callbacks and buffer release ordering. Separate rendering completion from presentation completion; never release buffers still in use.
- Add asynchronous client configure transactions with timeout policy so slow clients cannot freeze the session.
- Prefer correct full redraw initially; measure before enabling incremental rendering.

**Exit gate:** real applications map, draw, accept input, resize, and close in a nested session; all four profiles work; an invalid live config preserves mapped clients; nested host shutdown is orderly; headless tests still run without graphics dependencies.

### P7 — Standalone Wayland on DRM/KMS and real input devices

**Depends on:** P6.

**Work:**

- Add session/seat acquisition, DRM/KMS output handling, GBM/EGL integration, and libinput translation through the Wayland adapter.
- Handle output modes, scale/transform, hotplug, VT switching, suspend/resume, input device add/remove, and session deactivation.
- Add multi-output workspace migration and restoration policy; do not discard spatial positions when a monitor disappears.
- Negotiate client DMA-BUF import formats/modifiers and synchronization; preserve supported fallback paths rather than assuming every buffer is importable.
- Ensure presenter teardown restores device/session state on normal exit and recoverable initialization failures.

**Exit gate:** a standalone session supports keyboard/pointer input and multiple outputs; unplug/replug, VT switching, suspend/resume, and renderer initialization failures are exercised on hardware; headless/nested paths continue to work.

### P8 — Desktop integration and spatial spawn workflow

**Depends on:** P5–P7.

**Work:**

- Add layer-shell surfaces, exclusive work areas, decoration negotiation, foreign-toplevel integration, clipboard/data device, drag-and-drop, activation policy, and idle/session-lock integration with documented protocol coverage.
- Treat lock state as a security boundary: no client input or screen exposure while locked, including hotplug and locker failure paths. Do not advertise unsupported lock semantics.
- Implement pending-spawn selection, world-coordinate preview rectangle, configurable screen-space minimum threshold, cancel/timeout, and non-SPATIAL fallback placement.
- Associate a launch with its window using a bounded launch token and protocol/process metadata where available. `app_id` alone is not unique; define ambiguous, delayed, multi-window, failed, and single-instance launch behavior.
- Send requested initial size through the backend's normal configure flow. Avoid an initial placement jump where possible, but do not promise that clients will honor an exact size or map synchronously.
- Define launcher integration through `horyctl`; layer-shell alone does not make third-party launchers emit Horyzond IPC commands.

**Exit gate:** a launcher/panel works; exclusive zones affect applicable layouts; clipboard and drag-and-drop work; a window can be launched into a selected rectangle; failed/ambiguous launch matching cannot move an unrelated window; lock behavior passes dedicated scenarios.

### P9 — Rendering correctness, shader reload, and performance

**Depends on:** P6–P8.

**Work:**

- Add external shader selection and prepare/validate/swap reload with last-good fallback and delayed destruction of in-flight GPU resources.
- Implement damage for old/new bounds, movement, camera changes, popups, effects, scaling, and output changes. Effects declare expanded damage or request full redraw.
- Add fractional-scale handling and document the difference between compositor camera zoom and client buffer rasterization; zoom alone cannot guarantee crisp text at every scale.
- Add presentation timing, frame pacing, measured culling, and unnecessary-work elimination.
- Add direct scanout only after correctness is established, guarded by output/buffer compatibility, transform, synchronization, occlusion, effects, overlays, and cursor conditions. Fall back to composition safely.
- Define renderer-neutral effect identifiers/parameters with backend-specific source artifacts; do not make arbitrary GL shader text a universal shader contract.

**Exit gate:** invalid shaders never replace working pipelines; visual comparisons catch stale damage; continuous pan/zoom and resize have measured timings; direct scanout enters/exits safely; TRACE logging overhead is measured and reported alongside frame latency.

### P10 — Persistence and release hardening

**Depends on:** P1–P9.

**Work:**

- Persist versioned workspace/profile/camera state with atomic writes, bounded size, backup/recovery, and explicit migration. Do not save live protocol IDs as durable identities.
- Restore geometry with best-effort application identity matching and explicit ambiguity handling; restarting the WM does not resurrect application processes automatically.
- Run soak and failure tests covering config storms, malformed scripts, slow clients, output removal, writer overload, disk failure, and repeated startup/shutdown.
- Document installation/session launch, config API, imports, profiles, reload semantics, recovery, log format, crash collection, and known unsupported protocols.
- Add packaging, feature-build checks, dependency/license inventory, and a release checklist. Exclude example repositories and personal logs from distributable artifacts.

**Exit gate:** a documented Wayland/OpenGL release candidate supports all four profiles and the requested config/log behavior; recovery paths are reproducible; performance baselines and known limitations are published.

### P11 — X11 application compatibility and native X11 WM

**Depends on:** P6–P10 for delivery; compatibility contracts already exist from P0.

- **P11.1 Xwayland:** manage optional server lifecycle inside the Wayland session; bridge managed/unmanaged windows, transient relationships, selections, activation, and coordinate conversion. Reuse core/profile policy. A failed Xwayland instance must not terminate native Wayland clients.
- **P11.2 Native X11:** implement WM ownership, map/configure/unmap/destroy handling, input grabs, output changes, ICCCM/EWMH behavior, focus, workspaces, and stacking through an isolated Rust X11 adapter.
- Start native X11 without compositing: TILING, SCROLLING, STACKING, and spatial panning use real window management; scaled SPATIAL overview and shaders return an explicit unsupported capability.
- **P11.3 Optional X11 compositing:** if full visual parity is required, add Composite/Damage content acquisition and a presenter that can reuse the GL/Vulkan renderer contract. Track this as distinct work, not an implicit feature of P11.2.

**Exit gate:** X11 applications function through Xwayland; native X11 tests run under a nested X server; the same core/profile tests pass against both adapters; capability reporting accurately describes visual limitations. Full zoom/effect parity is gated on P11.3.

### P12 — Vulkan renderer

**Depends on:** P9–P10 and proven render/presentation contracts; may proceed independently of P11 implementation.

**Work:**

- Add an isolated Vulkan implementation, evaluating a Rust binding such as `ash` at implementation time.
- Implement device/queue selection, image import/upload, format/modifier negotiation, render targets, descriptors, pipelines, synchronization, and frame completion.
- Add Vulkan shader artifacts, compilation/reflection/validation, pipeline caching, and transactional shader reload. Do not assume existing GL shader files can be used unchanged.
- Negotiate backend/presenter interoperability and reject unsupported device/format combinations with a useful diagnostic.
- Handle device loss and resource cleanup with a clear recovery/exit policy; backend selection remains a documented startup choice unless live switching is separately implemented.

**Exit gate:** the same scene/profile scenarios render correctly through GL and Vulkan; image comparisons and synchronization/lifetime tests pass on supported hardware; no renderer-specific change is required in Lua profiles or core policy.

## 8. Validation and milestone completion policy

Use tests where failure would break user sessions or architectural contracts. Avoid large test suites that merely duplicate simple implementation details.

| Area | Required evidence |
| --- | --- |
| Module independence | Headless-only builds, dependency graph checks, alternate renderer/backend contract scenarios |
| Configuration | Temporary-home bootstrap, import/reload/error/recovery scenarios, no file overwrite, no reload feedback loops |
| Geometry and profiles | Coordinate properties, finite geometry, stable membership, tiling non-overlap, scrolling order, stacking order, spatial persistence |
| Input and commands | Press/release ownership, cancellation, focus changes, deterministic rules, bounded IPC |
| Protocol and graphics | Real nested clients, configure/commit ordering, buffer release, popups/input transforms, supported format negotiation |
| Hardware | DRM hotplug, mixed scale, VT switch, suspend/resume, device loss/failure observations |
| Diagnostics | Session archival, correlated panic reports, unclean markers, writer overload, disk-error fallback, honest lost-record reporting |
| Performance | Frame timings and input-to-present measurements with full-detail logging enabled and disabled, plus long-session memory/log growth |

CI should run neutral/headless checks by default, nested graphics checks where available, and retain hardware validation as an explicit release gate. Establish measured budgets during P6–P9 on named hardware instead of promising arbitrary FPS or zero latency now.

Each milestone is complete only when its exit gate passes and its behavior is documented. Future-only capabilities must report unsupported rather than return fake success. Track implementation in small reviewable changes aligned with the substeps above.

## 9. Delivery checkpoints and exclusions

| Checkpoint | Outcome |
| --- | --- |
| P0–P2 | Modular foundation, automatic config generation, detailed logs, safe Lua reload |
| P3–P5 | Headless WM with all four profiles, modes, rules, IPC, and CLI |
| P6 | First usable nested Wayland/OpenGL session |
| P7–P8 | Standalone session and desktop/spatial workflows |
| P9–P10 | Hardened Wayland/OpenGL release candidate |
| P11 | Xwayland and native X11, with explicit compositing capability stages |
| P12 | Vulkan rendering implementation |

The first release does not include a custom panel, launcher, file manager, general desktop environment, remote distributed modules, dynamic native plugins, or a universal shader compiler. External desktop components connect through supported protocols and IPC. Optional capabilities can grow without turning core policy into a platform-specific monolith.

The next coding task, after plan review, is P0. No WM implementation, Cargo changes, or user configuration-directory creation is part of this planning change.
