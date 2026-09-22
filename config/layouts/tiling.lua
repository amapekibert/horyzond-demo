layout = { name = "tiling", profile = "tiling", api_version = 1 }

-- Master-stack tiling in canonical world coordinates.
function calculate(windows, bounds, _camera)
  local result = {}
  local count = #windows
  if count == 0 then return result end
  if count == 1 then
    result[windows[1].id] = { x = bounds.x, y = bounds.y, width = bounds.width, height = bounds.height }
    return result
  end
  result[windows[1].id] = { x = bounds.x, y = bounds.y, width = bounds.width / 2, height = bounds.height }
  local stack_height = bounds.height / (count - 1)
  for index = 2, count do
    result[windows[index].id] = { x = bounds.x + bounds.width / 2, y = bounds.y + (index - 2) * stack_height, width = bounds.width / 2, height = stack_height }
  end
  return result
end
