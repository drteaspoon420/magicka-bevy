-- Magicks on the caster (and the global ones that need no target).
local C = require("example.lua_magicks.common")

function Haste(ctx)
    Entities.Haste(ctx.caster)
    World.Cue("magick_haste", { position = ctx.caster:GetPosition() })
end

function Invisibility(ctx)
    Entities.SetInvisible(ctx.caster, 15)
    World.Cue("magick_invisibility", { position = ctx.caster:GetPosition() })
end

function Levitate(ctx)
    Entities.Levitate(ctx.caster)
end

function Etherealize(ctx)
    local p = ctx.caster:GetPosition()
    Entities.SetEthereal(ctx.caster, true)
    World.Effect("magick_etherealize", p, ctx.caster:GetForward())
    World.Cue("chr_daemon_etherealize3", { position = p, bank = C.CHARACTERS })
end

function Corporealize(ctx)
    World.Corporealize(ctx.caster:GetPosition())
end

function Grow(ctx)
    Entities.Resize(ctx.caster, "grow")
end

function Shrink(ctx)
    Entities.Resize(ctx.caster, "shrink")
end

-- 10 m along the facing; the move needs ground below the destination.
function Teleport(ctx)
    Entities.Blink(ctx.caster, C.ahead(ctx.caster, 10))
end

-- The longest-dead eligible player rises 4 m ahead with 10 % health.
function Revive(ctx)
    Entities.Revive(C.ahead(ctx.caster, 4), {
        from = ctx.caster:GetPosition(),
        health = 0.1,
    })
end

-- Half damage from every element for 10 s (the native self aura), with
-- the withdrawal cue when 4 s remain.
function PerformanceEnchantment(ctx)
    local caster = ctx.caster
    Modifiers.Apply(caster, "example.lua_magicks:performance", { duration = 10 })
    World.Cue("woot_magick_enchantment_buff", {
        position = caster:GetPosition(), bank = C.ADDITIONAL,
    })
    Timers.After(6, function()
        if caster:IsValid() then
            World.Cue("woot_magick_enchantment_debuff", {
                position = caster:GetPosition(), bank = C.ADDITIONAL,
            })
        end
    end)
end

function Nullify(ctx)
    World.Nullify()
end

function TimeWarp(ctx)
    World.TimeWarp(ctx.caster)
end
