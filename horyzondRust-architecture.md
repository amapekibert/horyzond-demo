# Horyzond — Window Manager Architecture

Universal modal window manager and compositor with Wayland (primary mode) and X11 (legacy fallback) support, OpenGL and extensible Vulkan rendering pipelines, and four independent window layout profiles: SPATIAL, SCROLLING, TILING, STACKING. All layout logic, keybindings, window rules, and shaders reside in external Lua files, hot-reloadable at runtime without restarting the process.

Implementation language: **Rust** (Cargo workspace, separate crate per module). Configuration and scripting language: **Lua** (`mlua`, sandboxed). Primary render pipeline: **OpenGL** (EGL + GLES/GL) with modular abstraction for **Vulkan** renderers.

---

## 1. General Architecture Diagram

```
                               ┌────────────────────────────────────────┐
                               │               wm-ipc                   │
                               │  (Unix Socket, Wayland Protocols, CLI) │
                               └───────────────────┬────────────────────┘
                                                   │ Commands / Events
                                                   ▼
┌────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                           wm-core                                              │
│                    (Event Loop, State Machine, Window Manager Context)                         │
└──────────────┬───────────────────────────┬───────────────────────────┬─────────────────────────┘
               │                           │                           │
               ▼                           ▼                           ▼
┌──────────────────────────┐   ┌───────────────────────┐   ┌─────────────────────────────┐
│        wm-backend        │   │       wm-layout       │   │          wm-scene           │
│ ┌──────────────────────┐ │   │ ┌───────────────────┐ │   │ ┌─────────────────────────┐ │
│ │ Wayland (DRM/libinp) │ │   │ │ SPATIAL           │ │   │ │ Scene Graph Tree        │ │
│ ├──────────────────────┤ │   │ ├───────────────────┤ │   │ ├─────────────────────────┤ │
│ │ X11 Backend (XCB)    │ │   │ │ SCROLLING (Niri)  │ │   │ │ 2D Camera (Zoom/Pan)    │ │
│ ├──────────────────────┤ │   │ ├───────────────────┤ │   │ ├─────────────────────────┤ │
│ │ Xwayland Subsystem   │ │   │ │ TILING (BSP)      │ │   │ │ Damage Tracking (Dirty) │ │
│ └──────────────────────┘ │   │ ├───────────────────┤ │   │ └─────────────────────────┘ │
│                          │   │ │ STACKING (kwin-like)│   │                             │
│                          │   │ └───────────────────┘ │   │                             │
└──────────────┬───────────┘   └───────────────────────┘   └──────────────┬──────────────┘
               │                                                          │
               └───────────────────────────┬──────────────────────────────┘
                                           ▼
┌────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                           wm-render                                            │
│              RenderDevice Trait (OpenGL EGL/GLES & Extensible Vulkan Pipeline)                 │
└────────────────────────────────────────────────────────────────────────────────────────────────┘
```

### Core Philosophy

To unify tiling, scrolling, stacking, and infinite zooming within a single cohesive model: all windows reside on an infinite 2D plane (global coordinates: X, Y, Width, Height). The 2D camera (`Camera2D`) maintains a position `(x, y)` and `zoom`. Each profile interprets these coordinates according to its layout rules:

- **SPATIAL** performs no screen-space recalculation — window geometry *is* persistent world-space coordinates; the camera projects this unbounded canvas onto the display viewport (similar to vxwm, hevel, and Plan 9).
- **SCROLLING** organizes windows in an ordered ribbon; the camera moves along a 1D axis (offset), typically with a fixed zoom.
- **TILING** overrides custom window coordinates — layout algorithms (BSP / master-stack) compute non-overlapping rectangles filling the visible camera viewport.
- **STACKING** grants geometry control to the user: windows are manually moved and resized; stacking order (z-order) is user-managed (similar to KWin or labwc).

During infinite zooming, cursor coordinates on the physical monitor are projected into 2D world-space coordinates via the camera's inverse projection matrix ($Camera^{-1}$).

---

## 2. Four Canonical Profiles

