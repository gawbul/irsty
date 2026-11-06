-- slap.luau: /slap <nick> slaps someone with a large trout, irssi-style.
-- It acts for you, so it needs the send capability:
--
--   [[script]]
--   path = "scripts/slap.luau"
--   allow = ["send"]

irsty.command("slap", function(args, here)
    local nick = args:match("^%s*(%S+)")
    if not nick then
        irsty.print("Usage: /slap <nick>")
        return
    end
    if not here or not here.buffer then
        irsty.print("/slap needs a channel or query")
        return
    end
    irsty.exec("/me slaps " .. nick .. " around a bit with a large trout")
end)
