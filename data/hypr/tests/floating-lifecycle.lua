local path = assert(arg[1])
local function eq(a, b) assert(a == b, tostring(a) .. ' ~= ' .. tostring(b)) end
local callbacks, windows, commands, timers = {}, {}, {}, {}
local cursor, active, drag_target
local m = { name = 'DP-1', width = 1920, height = 1080, scale = 1, x = 0, y = 0, transform = 0,
    reserved = { top = 40, bottom = 80, left = 0, right = 0 }, enabled = true }
local ws = { id = 1, monitor = m }
local function window(address, floating, workspace)
    return { address = address, mapped = true, floating = floating, fullscreen = 0, tags = {}, workspace = workspace or ws,
        monitor = m, at = { x = 100, y = 150 }, size = { x = 800, y = 600 } }
end
local function event(name, ...) for _, cb in ipairs(callbacks[name] or {}) do cb(...) end end
local function factory(name) return function(args) return { name = name, args = args } end end
local moves, resizes = 0, 0
hl = {
    dsp = { window = { tag = factory('tag'), float = factory('float'), move = factory('move'), resize = factory('resize'), drag = factory('drag') } },
    on = function(name, callback) callbacks[name] = callbacks[name] or {}; table.insert(callbacks[name], callback) end,
    get_config = function(key)
        if key == 'general.border_size' then return 2 end
        if key == 'general.gaps_out' then return { top = 8, right = 8, bottom = 8, left = 8 } end
        return { top = 4, right = 4, bottom = 4, left = 4 }
    end,
    get_windows = function() return windows end,
    get_monitors = function() return { m } end,
    get_active_window = function() return active end,
    get_cursor_pos = function() return cursor end,
    exec_cmd = function(cmd) commands[#commands + 1] = cmd end,
    window_rule = function(rule) eq(rule.enabled, false) end,
    workspace_rule = function(rule) assert(rule.layout ~= 'floating') end,
    timer = function(callback, opts)
        eq(opts.timeout, 33); eq(opts.type, 'repeat')
        local timer = { callback = callback, enabled = true }
        function timer:set_enabled(on) self.enabled = on end
        timers[#timers + 1] = timer
        return timer
    end,
    dispatch = function(action)
        local args, name = action.args, action.name
        local w = args and args.window
        if name == 'tag' then
            local sign, tag = args.tag:sub(1, 1), args.tag:sub(2)
            for i = #w.tags, 1, -1 do if w.tags[i] == tag then table.remove(w.tags, i) end end
            if sign == '+' then w.tags[#w.tags + 1] = tag end
        elseif name == 'float' then w.floating = args.action == 'on'
        elseif name == 'resize' then resizes = resizes + 1; w.size = { x = args.x, y = args.y }
        elseif name == 'move' then moves = moves + 1; w.at = { x = args.x, y = args.y }
        elseif name == 'drag' then if drag_target then active = drag_target end end
        return { success = true }
    end,
}
local M = dofile(path)
local tiled, own, fs, pinned, named, special = window('tiled', false), window('own', true), window('fs', false), window('pin', true), window('named', false, {}), window('special', false, { id = 2, special = true })
fs.fullscreen, pinned.pinned = 2, true
windows = { tiled, own, fs, pinned, named, special }
event('config.reloaded') -- generated default in this fixture is floating
assert(tiled.floating); assert(own.floating); assert(pinned.floating)
assert(not fs.floating); assert(not named.floating); assert(not special.floating)
eq(#tiled.tags, 1); eq(#own.tags, 0); eq(#timers, 0)
-- A new client is arranged synchronously in the map callback without another timer.
local fresh = window('fresh', false)
windows[#windows + 1] = fresh
event('window.open', fresh)
assert(fresh.floating); assert(fresh.at.x ~= tiled.at.x or fresh.at.y ~= tiled.at.y)
eq(#timers, 0)
-- Auto-grouping inherits mode ownership, but an app's own initial float rule does not.
local child, dialog = window('child', false), window('dialog', true)
event('window.open_early', child); event('window.open_early', dialog)
child.floating = true
local group = { members = { tiled, child, dialog } }
tiled.group, child.group, dialog.group = group, group, group
windows[#windows + 1], windows[#windows + 2] = child, dialog
event('window.open', child); event('window.open', dialog)
eq(#child.tags, 1); eq(#dialog.tags, 0)
tiled.group, child.group, dialog.group = nil, nil, nil
child.workspace = { id = 2 }; event('window.move_to_workspace', child); event('config.props_refreshed')
assert(not child.floating); assert(dialog.floating)
-- A moved target is not mutated until the deferred property pass.
tiled.workspace = { id = 2 }
event('window.move_to_workspace', tiled)
assert(tiled.floating)
event('config.props_refreshed') -- fixture overrides workspace 2 to dwindle
assert(not tiled.floating); eq(#tiled.tags, 0)
fs.fullscreen = 0
event('window.fullscreen', fs); assert(not fs.floating)
event('config.props_refreshed'); assert(fs.floating)
-- A pinned owned float gives up ownership and stays floating after moving out.
fresh.pinned = true
event('window.update_state', fresh, { pin = true }); eq(#fresh.tags, 0)
fresh.workspace = { id = 2 }; event('window.move_to_workspace', fresh); event('config.props_refreshed')
assert(fresh.floating)
-- Keyboard and mouse share gap/reservation geometry; repeated snaps retain the first size.
active, drag_target = own, own
M.snap('left')(); eq(own.at.x, 10); eq(own.at.y, 50); eq(own.size.x, 946); eq(own.size.y, 940)
M.snap('right')(); eq(own.at.x, 964)
M.snap('restore')(); eq(own.size.x, 800); eq(own.size.y, 600); eq(own.at.x, 100); eq(#own.tags, 0)
local binding = M.drag()
cursor = { x = 400, y = 400 }
binding(); eq(#timers, 1); assert(timers[1].enabled)
cursor = { x = 2, y = 400 }; timers[1].callback()
local count = #commands
assert(commands[count]:match('^digit msg snap%-zone DP%-1 '))
timers[1].callback(); eq(#commands, count) -- no repeated preview messages
local before_moves = moves
binding(); eq(moves, before_moves + 1); assert(not timers[1].enabled); eq(commands[#commands], 'digit msg snap-zone off')
eq(own.size.x, 946)
-- A native drag cannot rebase its hotspot from Lua: restore once on release, under pointer.
cursor = { x = 400, y = 400 }; binding()
cursor = { x = 700, y = 500 }; timers[1].callback(); eq(own.size.x, 946)
binding(); eq(own.size.x, 800); eq(own.size.y, 600); assert(not timers[1].enabled)
-- Workspace change, close, reload, and callback failure all disarm the sole reusable timer.
for _, name in ipairs({ 'workspace.active', 'workspace.special_active', 'monitor.layout_changed', 'window.close', 'config.unload' }) do
    cursor = { x = own.at.x + 100, y = own.at.y + 100 }; binding()
    assert(timers[1].enabled)
    event(name, own); assert(not timers[1].enabled)
    before_moves = moves; binding(); eq(moves, before_moves)
end
cursor = { x = own.at.x + 100, y = own.at.y + 100 }; binding()
local get_cursor = hl.get_cursor_pos
hl.get_cursor_pos = function() error('simulated cursor failure') end
timers[1].callback(); assert(not timers[1].enabled)
hl.get_cursor_pos = get_cursor; binding()
-- A tiled native drag must not start a snap timer, even if native dragging floats it.
tiled.floating = false; active, drag_target = tiled, tiled
cursor = { x = tiled.at.x + 10, y = tiled.at.y + 10 }
binding(); assert(not timers[1].enabled); binding()
-- Saved restore geometry is reconstructed from window tags in a fresh Lua VM.
active, drag_target = own, own
M.snap('left')()
callbacks = {}; M = dofile(path); event('config.reloaded'); M.snap('restore')()
eq(own.size.x, 800); eq(#own.tags, 0)
-- Even a malicious output name cannot reach the shell command string.
m.name = 'DP-$(touch /tmp/never)'
binding = M.drag(); cursor = { x = own.at.x + 10, y = own.at.y + 10 }; binding()
cursor = { x = 1, y = 400 }; timers[#timers].callback(); binding()
for _, cmd in ipairs(commands) do assert(not cmd:find('touch', 1, true)) end
