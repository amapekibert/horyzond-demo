# Lua layout provider API

Layouts are independent Lua providers. Horyzond does not reserve or interpret layout names. Add a provider by naming it in `layouts` within `~/.config/horyzond/config.lua`:

```lua
settings = {
  default_layout = "my_layout",
}

layouts = {
  my_layout = "layouts/my_layout.lua",
}
```

The table key is the provider's opaque ID. It must match the `layout.id` value in the Lua file.

The configured `default_layout` selects the initial provider. Future mode and IPC code can select any configured opaque ID through the neutral runtime; the WM does not attach behavior to an ID or its filename.

```lua
layout = {
  id = "my_layout",
  api_version = 1,
}

function calculate(windows, bounds, camera, state)
  local result = {}
  for index, window in ipairs(windows) do
    result[window.id] = {
      x = bounds.x + (index - 1) * 32,
      y = bounds.y + (index - 1) * 32,
      width = bounds.width * 0.7,
      height = bounds.height * 0.7,
    }
  end
  result.state = state
  return result
end
```

`calculate` receives four values:

- `windows` is an array in stable coordinator order. Every item has `id`. If a rectangle was previously saved for that layout and window, it also has `x`, `y`, `width`, and `height`.
- `bounds` has finite `x`, `y`, `width`, and `height` values for the current work area.
- `camera` is reserved for future provider camera intent. API version 1 supplies an empty table.
- `state` is the provider's prior JSON-compatible data. It is `null` when the provider has not returned state before.

The return value must contain one rectangle for every supplied window, keyed by its numeric `window.id`. A rectangle requires finite `x`, `y`, `width`, and `height`; widths and heights must be positive. Extra entries are ignored. Missing or invalid entries reject that calculation and use generic recovery placement for that request.

The result can also set `order` to an array containing every supplied window ID exactly once, from back to front. When `order` is absent, the stable `windows` input order is retained. An invalid order rejects the whole calculation and uses generic recovery placement.

The result can set `state` to any JSON-compatible value. It is stored under the provider's opaque ID in workspace state and is supplied to the next calculation for that ID. Omitting `state` retains the prior value. Functions, userdata, recursive tables, and values that cannot be converted to JSON reject the calculation and use generic recovery placement.

## Optional interactions

Providers can define an `interact(windows, bounds, camera, state, event)` function. It returns the same result table as `calculate`. The runtime calls it when a future mode or IPC command routes an opaque action to the active provider.

`event` is JSON-compatible and has two fields: `action`, a non-empty provider-defined string, and `payload`, a provider-defined JSON value. Horyzond does not reserve action names or inspect payload fields. If `interact` is absent, the runtime falls back to `calculate`. An invalid interaction result uses generic recovery placement for that request.

Provider source is evaluated in a restricted Lua environment with table, string, math, and UTF-8 libraries. Memory and instruction limits apply to loading and every calculation. Operating-system access, file access, module loading, process spawning, and compositor globals are unavailable.

Configuration reload validates every provider declaration before it becomes active. A failed edit retains that provider's most recent valid implementation when available. Removing an ID from `layouts` removes it intentionally. Providers must not rely on Rust code recognizing their names.
