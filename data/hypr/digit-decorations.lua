-- Look and feel of windows. Written by digit: Settings > Desktop > Corners sets the rounding,
-- Settings > Compositor the gaps and borders.
-- Change the rest in hyprland.lua, which loads after this.

hl.config({
    general = {
        resize_on_border = true,
        col = { active_border = "rgba(ffffff47)", inactive_border = "rgba(ffffff14)" },
    },
    decoration = {
        rounding = {{rounding}},
        rounding_power = 2,
        shadow = { enabled = true, range = 14, render_power = 3, color = 0x66000000 },
        blur = { enabled = false },
    },
    animations = { enabled = true },
})

hl.curve("linear", { type = "bezier", points = { { 0, 0 }, { 1, 1 } } })
hl.curve("md3_standard", { type = "bezier", points = { { 0.2, 0 }, { 0, 1.0 } } })
hl.curve("md3_decel", { type = "bezier", points = { { 0.05, 0.7 }, { 0.1, 1 } } })
hl.curve("md3_accel", { type = "bezier", points = { { 0.3, 0 }, { 0.8, 0.15 } } })
hl.curve("soft", { type = "bezier", points = { { 0.26, 0.26 }, { 0.15, 1 } } })
hl.curve("md2", { type = "bezier", points = { { 0.4, 0 }, { 0.2, 1 } } })

hl.animation({ leaf = "global", enabled = true, speed = 10, bezier = "default" })
hl.animation({ leaf = "windows", enabled = true, speed = 3, bezier = "soft" })
hl.animation({ leaf = "windowsIn", enabled = true, speed = 3, bezier = "md3_decel", style = "popin 85%" })
hl.animation({ leaf = "windowsOut", enabled = true, speed = 3, bezier = "md3_accel", style = "popin 85%" })
hl.animation({ leaf = "border", enabled = true, speed = 2, bezier = "linear" })
hl.animation({ leaf = "fade", enabled = true, speed = 3, bezier = "md3_decel" })
hl.animation({ leaf = "layersIn", enabled = true, speed = 3, bezier = "md2", style = "fade" })
hl.animation({ leaf = "layersOut", enabled = true, speed = 3, bezier = "md2", style = "fade" })
hl.animation({ leaf = "workspaces", enabled = true, speed = 3, bezier = "md3_standard", style = "slidefadevert 20%" })
hl.animation({ leaf = "specialWorkspaceIn", enabled = true, speed = 3, bezier = "md3_standard", style = "slidefade left 10%" })
hl.animation({ leaf = "specialWorkspaceOut", enabled = true, speed = 3, bezier = "md3_standard", style = "slidefade right 10%" })
