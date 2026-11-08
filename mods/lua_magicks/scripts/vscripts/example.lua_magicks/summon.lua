-- Summons and simulated Magick entities.
local C = require("example.lua_magicks.common")

-- One of five undead, 4 m ahead, fighting for the caster.
local UNDEAD = {
    "ghoul",
    "skeleton_wight_friendly",
    "skeleton_darksoul_arcane",
    "skeleton_darksoul_frost",
    "skeleton_darksoul_lightning",
}
function SUndead(ctx)
    local unit = UNDEAD[Random.Int(1, #UNDEAD)]
    Entities.Summon(unit, C.ahead(ctx.caster, 4), { owner = ctx.caster, undead = true })
end

-- These are native entities with their own per-frame simulation; each
-- takes its pose from the caster, as the built-in Magick does.
local function native(kind)
    return function(ctx) Entities.SpawnNative(kind, ctx.caster) end
end
SElemental = native("elemental")
SDeath = native("death")
SPhoenix = native("phoenix")
Tornado = native("tornado")
Vortex = native("vortex")
Wave = native("wave")
Portal = native("portal")
Earthquake = native("earthquake")
Conflagration = native("conflagration")
Amalgameddon = native("amalgameddon")
SummonCross = native("cross")
SpawnSlime = native("slime")
SpawnSlimeOverkill = native("slime_overkill")
EtherealClone = native("clone")
