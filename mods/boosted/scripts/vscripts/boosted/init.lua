-- Boosted (H1-H6). Gold from kills buys upgrade offers; purchases arrive
-- as client requests that the engine runs exactly once per request ID.
local currencies = require("boosted.currencies")
local offers = require("boosted.offers")
local settings = Settings.Load("npc/boosted/settings.kv").Boosted
offers.init(settings)

local players = {}

function OnPlayerJoin(p)
    players[p.slot] = true
    if not currencies.has(p.slot) then
        currencies.set(p.slot, tonumber(settings.StartingGold))
    end
    offers.generate(p.slot)
end

function OnNpcKilled(e)
    for slot in pairs(players) do currencies.earn(slot, tonumber(settings.KillReward)) end
end

function OnGameStart()
    -- Equipment changes (pickups, respawns) refresh the offers.
    Timers.Every(1, function()
        for slot in pairs(players) do offers.check_equipment(slot) end
    end)
end

-- boosted.buy { item = <offer> } / boosted.ban { key = <key> }
function OnModRequest(r)
    local player = r.player
    if r.channel == "boosted.buy" then
        local wanted = r.data and r.data.item
        local offer = wanted and offers.find(player, wanted.id)
        if not offer then return { error = "that offer is no longer available" } end
        if not currencies.spend(player, offer.price) then
            return { error = "not enough gold" }
        end
        offers.buy(player, offer)
        offers.generate(player)
        return { ok = true, bought = offer.key, target = offer.target, gold = currencies.get(player) }
    elseif r.channel == "boosted.ban" then
        local key = r.data and r.data.key
        if type(key) ~= "string" then return { error = "ban which key?" } end
        if not currencies.spend(player, tonumber(settings.BanPrice)) then
            return { error = "not enough gold" }
        end
        offers.ban(player, key)
        offers.generate(player)
        return { ok = true, banned = key }
    end
    return { error = "unknown request" }
end

function SaveState()
    return { gold = currencies.save(), offers = offers.save(), players = players }
end
function LoadState(s)
    currencies.load(s.gold)
    offers.load(s.offers or {})
    for slot in pairs(s.players or {}) do players[tonumber(slot)] = true end
end
