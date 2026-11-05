-- Session defaults: environment, layouts and small fixes. Written by digit;
-- change any of it in user.lua, which loads after this.

hl.env("XCURSOR_SIZE", "24")
hl.env("QT_QPA_PLATFORMTHEME", "qt6ct")

hl.config({
    dwindle = { preserve_split = true },
    master = { new_status = "master" },
    misc = {
        force_default_wallpaper = 0,
        disable_hyprland_logo = true,
    },
    cursor = { no_warps = true },
})

{{settings}}