-- Weather and sky Magicks. All are refused indoors.
local C = require("example.lua_magicks.common")

function Rain(ctx)
    World.Weather("rain", { owner = ctx.caster })
end

function Blizzard(ctx)
    World.Weather("blizzard", { owner = ctx.caster })
end

-- Thunderbolt: the aimed bolt 8 m ahead of the caster.
function ThunderB(ctx)
    local p = ctx.caster:GetPosition()
    if World.IsIndoors() then return C.fail(p) end
    Spells.Thunderbolt(ctx.caster, p, ctx.caster:GetForward())
end

-- Thunderstorm: Rain, then after 2 s, 1-2 strikes every 0.5-1 s at
-- uniform points within 20 m of the caster for as long as the rain
-- lasts (17.5 s). A recast restarts the storm.
local storm = { generation = 0 }
function ThunderS(ctx)
    if World.IsIndoors() then return end
    local caster = ctx.caster
    World.Weather("rain", { owner = caster })
    storm.generation = storm.generation + 1
    local generation, ends = storm.generation, GameRules.GetGameTime() + 17.5
    local center = caster:GetPosition()
    local function bolts()
        if generation ~= storm.generation or GameRules.GetGameTime() >= ends then return end
        if caster:IsValid() then center = caster:GetPosition() end
        for _ = 1, Random.Int(1, 2) do
            Spells.Strike(caster, C.disc_point(center, 20))
        end
        Timers.After(0.5 + Random.Float() * 0.5, bolts)
    end
    Timers.After(2.0, bolts)
end

-- Meteor Shower: the sky dims to 0.4 for 17.5 s; until 2.5 s remain,
-- 1-2 Earth 5 + Fire 5 meteors fall every 0.4-0.8 s from 146 m above
-- uniform points within 20 m of the caster, at 20 m/s with splash 4. A
-- recast while it rages only refreshes it.
local shower = nil
function MeteorS(ctx)
    local caster = ctx.caster
    if World.IsIndoors() then return C.fail(caster:GetPosition()) end
    local now = GameRules.GetGameTime()
    if shower and now < shower.ends then
        shower.ends = now + 17.5
        return
    end
    shower = { ends = now + 17.5, caster = caster, center = caster:GetPosition() }
    local current = shower
    World.SetLight(0.4)
    local function volley()
        if shower ~= current then return end
        local t = GameRules.GetGameTime()
        if t >= current.ends then
            World.SetLight(1.0)
            shower = nil
            return
        end
        if current.ends - t > 2.5 then
            if current.caster:IsValid() then current.center = current.caster:GetPosition() end
            World.Cue("magick_meteor_preblast")
            for _ = 1, Random.Int(1, 2) do
                local p = C.disc_point(current.center, 20)
                p.y = current.center.y + 146
                Spells.Projectile(current.caster, {
                    elements = { earth = 5, fire = 5 },
                    position = p,
                    velocity = { x = 0.01, y = -20, z = -0.01 },
                    splash = 4,
                })
                World.Cue("magick_meteor_preblast", { position = p })
            end
        end
        Timers.After(0.4 + Random.Float() * 0.4, volley)
    end
    volley()
end

-- Napalm's airstrike is a simulated native entity (the plane's track).
function Napalm(ctx)
    Entities.SpawnNative("napalm", ctx.caster)
end
