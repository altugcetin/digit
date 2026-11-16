-- Window and layer rules digit needs. Written by digit.
-- The desktop icons' surface is remapped to take keyboard focus; a fade would dim every icon.
hl.layer_rule({ name = "digit-no-anim", match = { namespace = "^(digit|digit-wallpapers|digit-desktop-icons)$" }, no_anim = true })