| Profile    | Geometry Ownership                     | Description |
|------------|-----------------------------------------|-------------|
| `SPATIAL`  | Camera + persistent world coordinates   | Infinite Zoom / Spatial Canvas (inspired by **hevel**, **vxwm**, Plan 9/acme): windows freely reside on an infinite 2D canvas with persistent `(x, y, w, h)` coordinates. The camera pans and zooms freely over this canvas; when `zoom < 1.0`, an aerial overview of the workspace is displayed; when `zoom > 1.0`, focus zooms in on specific windows. No rigid layout ordering or fixed pages. |
| `SCROLLING`| Ordered ribbon + camera offset          | Niri / PaperWM style: windows line up in an infinite horizontal (or vertical) ribbon; the camera maintains a 1D offset along the ribbon. |
| `TILING`   | Layout algorithm (BSP / master-stack)  | Layout computes rectangle coordinates ensuring visible output bounds are tiled without overlap. |
| `STACKING` | User control (drag/resize + z-order)    | Traditional desktop model (KWin / labwc style): floating overlapping windows, manual positioning, and interactive z-order / focus stack. |

Each profile is implemented as an independent `.lua` script in `~/.config/horyzond/layouts/`, conforming to a unified contract (§6). Switching profiles for an active workspace is a runtime operation (`horyctl layout set <profile>`) without restarting the compositor.

---

## 3. wm-core (Core & Coordinator)

The central hub owning system state and coordinating modules through events without tight coupling between them.

- **Main Event Loop** — processes system events via `epoll` / `kqueue` / event loops.
- **State Machine (Modal Controller)** — Vim-like finite state machine:
  - `Normal` — window navigation (`h/j/k/l`), camera zoom/pan, workspace switching.
  - `Insert / Passthrough` — all raw key events are forwarded directly to the focused client window.
  - `Resize / Move` — keyboard-driven adjustment of active window dimensions/positions (world coordinates in SPATIAL, screen coordinates in STACKING, BSP split ratios in TILING, ribbon index in SCROLLING).
  - `Layout Select` — quick profile switching (`SPATIAL ↔ SCROLLING ↔ TILING ↔ STACKING`) for the active workspace.
- **Workspace Manager** — virtual workspaces mapped to display outputs; each workspace maintains its active layout profile and its own scene-graph subtree.
- **Spawn-by-Rectangle (SPATIAL-specific)** — interactive application launching mechanism in `SPATIAL` profile:
  1. The user opens an external app launcher (via `wlr-layer-shell`, such as rofi, wofi, or fuzzel — communicating via IPC). The launcher sends `horyctl spawn-pending <app_id> <exec_cmd>` to `wm-core` via `wm-ipc`.
  2. `wm-core` transitions to `SpatialSpawnPending { app_id, exec_cmd }` mode; the cursor changes to a crosshair / "draw rectangle" indicator.
  3. The user holds the right mouse button on an empty canvas area and drags — `wm-core` renders an interactive preview bounding box on the scene overlay from the initial click point to current cursor position, projecting screen pixels into world coordinates via $Camera^{-1}$.
  4. Upon releasing the right button: computed `(world_x, world_y, w, h)` is locked, `wm-core` spawns the application process (`std::process::Command`), and when the client maps its first `xdg_surface`/`xdg_toplevel`, `wm-scene` immediately assigns this defined world rectangle (eliminating jumping artifacts).
  5. Pressing `Escape` during dragging cancels the spawn and returns to `Normal` mode.
  - Minimum drag threshold (e.g. 40×30 screen pixels): if dragged below this threshold, it is treated as a simple click, applying a default centered window size.
  - In non-SPATIAL profiles, `spawn-pending` automatically falls back to layout algorithm placement (`TILING`/`SCROLLING`) or cascading offset (`STACKING`).
- **Rules Engine** (`rules.rs`) — evaluates `rules.d/*.lua`, matching client windows by `app_id` / `title` regex to apply actions (`set_layout`, `float`, `pin_on_top`, `default_zoom`, `apply_shader`, etc.).
- **Hot Reload** (`hot_reload.rs`) — file system watcher monitoring `~/.config/horyzond/`, reloading Lua chunks or recompiling shaders on modification without dropping client sessions or compositor state.

---

