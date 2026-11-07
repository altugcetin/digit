local path = assert(arg[1])
local G = dofile(path)
local function eq(actual, expected)
    assert(actual == expected, tostring(actual) .. ' ~= ' .. tostring(expected))
end
local edges = { left = true, right = true, top = true, bottom = true }
for _, v in ipairs({ { 0, 0, 'tl' }, { 999, 0, 'tr' }, { 0, 799, 'bl' }, { 999, 799, 'br' },
    { 0, 400, 'left' }, { 999, 400, 'right' }, { 500, 0, 'max' } }) do
    eq(G.zone(v[1], v[2], 1000, 800, edges), v[3])
end
eq(G.zone(13, 400, 1000, 800, edges), nil)
eq(G.zone(23, 400, 1000, 800, edges, 'left'), 'left')
eq(G.zone(25, 400, 1000, 800, edges, 'left'), nil)
eq(G.zone(2, 63, 1000, 800, edges, 'left'), 'tl')
eq(G.zone(2, 79, 1000, 800, edges, 'tl'), 'tl')
eq(G.zone(2, 81, 1000, 800, edges, 'tl'), 'left')
local m = G.monitor({ name = 'DP-1', width = 2560, height = 1440, scale = 1.25, transform = 1, x = -1152, y = 80 })
eq(m.w, 1152); eq(m.h, 2048); eq(m.x, -1152)
local other = { name = 'DP-2', x = 0, y = 500, w = 1920, h = 1080 }
local blocked = G.edges(m, { m, other }, -1, 600)
eq(blocked.right, false)
eq(G.zone(m.w - 1, 520, m.w, m.h, blocked, 'right'), nil)
eq(G.edges(m, { m, other }, -1, 300).right, true)
-- Crossing a seam at its endpoint must not suddenly maximize instead.
eq(G.zone(999, 2, 1000, 800, { left = true, right = false, top = true, bottom = true }, 'max'), nil)
local area = G.workarea({ w = 1920, h = 1080 }, { top = 40, bottom = 90, left = 10, right = 20 }, { top = 8, right = 8, bottom = 8, left = 8 })
eq(area.x, 18); eq(area.y, 48); eq(area.w, 1874); eq(area.h, 934)
local left, right = G.rect(area, 'left', 10), G.rect(area, 'right', 10)
eq(left.x, area.x); eq(left.w + right.w + 10, area.w)
eq(right.x - left.x - left.w, 10)
eq(right.x + right.w, area.x + area.w)
local quarter = G.rect(area, 'br', { left = 10, top = 6 })
eq(quarter.x, right.x); eq(quarter.y + quarter.h, area.y + area.h)
eq(quarter.h * 2 + 6, area.h)
local max = G.rect(area, 'max', 10)
eq(max.x, area.x); eq(max.h, area.h)
local seen = {}
for i = 0, 99 do
    local r = G.cascade(area, i)
    assert(r.x >= area.x and r.y >= area.y and r.x + r.w <= area.x + area.w and r.y + r.h <= area.y + area.h)
    local key = r.x .. ':' .. r.y
    assert(not seen[key], 'cascade stacked two windows exactly')
    seen[key] = true
end
local back = G.restore({ w = 800, h = 600 }, { x = -200, y = 500 }, { x = 0.75, y = 0.25 }, m)
eq(back.x + m.x + back.w * 0.75, -200)
eq(back.y + m.y + back.h * 0.25, 500)

local alone = G.cascade({ x = 0, y = 40, w = 1000, h = 800 }, 5, {})
assert(alone.w == 700 and alone.h == 560, 'a lone window takes 70 percent')
assert(alone.x == 150 and alone.y == 160, 'a lone window is centred in the work area')
local huge = G.cascade({ x = 0, y = 0, w = 5120, h = 2160 }, 0, {})
assert(huge.w == 1280 and huge.h == 860, 'a lone window stays a sensible size on a huge screen')
local bigger = G.scaled({ x = 10, y = 20, w = 1000, h = 800 }, { w = 500, h = 400 }, 1.2)
assert(bigger.w == 600 and bigger.h == 480 and bigger.x == 210 and bigger.y == 180, 'scaling centres the window')
local capped = G.scaled({ x = 0, y = 0, w = 1000, h = 800 }, { w = 950, h = 780 }, 1.2)
assert(capped.w == 1000 and capped.h == 800, 'scaling stops at the work area')
local tiny = G.scaled({ x = 0, y = 0, w = 1000, h = 800 }, { w = 250, h = 170 }, 0.8)
assert(tiny.w == 240 and tiny.h == 160, 'scaling stops at a usable size')
local moved = G.relative({ x = 0, y = 40, w = 1000, h = 800 }, { x = 0, y = 0, w = 2000, h = 1000 }, { x = 500, y = 440, w = 500, h = 400 })
assert(moved.x == 1000 and moved.y == 500 and moved.w == 1000 and moved.h == 500, 'the share of the screen survives a monitor change')
local inside = G.relative({ x = 0, y = 0, w = 1000, h = 800 }, { x = 0, y = 0, w = 500, h = 400 }, { x = 900, y = 700, w = 1000, h = 800 })
assert(inside.x == 0 and inside.y == 0 and inside.w == 500 and inside.h == 400, 'a moved window stays on the monitor')
local first = G.cascade(area, 0)
local after_reload = G.cascade(area, 0, { first })
assert(first.x ~= after_reload.x or first.y ~= after_reload.y)

-- Odd gaps and odd work areas share rounded boundaries, without spilling past the outer edge.
for _, width in ipairs({ 1000, 1001 }) do
    local a = { x = 8, y = 40, w = width, h = 801 }
    local l, r = G.rect(a, 'left', 5), G.rect(a, 'right', 5)
    eq(r.x - l.x - l.w, 5); eq(r.x + r.w, a.x + a.w)
    local b = G.rect(a, 'br', 5, 2)
    eq(b.x + b.w + 2, a.x + a.w); eq(b.y + b.h + 2, a.y + a.h)
end
local no_gaps = G.rect({ x = 0, y = 40, w = 1000, h = 800 }, 'max', 0, 3)
eq(no_gaps.y - 3, 40); eq(no_gaps.y + no_gaps.h + 3, 840)

local fractional = { x = 8.25, y = 40.75, w = 1200.5, h = 800.5 }
local placed = G.cascade(fractional, 0)
assert(placed.x >= fractional.x and placed.y >= fractional.y)
assert(placed.x + placed.w <= fractional.x + fractional.w and placed.y + placed.h <= fractional.y + fractional.h)
