layout = { name = "stacking", profile = "stacking", api_version = 1 }

-- Preserve user geometry; cascade only windows that have no previous rectangle.
function calculate(windows, bounds, _camera)
  local result = {}
  for index, window in ipairs(windows) do
    result[window.id] = {
      x = window.x or (bounds.x + 40 + (index - 1) * 24),
      y = window.y or (bounds.y + 40 + (index - 1) * 24),
      width = window.width or (bounds.width * 0.7),
      height = window.height or (bounds.height * 0.7),
    }
  end
  return result
end
