-- Storm Lance: a real Lightning+Arcane beam from the caster, plus a burst.
function OnSpellStart(ctx)
    local caster = ctx.caster
    Spells.CastElements(caster, { "lightning", "arcane" }, {
        delivery = "beam",
        target_position = ctx.target_position,
    })
end
