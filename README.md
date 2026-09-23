# Horyzond

Horyzond is a Rust window manager and compositor under development. Layout behavior is supplied by independent Lua providers selected from configuration; users can add their own without changing WM code. Wayland with OpenGL is the first graphical target; native X11 and Vulkan are planned extensions behind stable neutral contracts.

The project is licensed under [WTFPL](LICENSE). The repositories in `examples/` are design references only. They keep their own licenses and are not compiled or distributed as Horyzond code.

## Current status

P0 through P4 are implemented. The repository has a modular headless foundation, bounded Lua configuration reload, deterministic workspace lifecycle state, camera projection, scene snapshots, picking, damage tracking, and independent restricted Lua layout providers. This is still not yet a usable compositor.

## Development roadmap

| Phase | Scope | Status |
| --- | --- | --- |
| P0 | Workspace, neutral contracts, headless verification | Complete |
| P1 | First-run configuration bootstrap, session and crash diagnostics | Complete |
| P2 | Lua configuration, `source()`, validation, live reload | Complete |
| P3 | Headless core, workspaces, camera, scene | Complete |
| P4 | Provider-defined layout runtime and recovery | Complete |
| P5 | Modes, rules, hooks, IPC, `horyctl` | Planned |
| P6 | Nested Wayland session and OpenGL renderer | Planned |
| P7 | DRM/KMS, libinput, standalone Wayland | Planned |
| P8 | Desktop protocols and spatial spawn workflow | Planned |
| P9 | Shader reload, damage, performance, direct scanout | Planned |
| P10 | Persistence, hardening, release preparation | Planned |
| P11 | Xwayland and native X11 adapter | Planned |
| P12 | Vulkan renderer | Planned |

The detailed implementation plan is in [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md). Architectural decisions are recorded in [docs/decisions](docs/decisions).

The custom provider contract is documented in [docs/layout-provider-api.md](docs/layout-provider-api.md).

## Repository layout

```text
crates/wm-types/             Platform-neutral IDs, geometry, capabilities
crates/wm-backend/           Window-system adapter contract
crates/wm-backend-headless/  Deterministic adapter for tests
crates/wm-render/            Renderer contract and recording renderer
crates/wm-runtime/           Safe-boundary configuration and layout coordinator
docs/decisions/              Short architecture decisions
examples/                    Upstream design references, excluded from the build
```

## Build and test

Horyzond currently requires Rust 1.88 or later.

```sh
cargo fmt --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -- --once
```

The current `cargo run` command bootstraps `~/.config/horyzond/` (unless `--config-dir` is supplied), starts session diagnostics, and runs a headless configuration polling loop. Use `--once` for bootstrap-only verification. It does not yet create a visible compositor session.

## Development TODO

- [x] Create platform-neutral module boundaries.
- [x] Add a graphics-free headless verification path.
- [x] Document capability and coordinate contracts.
- [x] Install default configuration without replacing user files.
- [x] Create `latest.log`, archived session logs, and crash reports.
- [x] Implement sandboxed Lua configuration with imports and transactional reload.
- [x] Build the deterministic headless core, scene, camera, and damage foundations.
- [x] Define a neutral opaque layout-provider contract.
- [x] Execute bounded, versioned Lua layout providers and validate configured provider paths.
- [x] Select configured providers through the neutral core with generic recovery placement.
- [x] Reject invalid provider declarations before they enter the active layout registry.
- [x] Retain an individual provider's last valid implementation when its reload fails.
- [x] Synchronize accepted configuration generations to providers without coupling core to configuration.
- [x] Poll configuration and reapply accepted provider generations in the headless runtime.
- [x] Serialize versioned, data-only provider state and migrate compatible geometry through provider reload.
- [x] Let a provider return validated world geometry and paint order.
- [x] Allow callers to select any configured opaque provider through the neutral runtime.
- [x] Preserve JSON-compatible provider state without storing Lua closures.
- [x] Route opaque provider-defined interactions without hardcoded layout actions.
- [x] Preserve provider histories and opaque layout selections per workspace.
- [ ] Build P5 modal input, rules, hooks, and `horyctl` IPC.
- [ ] Add Wayland, OpenGL, X11, and Vulkan adapters in roadmap order.

## Contributing workflow

Keep new user-facing text, source comments, configuration, and documentation in English. Use Conventional Commits for local commits, for example `feat(types): add neutral geometry`. Run formatting, tests, and Clippy before completing a phase. Do not push from this workspace unless explicitly requested.

See [resume.md](resume.md) for the exact continuation point and outstanding work per phase.
