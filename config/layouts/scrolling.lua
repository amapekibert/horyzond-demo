layout = { id = "scrolling", api_version = 1 }

-- A horizontal world-space ribbon. Camera movement reveals distant columns.
function calculate(windows, bounds, _camera)
  local result = {}
  for index, window in ipairs(windows) do
    result[window.id] = { x = bounds.x + (index - 1) * bounds.width, y = bounds.y, width = bounds.width, height = bounds.height }
  end
  return result
end
