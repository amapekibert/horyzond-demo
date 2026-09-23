return {
  normal = { binds = {} },
  insert = { binds = { { key = "Escape", action = "switch_mode", mode = "normal" } } },
  passthrough = { binds = { { key = "Escape", action = "switch_mode", mode = "normal" } } },
  move = {
    binds = {
      { key = "Escape", action = "switch_mode", mode = "normal" },
      { key = "Enter", action = "move_commit", mode = "normal" },
    },
  },
  resize = {
    binds = {
      { key = "Escape", action = "switch_mode", mode = "normal" },
      { key = "Enter", action = "resize_commit", mode = "normal" },
    },
  },
  layout_select = {
    binds = {
      { key = "Escape", action = "switch_mode", mode = "normal" },
      { key = "1", action = "select_layout", command = { "spatial" }, mode = "normal" },
      { key = "2", action = "select_layout", command = { "scrolling" }, mode = "normal" },
      { key = "3", action = "select_layout", command = { "tiling" }, mode = "normal" },
      { key = "4", action = "select_layout", command = { "stacking" }, mode = "normal" },
    },
  },
  pending_spawn = {
    binds = {
      { key = "Escape", action = "cancel_pending_spawn", mode = "normal" },
    },
  },
}