## 4. wm-backend (System Environment Abstraction)

Isolates core logic from Wayland and X11 specific protocols.

```rust
pub trait DisplayBackend {
    fn init(&mut self) -> Result<(), BackendError>;
    fn poll_events(&mut self) -> Result<Vec<BackendEvent>, BackendError>;
    fn outputs(&self) -> Vec<OutputInfo>;
    fn present(&mut self, output_id: OutputId, buffer: &GraphicBuffer) -> Result<(), BackendError>;
}
```

- **WaylandBackend** (primary): operates directly over DRM/KMS (atomic modesetting, page-flipping) and `libinput` for mouse/keyboard/touch handling.
- **X11Backend** (legacy / fallback): operates as a classic X11 window manager via XCB / `SubstructureRedirect`, handling `Focus`, `KeyPress`, and `ConfigureRequest`.
- **HeadlessBackend** (testing / CI): virtual display outputs and event injection for automated testing without physical display hardware.
- **Xwayland Subsystem**: manages an embedded Xwayland server instance to bridge legacy X11 applications into the Wayland session.

---

## 5. wm-render (Pluggable GPU Render Pipeline)

Abstracts GPU graphics pipelines, hiding driver specifics. Renders scene graphs into frame buffers for each display output.

```rust
pub trait RenderDevice {
    fn create_texture(&mut self, width: u32, height: u32, format: PixelFormat, data: Option<&[u8]>) -> Result<TextureHandle, RenderError>;
    fn import_dma_buffer(&mut self, buffer: &GraphicBuffer) -> Result<TextureHandle, RenderError>;
    fn destroy_texture(&mut self, texture: TextureHandle) -> Result<(), RenderError>;
    fn begin_frame(&mut self, output: &OutputInfo) -> Result<(), RenderError>;
    fn render_quad(&mut self, bounds: Rect<f32>, texture: &TextureHandle, transform: &glam::Mat4, pipeline: Option<&PipelineHandle>);
    fn render_color_rect(&mut self, bounds: Rect<f32>, color: Color, transform: &glam::Mat4);
    fn end_frame(&mut self) -> Result<(), RenderError>;
}
```

- **GlRenderDevice**: `EGLDisplay` and `EGLContext` initialized on GBM / DRM devices. Client DMA-BUF buffers are imported via `EGL_EXT_image_dma_buf_import` into GL texture targets (`glEGLImageTargetTexture2DOES`).
- **VkRenderDevice** (planned / extension): clean drop-in implementation of `RenderDevice` using Vulkan (via `ash`), importing memory via `VK_EXT_external_memory_dma_buf` and rendering through SPIR-V pipelines.
- **Shaders**: standard GLSL (`.vert` and `.frag`), compiled dynamically. File changes trigger hot-recompilation and live shader swap without dropped frames.
- **Direct Scanout**: in fullscreen unshaded windows (TILING/STACKING), the client buffer passes directly to the DRM plane via atomic KMS commits, bypassing the compositing pass to eliminate input latency.

---

## 6. wm-scene (2D Spatial Canvas & Scene Graph)

Maintains the hierarchical scene structure and computes spatial coordinate transformations.

- **Scene Graph Tree**: `RootNode → WorkspaceNode → ContainerNode → WindowNode`.
- **2D Camera**: `Camera2D { position: Point<f64>, zoom: f32, rotation: f32 }`. Window coordinates are stored in the unbounded world coordinate system. The camera generates orthographic view-projection matrices for `wm-render`.
- **Damage Tracking**: dirty rectangle accumulation (`DamageTracker`) — coalescing bounding boxes to redraw only modified screen areas.

---

## 7. wm-layout (Lua Profile Engine)

Computes window coordinates from the active window list, passing target geometries to `wm-scene`. Each profile is an external Lua file sandboxed via `mlua`.

```rust
pub trait LayoutEngine {
    fn name(&self) -> &str;
    fn profile(&self) -> LayoutProfile; // Spatial | Scrolling | Tiling | Stacking
    fn calculate(&mut self, windows: &[WindowState], bounds: Rect<f64>, camera: &Camera2D) -> HashMap<WindowId, Rect<f64>>;
    fn on_move(&mut self, win: WindowId, world_pos: Point<f64>) -> Option<Point<f64>>;
    fn on_spawn(&mut self, win: WindowId, pending_rect: Option<Rect<f64>>) -> Rect<f64>;
    fn handle_event(&mut self, event: LayoutEvent);
}
```

