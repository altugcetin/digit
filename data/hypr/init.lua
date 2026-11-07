-- digit's part of the Hyprland config, loaded by require("digit") in hyprland.lua.
-- digit writes every file in this folder; changes made here are replaced.
require("digit.digit-base")
require("digit.digit-decorations")
require("digit.digit-rules")
require("digit.digit-workspaces")
require("digit.digit-monitors")
require("digit.digit-primary")
-- A fault in the floating module must never take the keybinds down with it.
if not pcall(require, "digit.digit-floating") then
    package.loaded["digit.digit-floating"] = {
        drag = hl.dsp.window.drag,
        snap = function() return function() end end,
        to_monitor = function() return function() end end,
        arrow = function(_, otherwise) return otherwise end,
        outside = function(action) return action end,
    }
end
require("digit.digit-keybinds")
require("digit.digit-autostart")
