layout = { name = "spatial", profile = "spatial", api_version = 1 }

-- Keep persistent world-space geometry. The scene camera performs projection.
function calculate(windows, _bounds, _camera)
  local result = {}
  for index, window in ipairs(windows) do
    result[window.id] = {
      x = window.x or (-320 + (index - 1) * 32),
      y = window.y or (-240 + (index - 1) * 32),
      width = window.width or 640,
      height = window.height or 480,
    }
  end
  return result
end
