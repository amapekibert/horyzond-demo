# 0001: Keep a single-process modular architecture

## Status

Accepted for P0.

## Decision

Horyzond runs in one process with one primary event-loop thread. Each Cargo crate has a focused responsibility, private mutable state, and explicit typed contracts. The root binary is the composition root.

The project does not use a global event bus, dependency-injection container, ECS, dynamic native plugin ABI, or one process per module. Bounded workers may be introduced only for operations that would block the event loop.

## Consequences

The code remains easy to trace and test while backend, renderer, layout, and configuration implementations can evolve independently. Cross-module ordering belongs to `wm-core` when it is introduced.
