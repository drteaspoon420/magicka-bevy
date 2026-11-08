-- Magicks that hit: target Magicks, bolts, sprays and grease.
local C = require("example.lua_magicks.common")

-- Votal: a random one of seven 50-point Status hits on the target.
function Votal(ctx)
    local caster, target = ctx.caster, C.target(ctx)
    local hit = { attack = "status", amount = 50 }
    local roll = Random.Int(0, 6)
    if roll == 0 then hit.element = "poison"
    elseif roll == 1 then hit.element = "water"
    elseif roll == 2 then hit.attack = "knockdown"
    elseif roll == 3 then
        Spells.Hit(target, { attack = "status", element = "water", amount = 1 }, { source = caster })
        hit.element = "cold"
    elseif roll == 4 then hit.element = "cold"
    elseif roll == 5 then hit.attack, hit.amount = "bleed", 30
    else hit.element = "fire" end
    Spells.Hit(target, hit, { source = caster })
end

-- One in four knocks the target down; otherwise a 10-point Status hit.
function KnockDownChance(ctx)
    local knockdown = Random.Int(0, 3) == 0
    Spells.Hit(C.target(ctx), {
        attack = knockdown and "knockdown" or "status",
        amount = 10,
    }, { source = ctx.caster })
end

function InflictBleedDamage(ctx)
    local target = C.target(ctx)
    Spells.Status(target, "bleeding", {
        dps = (target:GetMaxHealth() or 0) / 30, magnitude = 3,
    })
end

function InflictPoisonDamage(ctx)
    Spells.Status(C.target(ctx), "poisoned", { dps = 100, magnitude = 3 })
end

-- Zap: an 850-point bolt along the facing, range 15. A wet caster can be
-- struck by their own lightning.
function ChainLightning(ctx)
    local caster = ctx.caster
    Spells.Lightning(caster, {
        damage = 850, range = 15, hit_owner = caster:HasStatus("wet"),
    })
    World.Cue("magick_thunderbolt", { position = caster:GetPosition() })
    World.Shake(1.5, 0.333)
end

-- Vlad's zap: the same 850-point bolt, aimed at the target.
function VladZap(ctx)
    Spells.Lightning(ctx.caster, { target = C.target(ctx), damage = 850 })
end

-- Twenty homing missiles over a second (a native spray effect).
function JudgementSpray(ctx)
    Entities.SpawnNative("judgement_spray", ctx.caster)
    World.Cue("woot_magick_judgment_fire", {
        position = ctx.caster:GetPosition(), bank = C.ADDITIONAL,
    })
end

function Grease(ctx)
    World.Cue("magick_grease", { position = ctx.caster:GetPosition() })
    Entities.SpawnNative("grease_spray", ctx.caster)
end

-- Seven grease fields 1-4 m around the caster, each turning the running
-- direction a further pi/7 * i.
function GreaseSplash(ctx)
    local caster = ctx.caster
    local p, d = caster:GetPosition(), caster:GetForward()
    for i = 0, 6 do
        local a = math.pi / 7 * i
        local s, c = math.sin(a), math.cos(a)
        d = { x = d.x * c + d.z * s, y = d.y, z = -d.x * s + d.z * c }
        local distance = 1 + Random.Float() * 3
        Spells.GreaseField({ x = p.x + d.x * distance, y = 0, z = p.z + d.z * distance }, caster)
    end
end

-- A lobbed plasma ball (a native missile with its own conditions).
function ProppMagick(ctx)
    Entities.SpawnNative("propp", ctx.caster)
end
