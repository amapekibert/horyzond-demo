# Horyzond

Horyzond is a Rust window manager and compositor under development. It will provide four independent workspace profiles: SPATIAL, SCROLLING, TILING, and STACKING. Wayland with OpenGL is the first graphical target; native X11 and Vulkan are planned extensions behind stable neutral contracts.

The project is licensed under [0BSD](LICENSE). The repositories in `examples/` are design references only. They keep their own licenses and are not compiled or distributed as Horyzond code.

## Current status

P0 is implemented: the repository is a Cargo workspace with platform-neutral types, separate window-system and renderer contracts, a deterministic headless adapter, and a recording renderer. This is a foundation, not yet a usable compositor.

## Development roadmap

| Phase | Scope | Status |
| --- | --- | --- |
| P0 | Workspace, neutral contracts, headless verification | Complete |
| P1 | First-run configuration bootstrap, session and crash diagnostics | Next |
| P2 | Lua configuration, `source()`, validation, live reload | Planned |
| P3 | Headless core, workspaces, camera, scene | Planned |
| P4 | SPATIAL, SCROLLING, TILING, STACKING layout providers | Planned |
| P5 | Modes, rules, hooks, IPC, `wmctl` | Planned |
| P6 | Nested Wayland session and OpenGL renderer | Planned |
| P7 | DRM/KMS, libinput, standalone Wayland | Planned |
| P8 | Desktop protocols and spatial spawn workflow | Planned |
| P9 | Shader reload, damage, performance, direct scanout | Planned |
| P10 | Persistence, hardening, release preparation | Planned |
| P11 | Xwayland and native X11 adapter | Planned |
| P12 | Vulkan renderer | Planned |

The detailed implementation plan is in [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md). Architectural decisions are recorded in [docs/decisions](docs/decisions).

## Repository layout

```text
crates/wm-types/             Platform-neutral IDs, geometry, capabilities
crates/wm-backend/           Window-system adapter contract
crates/wm-backend-headless/  Deterministic adapter for tests
crates/wm-render/            Renderer contract and recording renderer
docs/decisions/              Short architecture decisions
examples/                    Upstream design references, excluded from the build
```

## Build and test

Horyzond currently requires Rust 1.85 or later.

```sh
cargo fmt --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run
```

The current `cargo run` command only demonstrates P0 composition with a virtual output. It does not create `~/.config/horyzond/`; that begins in P1.

## Development TODO

- [x] Create platform-neutral module boundaries.
- [x] Add a graphics-free headless verification path.
- [x] Document capability and coordinate contracts.
- [ ] Install default configuration without replacing user files.
- [ ] Create `latest.log`, archived session logs, and crash reports.
- [ ] Implement sandboxed Lua configuration with imports and transactional reload.
- [ ] Build the headless core and all four layouts.
- [ ] Add Wayland, OpenGL, X11, and Vulkan adapters in roadmap order.

## Contributing workflow

Keep new user-facing text, source comments, configuration, and documentation in English. Use Conventional Commits for local commits, for example `feat(types): add neutral geometry`. Run formatting, tests, and Clippy before completing a phase. Do not push from this workspace unless explicitly requested.

See [resume.md](resume.md) for the exact continuation point and outstanding work per phase.
