-- Window and layer rules digit needs. Written by digit.
-- The desktop icons' surface is remapped to take keyboard focus; a fade would dim every icon.
-- The desktop clock hides while the lock screen's settles onto it (digit-unlock), and the two
-- swap with the same picture; a fade would dim the clock.
hl.layer_rule({ name = "digit-no-anim", match = { namespace = "^(digit|digit-wallpapers|digit-desktop|digit-desktop-icons|digit-unlock)$" }, no_anim = true })
