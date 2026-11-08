-- Gold per player (H1). Balances replicate through the "boosted" net table
-- and are saved with the match; clients only ever display them.
local M = {}
local gold = {}

function M.get(player) return gold[player] or 0 end
function M.has(player) return gold[player] ~= nil end

function M.set(player, amount)
    gold[player] = amount
    NetTables.Set("boosted", "gold." .. player, amount)
end

function M.earn(player, amount) M.set(player, M.get(player) + amount) end

-- Returns false (and changes nothing) when the player cannot afford it.
function M.spend(player, amount)
    if M.get(player) < amount then return false end
    M.set(player, M.get(player) - amount)
    return true
end

function M.save()
    local out = {}
    for player, amount in pairs(gold) do out[tostring(player)] = amount end
    return out
end
function M.load(saved)
    for player, amount in pairs(saved or {}) do M.set(tonumber(player), amount) end
end

return M
