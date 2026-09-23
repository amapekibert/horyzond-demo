-- Key bindings are evaluated by the modal input system in P5.
keybinds = {
  { key = "Super+Enter", action = "spawn", command = { "ghostty" }, mode = "pending_spawn" },
  { key = "Super+I", action = "switch_mode", mode = "insert" },
  { key = "Super+P", action = "switch_mode", mode = "passthrough" },
  { key = "Super+M", action = "move_begin", mode = "move" },
  { key = "Super+R", action = "resize_begin", mode = "resize" },
  { key = "Super+Space", action = "switch_mode", mode = "layout_select" },
}
