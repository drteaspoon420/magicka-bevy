-- Warm hands: a small heal every two seconds while the staff is held.
function OnIntervalThink(ctx)
    Entities.Heal(ctx.target, 10)
end
