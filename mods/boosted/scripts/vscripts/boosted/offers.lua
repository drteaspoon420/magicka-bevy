-- Upgrade offers (H3): seeded weighted rarities over the upgradable numbers
-- of the items a player holds, without blocked, banned or linked
-- duplicates. Offer contents live on the server; clients see a copy.
local M = {}
local settings
local rarities = {}
local offers = {}   -- player -> list of offers
local bans = {}     -- player -> set of keys
local bought = {}   -- player -> "target|key" -> total percent
local serial = 0
local equipment = {} -- player -> held item signature

function M.init(s)
    settings = s
    rarities = {}
    for name, r in pairs(s.Rarities) do
        table.insert(rarities, { name = name, weight = tonumber(r.Weight),
            percent = tonumber(r.Percent), price = tonumber(r.Price) })
    end
    -- pairs() order is unspecified: sort for a reproducible roll.
    table.sort(rarities, function(a, b) return a.weight > b.weight end)
end

local function roll_rarity()
    local total = 0
    for _, r in ipairs(rarities) do total = total + r.weight end
    local roll = Random.Float() * total
    for _, r in ipairs(rarities) do
        roll = roll - r.weight
        if roll <= 0 then return r end
    end
    return rarities[#rarities]
end

local function signature(items)
    local names = {}
    for _, item in ipairs(items) do table.insert(names, item.slot .. "=" .. item.name) end
    table.sort(names)
    return table.concat(names, ",")
end

local function total(player, target, key)
    return ((bought[player] or {})[target .. "|" .. key]) or 0
end

-- The value a percent total gives (the engine resolves the same way).
local function value(base, percent) return base * (1 + percent / 100) end

local function round(v) return math.floor(v * 100 + 0.5) / 100 end

function M.generate(player)
    local items = Upgrades.Items(player)
    equipment[player] = signature(items)
    local candidates = {}
    for _, item in ipairs(items) do
        for _, k in ipairs(Upgrades.Keys()) do
            local base = item.numbers[k.key]
            local blocked = settings.Blocked[k.key] ~= nil
            local banned = (bans[player] or {})[k.key]
            if base ~= nil and base ~= 0 and not blocked and not banned then
                table.insert(candidates, { target = string.lower(item.name), key = k.key, base = base })
            end
        end
    end
    -- Deterministic shuffle with the server RNG.
    for i = #candidates, 2, -1 do
        local j = Random.Int(1, i)
        candidates[i], candidates[j] = candidates[j], candidates[i]
    end
    local list, taken = {}, {}
    for _, c in ipairs(candidates) do
        if #list >= tonumber(settings.OfferCount) then break end
        local partner = settings.Linked[c.key]
        local id = c.target .. "|" .. c.key
        if not taken[id] and not (partner and taken[c.target .. "|" .. partner]) then
            taken[id] = true
            local rarity = roll_rarity()
            local percent = rarity.percent * (tonumber(settings.Nerfed[c.key]) or 1)
            if settings.Inverted[c.key] then percent = -percent end
            local before = total(player, c.target, c.key)
            serial = serial + 1
            table.insert(list, {
                id = serial, target = c.target, key = c.key, rarity = rarity.name,
                percent = percent, price = rarity.price,
                from = round(value(c.base, before)), to = round(value(c.base, before + percent)),
                linked = partner,
            })
        end
    end
    offers[player] = list
    NetTables.Set("boosted", "offers." .. player, list)
end

-- Regenerate when held equipment changed (H6).
function M.check_equipment(player)
    if equipment[player] ~= signature(Upgrades.Items(player)) then M.generate(player) end
end

function M.find(player, id)
    for _, offer in ipairs(offers[player] or {}) do
        if offer.id == id then return offer end
    end
end

-- Apply a bought offer: the key, and its linked partner once.
function M.buy(player, offer)
    Upgrades.Apply(player, offer.target, offer.key, "percent", offer.percent)
    bought[player] = bought[player] or {}
    local id = offer.target .. "|" .. offer.key
    bought[player][id] = total(player, offer.target, offer.key) + offer.percent
    if offer.linked then
        for _, item in ipairs(Upgrades.Items(player)) do
            if string.lower(item.name) == offer.target and item.numbers[offer.linked] ~= nil then
                Upgrades.Apply(player, offer.target, offer.linked, "percent", offer.percent)
                local lid = offer.target .. "|" .. offer.linked
                bought[player][lid] = total(player, offer.target, offer.linked) + offer.percent
            end
        end
    end
    NetTables.Set("boosted", "upgrades." .. player, bought[player])
end

function M.ban(player, key)
    bans[player] = bans[player] or {}
    bans[player][key] = true
    local list = {}
    for k in pairs(bans[player]) do table.insert(list, k) end
    table.sort(list)
    NetTables.Set("boosted", "bans." .. player, list)
end

function M.save()
    local out = { serial = serial, bans = {}, bought = {} }
    for p, set in pairs(bans) do out.bans[tostring(p)] = set end
    for p, b in pairs(bought) do out.bought[tostring(p)] = b end
    return out
end
function M.load(s)
    serial = s.serial or 0
    for p, set in pairs(s.bans or {}) do bans[tonumber(p)] = set end
    for p, b in pairs(s.bought or {}) do bought[tonumber(p)] = b end
end

return M
