-- Magicks that take control of other characters. Selection happens here;
-- the effect on each character is the native routine.
local C = require("example.lua_magicks.common")

-- Every enemy within 6 m flees from the caster.
function Fear(ctx)
    local caster = ctx.caster
    local p = caster:GetPosition()
    World.Effect("magick_fear", p, { x = 1, y = 0, z = 0 })
    for _, e in ipairs(C.enemies(caster, p, 6)) do
        Entities.Fear(e, caster)
    end
    World.Cue("magick_fear", { position = p })
end

-- The enemy within 14 m closest to the caster's facing fights for the
-- caster for 15 s.
function Charm(ctx)
    local caster = ctx.caster
    local faction = caster:GetFaction() or 0
    local target = C.facing_target(caster, 14, function(e)
        return ((e:GetFaction() or 0) & faction) == 0
    end)
    if target == nil then return C.fail(caster:GetPosition()) end
    World.Cue("magick_charm", { position = target:GetPosition() })
    Entities.Charm(target, caster, 15)
end

-- The character within 14 m closest to the caster's facing (any faction)
-- is confused.
function Confuse(ctx)
    local caster = ctx.caster
    local target = C.facing_target(caster, 14)
    if target == nil then return C.fail(caster:GetPosition()) end
    Entities.Confuse(target, caster)
end

-- The non-ethereal character closest to the point 3 m ahead, within 6 m
-- of it, is polymorphed.
function Polymorph(ctx)
    local caster = ctx.caster
    for _, e in ipairs(Entities.FindInRadius(C.ahead(caster, 3), 6)) do
        if not e:IsEthereal() then
            Entities.Polymorph(e)
            return
        end
    end
    C.fail(caster:GetPosition())
end

-- CTD: one character at random within 20 m (allies and the caster
-- included, bosses with more than 100000 health excluded) is terminated.
function Ctd(ctx)
    local caster = ctx.caster
    local p = caster:GetPosition()
    local pool = {}
    for _, e in ipairs(Entities.FindInRadius(p, 20)) do
        if e:IsAlive() and (e:GetMaxHealth() or 0) <= 100000 then pool[#pool + 1] = e end
    end
    if #pool == 0 then return C.fail(p) end
    local victim = pool[Random.Int(1, #pool)]
    local at = victim:GetPosition()
    World.Effect("magick_ctd", at)
    World.Cue("magick_ctd", { position = at, bank = C.ADDITIONAL })
    Entities.Terminate(victim)
end

-- Everything within 15 m is pulled towards the caster.
function TractorPull(ctx)
    Spells.Push(ctx.caster:GetPosition(), { owner = ctx.caster, pull = true })
end
