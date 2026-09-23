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

```lua
layout = {
  id = "my_layout",
  api_version = 1,
}

function calculate(windows, bounds, camera)
  local result = {}
  for index, window in ipairs(windows) do
    result[window.id] = {
      x = bounds.x + (index - 1) * 32,
      y = bounds.y + (index - 1) * 32,
      width = bounds.width * 0.7,
      height = bounds.height * 0.7,
    }
  end
  return result
end
```

`calculate` receives three tables:

- `windows` is an array in stable coordinator order. Every item has `id`. If a rectangle was previously saved for that layout and window, it also has `x`, `y`, `width`, and `height`.
- `bounds` has finite `x`, `y`, `width`, and `height` values for the current work area.
- `camera` is reserved for future provider camera intent. API version 1 supplies an empty table.

The return value must contain one rectangle for every supplied window, keyed by its numeric `window.id`. A rectangle requires finite `x`, `y`, `width`, and `height`; widths and heights must be positive. Extra entries are ignored. Missing or invalid entries reject that calculation and use generic recovery placement for that request.

Provider source is evaluated in a restricted Lua environment with table, string, math, and UTF-8 libraries. Memory and instruction limits apply to loading and every calculation. Operating-system access, file access, module loading, process spawning, and compositor globals are unavailable.

Configuration reload validates every provider declaration before it becomes active. A failed edit retains that provider's most recent valid implementation when available. Removing an ID from `layouts` removes it intentionally. Providers must not rely on Rust code recognizing their names.
