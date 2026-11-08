-- Shared pieces of the reference Magicks: aim, target selection and the
-- native sound banks.
local M = {}

M.SPELLS, M.CHARACTERS, M.ADDITIONAL = 16, 32, 512

function M.same(a, b)
    return a ~= nil and b ~= nil and a:GetHandle() == b:GetHandle()
end

-- The point `distance` metres along the caster's facing.
function M.ahead(caster, distance)
    local p, f = caster:GetPosition(), caster:GetForward()
    return { x = p.x + f.x * distance, y = p.y + f.y * distance, z = p.z + f.z * distance }
end

function M.fail(position)
    World.Cue("magick_fail", { position = position })
end

-- Charm/Confuse target selection (Helper.FacingTarget): among living
-- characters within `radius` other than the caster that pass `accept`,
-- the one whose flat direction is closest to the caster's facing.
function M.facing_target(caster, radius, accept)
    local p, f = caster:GetPosition(), caster:GetForward()
    local fl = math.sqrt(f.x * f.x + f.z * f.z)
    local best, best_dot = nil, -math.huge
    for _, e in ipairs(Entities.FindInRadius(p, radius)) do
        if not M.same(e, caster) and e:IsAlive() and (accept == nil or accept(e)) then
            local q = e:GetPosition()
            local dx, dz = q.x - p.x, q.z - p.z
            local l = math.sqrt(dx * dx + dz * dz)
            local dot = l > 0 and (dx * f.x + dz * f.z) / (l * fl) or 1
            if dot > best_dot then best, best_dot = e, dot end
        end
    end
    return best
end

-- Characters within `radius` whose factions share no bit with the caster's.
function M.enemies(caster, center, radius)
    local out, faction = {}, caster:GetFaction() or 0
    for _, e in ipairs(Entities.FindInRadius(center, radius)) do
        if not M.same(e, caster) and e:IsAlive() and ((e:GetFaction() or 0) & faction) == 0 then
            out[#out + 1] = e
        end
    end
    return out
end

-- A uniform point in the flat disc of `radius` around `center`.
function M.disc_point(center, radius)
    local r = math.sqrt(Random.Float()) * radius
    local a = Random.Float() * 2 * math.pi
    return { x = center.x + r * math.cos(a), y = center.y, z = center.z + r * math.sin(a) }
end

-- The handler's target (the native aim), else the caster.
function M.target(ctx)
    return ctx.target or ctx.caster
end

return M
