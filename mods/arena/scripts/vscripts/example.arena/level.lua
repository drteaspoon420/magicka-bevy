-- Step into the start area to call a goblin wave; clear it to win.
local start, spawned

function OnLevelStart(e)
    local hero = Players.Get(0)
    if not hero or not hero.entity then return end
    start = hero.entity:GetPosition()
    Levels.SetObjective("clear", "#example_arena_objective", false)
    Levels.WatchArea("arena", start, 4)
end

function OnTriggerEnter(e)
    if e.area ~= "arena" or spawned then return end
    spawned = true
    Levels.SpawnWave("goblins", {
        { unit = "Goblin_pirate", position = { x = start.x + 5, y = start.y, z = start.z } },
        { unit = "Goblin_pirate", position = { x = start.x - 5, y = start.y, z = start.z } },
    })
end

function OnWaveComplete(e)
    Levels.SetObjective("clear", "#example_arena_objective", true)
    -- A native trigger action: set a level counter the authored XML can read.
    Levels.Action("Counter", { name = "ArenaCleared", action = "set", value = 1 })
end
