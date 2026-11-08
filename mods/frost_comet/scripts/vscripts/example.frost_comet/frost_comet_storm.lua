-- Frost Comet Storm: the native Meteor Shower's structure (see the
-- reference mod's meteor_shower.lua) with ice comets. Outdoors only; the
-- sky darkens, then 1-2 comets fall every 0.4-0.8 s for 15 s somewhere in
-- the 20 m disc around the caster. Each comet is an Ice + Cold missile
-- whose splash freezes what it lands next to.
local DURATION = 15.0
local RADIUS, HEIGHT = 20.0, 146.0
local FALL = { x = 0.01, y = -20.0, z = -0.01 }
local SPLASH = 6.0

local function comet(caster, center)
    local r = math.sqrt(Random.Float()) * RADIUS
    local angle = Random.Float() * 2 * math.pi
    local position = {
        x = center.x + r * math.cos(angle),
        y = center.y + HEIGHT,
        z = center.z + r * math.sin(angle),
    }
    Spells.Projectile(caster, {
        elements = { ice = 5, cold = 5 },
        position = position,
        velocity = FALL,
        splash = SPLASH,
    })
    World.Cue("magick_meteor_preblast", { position = position })
end

function OnSpellStart(ctx)
    local caster = ctx.caster
    local center = caster:GetPosition()
    if World.IsIndoors() then
        World.Cue("magick_fail", { position = center })
        return
    end
    World.SetLight(0.4)
    World.Cue("magick_blizzard")
    local remaining = DURATION
    local function volley()
        -- The storm follows its caster while they live.
        if caster:IsValid() then center = caster:GetPosition() end
        World.Cue("magick_meteor_preblast")
        for _ = 1, Random.Int(1, 2) do comet(caster, center) end
        local wait = 0.4 + Random.Float() * 0.4
        remaining = remaining - wait
        if remaining > 0 then
            Timers.After(wait, volley)
        else
            Timers.After(2.5, function() World.SetLight(1.0) end)
        end
    end
    volley()
end
