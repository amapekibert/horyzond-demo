# 0003: Separate window system, renderer, and presenter

## Status

Accepted for P0.

## Decision

Window-system adapters own native protocol state and normalize lifecycle/input/output events. Renderers consume immutable scene snapshots. Presenters will acquire targets and submit completed frames. Capability descriptors make optional behavior explicit.

Native resource handles, synchronization primitives, and protocol serials remain in their owning adapters. Xwayland, a native X11 manager, and an X11 compositor are separate future deliverables.

## Consequences

The first headless adapter and recording renderer can verify the boundary without Wayland, OpenGL, Vulkan, or X11 libraries. A future native X11 manager can report missing compositing or camera-scaling support honestly.
