# 0004: Treat user configuration as transactional external policy

## Status

Accepted for P0; implemented in P1 and P2.

## Decision

Shipped defaults are readable files under `config/` and are packaged for non-destructive first-run installation. `~/.config/horyzond/` is the default configuration location. The future `source()` function resolves relative imports against the importing file, detects cycles, and rejects root escapes.

Reload will evaluate and validate a fresh candidate before committing one generation at a safe event-loop boundary. An invalid candidate preserves the last valid configuration and active clients.

## Consequences

Rust owns validation and safety limits, while settings, bindings, rules, profiles, hooks, and themes remain user-editable policy. P1 owns installation/log bootstrap; P2 owns Lua evaluation and file watching.
