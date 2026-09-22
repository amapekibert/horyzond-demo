layout = { name = "tiling", profile = "tiling", api_version = 1 }

-- Master-stack tiling in canonical world coordinates.
local master_ratio = 0.5
local gap = 0

function calculate(windows, bounds, _camera)
  local result = {}
  local count = #windows
  if count == 0 then return result end
  if count == 1 then
    result[windows[1].id] = { x = bounds.x, y = bounds.y, width = bounds.width, height = bounds.height }
    return result
  end
  local horizontal_gap = math.min(gap, bounds.width / 3)
  local available_width = bounds.width - horizontal_gap
  local master_width = available_width * master_ratio
  local stack_width = available_width - master_width
  local stack_count = count - 1
  local vertical_gap = stack_count <= 1 and 0 or math.min(gap, bounds.height / (2 * stack_count))
  local stack_height = (bounds.height - vertical_gap * (stack_count - 1)) / stack_count
  result[windows[1].id] = { x = bounds.x, y = bounds.y, width = master_width, height = bounds.height }
  for index = 2, count do
    result[windows[index].id] = { x = bounds.x + master_width + horizontal_gap, y = bounds.y + (index - 2) * (stack_height + vertical_gap), width = stack_width, height = stack_height }
  end
  return result
end
