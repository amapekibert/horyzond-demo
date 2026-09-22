# 0002: Apply camera projection once

## Status

Accepted for P0.

## Decision

Layout providers return validated world-space rectangles. `wm-scene` will transform them once into output-local logical rectangles; the output transform and scale then produce physical pixels. Input follows the inverse path.

World positions use finite `f64` values. The scene implementation will subtract the camera origin before GPU conversion. Camera rotation is deferred until it has a real product requirement.

## Consequences

SPATIAL data remains persistent while moving the camera. Layout scripts do not need renderer knowledge, and profiles cannot accidentally apply a second camera transform.