`LuaLayout` implements `LayoutEngine`, evaluating sandboxed Lua chunks. If a script fails or is missing, it falls back to native Rust implementations in `wm-layout/src/builtin/`.

**Lua Script Contract:**

```lua
-- ~/.config/horyzond/layouts/spatial.lua
layout = {
    name = "spatial",
    profile = "SPATIAL",
}

-- windows: [{id, world_x, world_y, w, h, focused}]
-- bounds:  {x, y, w, h}  -- viewport screen space
-- camera:  {x, y, zoom}
function calculate(windows, bounds, camera)
    local result = {}
    local half_w = bounds.w / 2.0
    local half_h = bounds.h / 2.0

    for _, win in ipairs(windows) do
        result[win.id] = {
            x = bounds.x + half_w + (win.world_x - camera.x) * camera.zoom,
            y = bounds.y + half_h + (win.world_y - camera.y) * camera.zoom,
            width  = win.w * camera.zoom,
            height = win.h * camera.zoom,
            z_index = win.focused and 100 or 0,
        }
    end
    return result
end

function on_move(win_id, new_world_x, new_world_y)
    return { world_x = new_world_x, world_y = new_world_y }
end

function on_spawn(win_id, pending_rect)
    if pending_rect then
        return pending_rect
    end
    return { x = -320, y = -240, width = 640, height = 480 }
end
```

---

## 8. wm-ipc (External Management Interface)

Interfaces with external clients, panels, status bars, and CLI tools:

- **Wayland Protocols**: `xdg-shell`, `wlr-layer-shell` (panels, wallpapers, docks), `wlr-foreign-toplevel-management`.
- **IPC Socket**: Unix Domain Socket (JSON protocol). Commands:
  - `layout set <profile>`
  - `camera zoom <factor>`
  - `camera pan <dx> <dy>`
  - `window focus <direction>`
  - `spawn-pending <app_id> <exec_cmd>`
  - `status`

---

## 9. Configuration (`~/.config/horyzond/`)

Modular user configuration with runtime hot-reloading:

```
~/.config/horyzond/
├── config.lua                  # Global settings
├── modes.lua                   # Modal keybindings and state definitions
├── layouts/
│   ├── spatial.lua              # SPATIAL layout
│   ├── scrolling.lua            # SCROLLING layout
│   ├── tiling.lua                # TILING (BSP) layout
│   ├── stacking.lua              # STACKING layout
│   └── custom.lua                # User-defined custom layouts
├── shaders/
│   ├── window.vert
│   ├── window.frag
│   └── rounded_border.frag
├── rules.d/
│   ├── media.lua
│   └── dev.lua
└── hooks/
    ├── on_window_open.lua
    └── on_workspace_change.lua
```

**`modes.lua`** — modal keybindings:

```lua
return {
  normal = {
    binds = {
      { key = "Super+Enter",  action = "spawn", arg = "ghostty" },
      { key = "Super+h",     action = "focus_left" },
      { key = "Super+Space", action = "switch_mode", arg = "layout_select" },
      { key = "Super+r",     action = "switch_mode", arg = "resize" },
      { key = "Super+z",     action = "camera_zoom_toggle" },
    },
  },
  resize = {
    status_indicator = "RESIZE MODE",
    binds = {
      { key = "h", action = "window_resize", arg = { left = -20 } },
      { key = "l", action = "window_resize", arg = { right = 20 } },
      { key = "Escape", action = "switch_mode", arg = "normal" },
    },
  },
  layout_select = {
    binds = {
      { key = "1", action = "set_layout", arg = "spatial" },
      { key = "2", action = "set_layout", arg = "scrolling" },
      { key = "3", action = "set_layout", arg = "tiling" },
      { key = "4", action = "set_layout", arg = "stacking" },
      { key = "Escape", action = "switch_mode", arg = "normal" },
    },
  },
}
```

