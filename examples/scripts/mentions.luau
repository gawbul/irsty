-- mentions.luau: keeps the lines that mention you while you're elsewhere,
-- and lists them with /mentions. Needs no capabilities.
--
--   [[script]]
--   path = "scripts/mentions.luau"

local kept = {}
local MAX = 50

irsty.on("message", function(e)
    if not e.highlight or e.own then
        return
    end
    table.insert(kept, string.format("%s %s/%s <%s> %s",
        os.date("%H:%M", e.time), e.network, e.buffer or "", e.nick or "?", e.text))
    if #kept > MAX then
        table.remove(kept, 1)
    end
end)

irsty.command("mentions", function(args)
    if args == "clear" then
        kept = {}
        irsty.print("Mentions cleared")
        return
    end
    if #kept == 0 then
        irsty.print("No mentions kept")
        return
    end
    for _, line in kept do
        irsty.print(line)
    end
end)
