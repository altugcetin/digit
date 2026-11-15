-- A stand-in for Hyprland's `hl` table, to run generated Lua outside Hyprland.
--   luac -p FILE.lua && lua hl-mock.lua FILE.lua ["SUPER + PAGE_DOWN" ...]
-- Loads FILE, then fires the named binds (or lists them) and prints every
-- dispatch they make. Edit STATE below to model the case you're checking:
-- an empty workspace at the edge, a shown special workspace, no active window.

local STATE = {
  workspaces = {
    { id = 1, name = "1", monitor = "DP-2", windows = 2 },
    { id = 2, name = "2", monitor = "DP-2", windows = 0 },
    { id = nil, name = "special:scratch", monitor = "DP-2", windows = 1, special = true },
  },
  active_workspace = 1,
  special = nil, -- e.g. { name = "special:scratch" } while one is shown
  window = { address = "0x1", at = { x = 10, y = 10 }, workspace = { id = 1 } },
  config = { ["binds.scroll_event_delay"] = 150 },
}

local binds, log = {}, {}

-- Any unknown path, such as hl.dsp.window.move{...}, records its name and arguments.
local function recorder(name)
  return setmetatable({}, {
    __index = function(_, k) return recorder(name .. "." .. k) end,
    __call = function(_, ...)
      local args = {}
      for i, a in ipairs({ ... }) do
        if type(a) == "table" then
          local parts = {}
          for k, v in pairs(a) do parts[#parts + 1] = tostring(k) .. "=" .. tostring(v) end
          table.sort(parts)
          args[i] = "{" .. table.concat(parts, ",") .. "}"
        else
          args[i] = tostring(a)
        end
      end
      return { action = name .. "(" .. table.concat(args, ", ") .. ")" }
    end,
  })
end

local function workspace(id)
  for _, w in ipairs(STATE.workspaces) do if w.id == id then return w end end
end

hl = setmetatable({
  bind = function(keys, action) binds[keys] = action end,
  unbind = function() end,
  dispatch = function(d) log[#log + 1] = type(d) == "table" and d.action or tostring(d) end,
  get_workspaces = function() return STATE.workspaces end,
  get_active_workspace = function() return workspace(STATE.active_workspace) end,
  get_active_special_workspace = function() return STATE.special end,
  get_active_window = function() return STATE.window end,
  get_active_monitor = function() return { name = "DP-2" } end,
  get_config = function(k) return STATE.config[k] end,
}, { __index = function(_, k) return recorder("hl." .. k) end })

local file = assert(arg[1], "usage: lua hl-mock.lua FILE.lua [KEYS...]")
dofile(file)

if #arg < 2 then
  local keys = {}
  for k in pairs(binds) do keys[#keys + 1] = k end
  table.sort(keys)
  for _, k in ipairs(keys) do print(k) end
  return
end
for i = 2, #arg do
  local action = binds[arg[i]]
  log = {}
  if type(action) == "function" then
    local ok, err = pcall(action)
    if not ok then print(arg[i] .. "  ERROR " .. tostring(err)) end
  elseif action then
    hl.dispatch(action)
  else
    print(arg[i] .. "  (not bound)")
  end
  print(arg[i] .. "  ->  " .. (#log > 0 and table.concat(log, " ; ") or "nothing"))
end
