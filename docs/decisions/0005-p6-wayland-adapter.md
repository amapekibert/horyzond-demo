# 0005: Use Smithay behind P6 adapter boundaries

## Status

Accepted for P6.

## Context

P6 needs a nested Wayland compositor host, protocol lifecycle support, and an
OpenGL ES renderer without allowing Wayland, EGL, or GL resource types into
the neutral core, scene, backend, or renderer contracts. The headless build
must continue to compile and test without graphics system libraries.

## Decision

Introduce Smithay 0.7 behind new concrete Wayland and GL adapter crates. Keep
Smithay optional from the composition root and enable it only for the nested
Wayland runtime path. Select only the frontend, nested-host, EGL, and GLES
features needed by the first visible slice; defer DRM, GBM, libinput, and seat
features to P7.

Native display handles, EGL contexts, GL textures, buffers, fences, and
presentation objects remain adapter-private. The adapters communicate with the
existing neutral contracts through normalized lifecycle events, immutable scene
snapshots, frame results, and explicit completion/release notifications.

## Consequences

The initial nested path gains maintained protocol and GLES building blocks
without making headless CI depend on a display server or graphics libraries.
P6 must add tests for configure/acknowledge/commit and buffer-release ordering
before declaring the visible slice complete. Smithay 0.7 is MIT-licensed and
requires Rust 1.87, which is compatible with this workspace's Rust 1.88 MSRV.
