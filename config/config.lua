-- Import other files.
source("keybinds.lua")
source("rules.lua")
source("theme.lua")

-- Default settings. These values are loaded in P2.
settings = {
  default_layout = "spatial",
  reload = { enabled = true, debounce_ms = 150 },
  logging = { level = "trace", detail = "full" },
}

modes = source("modes.lua")
layouts = {
  spatial = "layouts/spatial.lua",
  scrolling = "layouts/scrolling.lua",
  tiling = "layouts/tiling.lua",
  stacking = "layouts/stacking.lua",
}
hooks = {
  on_startup = "hooks/on_startup.lua",
  on_reload = "hooks/on_reload.lua",
  on_window_open = "hooks/on_window_open.lua",
  on_workspace_change = "hooks/on_workspace_change.lua",
}
