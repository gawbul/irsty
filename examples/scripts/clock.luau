-- clock.luau: says the time in the current buffer on the hour, and shows
-- timers and events together. /clock stops it. Needs no capabilities.

local function chime()
    irsty.print("It is " .. os.date("%H:%M"))
end

-- Wait for the next hour, then chime every hour.
local now = os.time()
local wait = (3600 - now % 3600) * 1000
local hourly
local first = irsty.timer(wait, function()
    chime()
    hourly = irsty.every(3600 * 1000, chime)
end)

irsty.command("clock", function()
    irsty.cancel(first)
    if hourly then
        irsty.cancel(hourly)
    end
    irsty.print("The clock stops")
end)