---

## 10. Cargo Workspace Structure

```
horyzond/
├── Cargo.toml                        # Workspace manifest and dependencies
├── config/                           # Default configuration templates and shaders
│   ├── config.lua
│   ├── modes.lua
│   ├── layouts/
│   └── shaders/
├── crates/
│   ├── wm-core/                      # State machine, workspace manager, rules engine
│   ├── wm-backend/                   # DisplayBackend trait, Wayland DRM/libinput, X11, Headless
│   ├── wm-render/                    # RenderDevice trait, OpenGL (EGL/GLES), Vulkan
│   ├── wm-scene/                     # 2D Camera, SceneGraph, Damage Tracking
│   ├── wm-layout/                    # LayoutEngine trait, Lua sandbox runtime, built-in fallbacks
│   ├── wm-ipc/                       # Unix Domain Socket IPC and protocol definitions
│   └── wm-cli/                       # horyctl CLI client
└── src/
    └── main.rs                       # Daemon entrypoint, wiring, and coordinator event loop
```

---

## 11. Phased Implementation Roadmap

- **Phase 0 — Skeleton & Neutral Traits**: Workspace, `DisplayBackend` and `RenderDevice` traits, headless harnesses, 2D geometry math.
- **Phase 1 — 2D Spatial Camera & Scene Graph**: Camera forward/inverse projections ($Camera^{-1}$), scene graph tree, damage coalescing.
- **Phase 2 — Layout Engine & Lua Sandbox**: 4 canonical layout profiles (SPATIAL, SCROLLING, TILING, STACKING) in Lua and native Rust, instruction limits.
- **Phase 3 — Modal Input, Rules & Hot-Reload**: Modal state machine (`Normal`, `Insert`, `Resize`, `LayoutSelect`, `SpatialSpawnPending`), window rules engine, configuration file watcher.
- **Phase 4 — IPC Subsystem & CLI**: Unix Domain Socket server, JSON request/response handling, `horyctl` CLI.
- **Phase 5 — Wayland Display Backend**: Wayland server globals, DRM/KMS mode-setting, libinput device management.
- **Phase 6 — OpenGL Renderer & Direct Scanout**: EGL context on GBM device, DMA-BUF import, GLSL shader hot-reloading, direct scanout fast-path.
- **Phase 7 — Surface Protocols & Spawn Workflow**: `xdg-shell`, `wlr-layer-shell`, interactive spawn-by-rectangle right-click drag flow.
- **Phase 8 — Advanced Extensions**: Native X11 backend (`x11rb`/`xcb`), embedded Xwayland server, and native Vulkan renderer (`ash`).

---

## 12. Architectural Points of Extension

- **Multi-Backend Architecture (`wm-backend`)**:
  - `DisplayBackend` completely separates window management from protocol specifics. Adding a native X11 backend or headless virtual backend requires implementing only this trait.
- **Multi-Renderer Architecture (`wm-render`)**:
  - `RenderDevice` completely separates rendering commands from graphics APIs. Adding a Vulkan renderer (`VkRenderDevice`) requires implementing this trait without altering `wm-scene` or `wm-core`.
- **Sandboxed Lua Runtime (`wm-layout`)**:
  - Safe sandboxed runtime with instruction counter hooks prevents untrusted or infinite-looping user scripts from freezing the compositor.
- **Interactive Spawn Workflow**:
  - Spatial spawn-by-rectangle combines modal state management, $Camera^{-1}$ raycasting, and immediate client surface bounds binding.

---

## 13. System Considerations & Risks

- **HiDPI & Fractional Scaling**: combining 2D camera zoom with fractional scaling requires sending clean buffer scales to avoid font rasterization blurriness.
- **Lua Execution Safety**: strict instruction limits and restricted standard libraries (`_G` sanitized of OS and IO access) protect against hangs and exploits.
- **Spatial Coordinates Persistence**: world coordinates in SPATIAL profile are preserved per workspace to maintain continuous spatial arrangements across restarts.
- **Hand-Rolled Protocol Coverage**: comprehensive seat capabilities, clipboards, and data devices are modularly structured to avoid incomplete Wayland feature sets.
