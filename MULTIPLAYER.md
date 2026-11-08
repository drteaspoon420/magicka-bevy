# Multiplayer

Run commands from the repository root. Games can be played on one computer, over
a LAN or over the internet. Retail content is read as data; these commands
execute only the rewritten binaries.

## Build and automated verification

```sh
./bin/viewer-qa multiplayer
```

This single wrapper runs authority/transport/pickup/presentation tests, checks
formatting and strict Clippy, builds release binaries, and starts a dedicated
server plus two rendered clients, then stops a second host to verify both
clients' disconnect notices. It also keeps an observer connected while a client
leaves and a replacement reuses the vacant slot, capturing before/removal/rejoin
states. It saves separate test and rendering logs in
`output/.cache/viewer-qa/`. Individual stages remain available as
`network-physics`, `network-demo`, `network-combat`, `network-disconnect` and
`network-rejoin`.

All builds, tests and lint commands use the lockfile and one Cargo build job.
The headless stage includes Revive, checkpoint, equipment lifecycle and campaign
regressions; hosted and lifecycle tests run serially.
Combat captures start with debug windows closed so the action stays visible.
The harness still verifies host-accepted names and robes; the demo stage retains
the player setup window for UI inspection.

Tests have a 2-minute deadline; builds, lint and viewer tests (which may link the
viewer) have a 10-minute deadline. Headless stages print progress and a log path
every 15 seconds. Each rendered client has a 2-minute deadline and the test
server has a 5-minute deadline. Timeout allows 10 seconds before forced cleanup,
fails the run and retains logs. Exit cleanup signals each test process group and
forces termination after a 5-second grace period without an unbounded wait.
`./bin/viewer-qa network-timeout-check` deliberately stalls a parent and child,
checking both deadline termination and exit cleanup; the full run includes it.

The modern wire protocol is now version 18. Version 18 adds kicking and the
host's choice to allow the debug respawn. Version 17 sends what spells leave
behind: ground decals, damage numbers and blast lights. Version 16 secures
sessions (see Security). Version 15 added editor commands. Version 14 adds per-wizard
presentation to the world state: a compact record per wizard with
something to draw (self-shield kind and projection inputs, aura halo tint,
IceBlade trail and ArcaneBlade strips) and one-shot pulses with serials
(lightning zap flashes) that each client draws once, so self-shields,
armour overlays, halos, weapon trails and zap flashes show on remote
wizards (`research/multiplayer-sync-gaps.md`, 2026-10-30). Version 13
acknowledged departures and preserved parked players. Version 12 replaces full
snapshots with per-client baselines and deltas: the host keeps each
connection's recent frames and sends a delta against the newest frame that
connection acknowledged (a one-byte-class `BaselineAck` on its own
unreliable channel), or a full frame when it has none. The encoder proves
every delta by reconstructing the frame, so clients only ever present frames
the host actually sent; a client missing a base drops the delta and the host
recovers from its acknowledgement or with a full frame. Emitters carry a
stable start time instead of a per-frame clock, frames carry the host clock,
and one-shot mod particles/sounds travel as identified events that each
client plays once (late joiners skip history). Mod UI net tables replicate
in frames. Measured on Havindr with two idle clients, state traffic fell
from ~30.1 KB/s to ~6.5 KB/s per client. An optional presentation interest
radius (`LoopbackHost::set_interest_radius`) culls far one-shot events and
emitters for players only; gameplay state is never culled.

Version 11 moves admission into
the hello: a joining player submits its selected profile (name, robe, colour
and persistent profile ID) and the host validates it before any wizard exists,
so no placeholder `Player N` avatar is ever published. Invalid names or robes,
full lobbies, disabled roles and mismatched mod content are refused with a
reason code (`RefusalCode`), and a robe the host cannot load is refused rather
than replaced. Hellos are sent in a short burst and then retried every 500 ms
(bounded; the client reports a timeout), and never after admission; liveness is
a one-byte heartbeat every 250 ms (or 15 updates) that the host echoes for
round-trip timing. Held input moved from the ordered-reliable channel to a
sequenced-unreliable one, and each packet carries the previous three samples
so a lost press/release edge still arrives. Every message type is counted by
`magicka_network::telemetry` (payload bytes, counts, RTT/jitter) with JSON/CSV
export, and `NetConditions` adds receive-side latency, jitter and loss for QA.
The frontend's Join now opens the profile page first: new profiles are named
before they are saved, and any profile can be renamed there.

Version 9 added lobby policy, seats
and roles. Version 10 adds a world presentation for state that host scene
scripts create, because clients run no scripts: animated level-part playback,
live book and item pickups (script-spawned ones are created on the client, and
handles are mapped host→client), tutorial hint, dialog and dark-screen targets,
element/cast-type/push gating, the allowed-magick mask, UI-enabled, and the draw
inputs of beams, lightning bolts, shields, barriers, mines, arcane blasts and
missiles. Each computer shows its own copy of a tutorial dialog to its players
and gates only their input until they dismiss it. The host's own dismissal
runs the continuation trigger, so tutorial progress still follows the host.
Clients without a wizard (spectators, dungeon masters, invaders) get no
dialogs and no element HUD. Queue snapshots preserve all eleven
per-element magnitudes and recognized magick identities. Snapshots also carry
the weapon and staff's stored charge queues. Clients restore those values and
authoritative clears without executing charge/release gameplay locally. Equipment
attachment slots, including robe accessories, apply immediately from the host
independently of health and death flags. Death drops held equipment; respawn and
Revive restore robe defaults. Scene and level changes preserve held equipment.
Before the first checkpoint, a shared restart reloads the initial scene and
restores robe defaults while retaining player ownership, profiles and unlocks.
Campaign completion advances on the host; connected clients load the advertised
level and scene while retaining their player slots. The end menu and chapter
cutscene remain unpresented. Rebuild
and restart hosts and clients together when changing protocol versions.

## More players, hot join and roles

The original game had four players and no joining mid-game. The rewrite keeps
four as the default and makes the rest opt-in (`research/multiplayer-scaling.md`):

```sh
# Up to 12 players, joining in progress locked (PvP), dungeon masters allowed:
./demo.sh --skip-build --server 127.0.0.1:17700 --lobby players=12,hotjoin=off,dm=on,invaders=on
./demo.sh --skip-build --join 127.0.0.1:17700 --name Watcher --role spectator
./demo.sh --skip-build --join 127.0.0.1:17700 --name GM --role dm
./demo.sh --skip-build --join 127.0.0.1:17700 --name Goblin --role invader
```

- In-game settings offer up to 16 players. A dedicated server can be configured
  up to 256 with `--lobby players=N`. Seating works at that size, but the host
  currently misses a 60 Hz tick above about 100 players, measured with idle
  wizards (`research/rewrite-state-assessment.md`).
- Players joining a started game spawn beside a living party member. With
  hot join locked (or a full lobby) they wait as spectators and join at the
  next level or shared restart.
- Spectators: Tab / shoulder buttons cycle the followed player, F9 toggles a
  free camera (arrows / left stick). F6 asks to join as a player; F7 steps out.
- Invaders pilot the nearest enemy with ordinary controls and take over
  another enemy when theirs dies.
- Dungeon masters: click to select a character, PageUp/PageDown choose a
  template, Insert spawns it at the cursor, P possesses, O releases, K smites,
  H heals, L toggles the hot-join lock, +/- change the player limit.
- Local co-op: in the lobby, Start on another pad seats it (A edits its slot,
  B leaves). In play, Start on a free pad joins beside the party; Back+Start
  leaves. Local players share one camera; remote players never move it.
- The menu's character page offers the same settings under the map. The
  Online page's "Join as" button chooses the role.
- Saves: any profile can play a save. Saved players whose profile is absent
  are kept and restored when that profile joins.

- LAN games: hosts and dedicated servers bound to a non-loopback address
  broadcast a small beacon on UDP port 17799 about once a second. The Online
  page lists them (name, level, players/limit, LAN) above the address
  history; select one and Join. Firewalls must allow UDP 17799 and the game
  port for discovery and play between computers.
- The IP-Address box accepts typing and Backspace; host names such as
  `localhost` resolve with IPv4 preferred.

## Security

Threat model, what was tested and what remains: `research/multiplayer-security.md`.

Every session is encrypted and authenticated with netcode.io, and the host checks
a joining computer really owns its address before it is seated or sent anything.
What that protects depends on the kind of game:

| | Open game (default) | Private game (passphrase) |
| --- | --- | --- |
| Who can join | Anyone who can reach the port | Only those who know the passphrase |
| Forged addresses, injected packets | Blocked | Blocked |
| Someone reading the network path | Can read and change the session | Cannot |
| Listed on the LAN | With its players | Without them, marked private |

```sh
# A private dedicated server. Secrets come from a file or the environment,
# because other local users can read command lines.
MAGICKA_PASSPHRASE='correct-horse-battery' ./demo.sh --skip-build --server 0.0.0.0:17700
MAGICKA_PASSPHRASE='correct-horse-battery' ./demo.sh --skip-build --join 203.0.113.7:17700 --name Alice
./demo.sh --skip-build --server 0.0.0.0:17700 --passphrase-file ~/magicka-passphrase

# Dungeon masters control the game. On anything but a game among friends, also
# require a credential (MAGICKA_DM_KEY or --dm-key-file, on both ends).
MAGICKA_DM_KEY='keys-to-the-dungeon' ./demo.sh --skip-build --server 0.0.0.0:17700 --lobby dm=on

# Dedicated server only: refuse addresses, or change how many connections one
# address may hold (default 16; a household or LAN party can share one address).
./demo.sh --skip-build --server 0.0.0.0:17700 --ban 203.0.113.9,198.51.100.4 --connections-per-address 4
```

In the menu, the Online page's Passphrase button sets it for both hosting and
joining. It is kept in memory only, and that box cannot type spaces. A passphrase
needs at least 8 characters. A
wrong one looks like a host that does not answer, because the host cannot tell it
from noise and does not reply to it.

The host and dungeon masters can remove a player from the pause menu's Players
page (with a confirmation). The player is told, its wizard leaves as if it had quit,
and its address is refused for the rest of the session, which also refuses anyone
else joining from that address. Connections from the same computer are removed but
not refused.

A connection that floods the host or sends what no client sends is removed and
its address refused for a minute. This does not apply to connections from the same
computer. On a server open to the internet, also rate-limit the UDP port in the
firewall: a flood of forged packets still costs the host time to discard.

## Dedicated server and two clients

Start the server in terminal 1:

```sh
./demo.sh --skip-build --server 127.0.0.1:17700 --demo-goblin --demo-goblin-order idle
```

Start Alice in terminal 2:

```sh
./demo.sh --skip-build --join 127.0.0.1:17700 --name Alice --character Wizard --robe-colour 0.2,0.4,0.8
```

Start Bob in terminal 3:

```sh
./demo.sh --skip-build --join 127.0.0.1:17700 --name Bob --character Wizard_Samurai --robe-colour 0.8,0.3,0.2
```

All three use the authored feature lab by default. The optional idle goblin is
a target for checking host damage and client NPC presentation. Omit
`--demo-goblin-order idle` for an attacking goblin, or omit both goblin options
for the map alone. Stop the dedicated server with Ctrl+C.

## Playable host instead

Stop the dedicated server before starting this alternative on the same port:

```sh
./demo.sh --skip-build --host 127.0.0.1:17700 --name Host --debug-menu --demo-goblin
```

Joining clients use the same commands above. A playable host reserves slot zero
for its local player; a dedicated server assigns all four slots to clients.

## Manual checks

- Hold left mouse to move each player independently. Both windows should show
  both players and host NPCs.
- Press F, then hold right mouse for Fire spray. Check that both clients show
  particles, the host applies damage, and releasing the button stops emission.
- Press F, then Shift+left-click to charge the weapon. Release the buttons,
  then Shift+left-click again to release its stored spell. Charge/release decisions
  belong to the host. Both windows should show the swinging wizard's IceBlade or
  ArcaneBlade trail and any self-shield or aura (protocol 14; rendered check
  pending); the automated storage checks verify both clients' queue values.
- Click nearby books/equipment. Only the server chooses the successful collector
  and changes equipment/unlocks. Try competing for one pickup.
- Open F1 and Player equipment to change name, robe and colour. Check both windows
  receive the accepted profile.
- Close one client, wait 10 seconds, then launch another joining client. The
  host should remove the departed wizard and reuse the vacant slot with a fresh
  profile, loadout and initial scene unlocks. The observing client should remove
  the old wizard and its equipment, then show the replacement even with the same
  robe. Rejoining starts a new player; it does not restore the departed identity.
  An empty dedicated server pauses gameplay.
- Stop the dedicated server while both clients are open. After 10 seconds
  without a fresh snapshot, each client should show a disconnect notice and stop
  sending controls. Restart the clients to join a newly started server.
- A host started with `--lobby respawn=on` lets a dead joining player press F5
  for the demo's server-owned debug respawn; without it the host ignores the
  request. Equipment and unlocked magicks should survive. A living client cannot
  use F5 to heal. F5 on a playable host restarts its checkpoint for the shared
  session.

## Current limits

Joining clients interpolate confirmed player, NPC and prop poses behind an
adaptive jitter buffer: 50 ms on a clean link, growing by three times the
measured arrival jitter up to 250 ms, and moving at most 2 ms per frame. With
prediction on (the default; `--no-predict` turns it off), a client also predicts
its own wizard's movement: on each host state it rolls back to that position and
replays the inputs the host has not applied (`PlayerState::last_input`), blending
small corrections and snapping large ones and lifetime changes. Only walking is
predicted. If the host keeps refusing to move the wizard (a wall, a cutscene, being
frozen), prediction pauses until it moves again. Spells and other players are not
predicted. Measured on the real wizard, mean display error
falls from 0.46 m to 0.11 m at a 100 ms round trip and from 0.89 m to 0.16 m
at 200 ms (`magicka-runtime/tests/prediction_measured.rs`). Checked in tests and
rendered QA; how it feels in play is for a person to judge. Gameplay ownership, health,
equipment and spell queues still apply immediately from the latest snapshot.
Death/respawn, replacement and scene changes snap across lifetime boundaries.
Packet gaps hold the newest confirmed pose. NPC clip times blend within compatible
clips; effect transforms still apply directly.
Both the dedicated and playable hosts are supported without assuming that their
publication ticks represent the same elapsed time.

NPC transforms, clip times and removal come from host snapshots. Clients create
particle emitters from host instance identities; particle randomness and emitted
history are presentation approximations, not synchronized gameplay. Late joins
start current emitters without replaying their historical particles. Very short
effects may be missed between snapshots. Each client runs an effect on its own
clock from the start, so bursts play, and moving effects and missiles coast
between host updates. Self-shields, auras, weapon trails and zap pulses on remote
wizards travel since protocol 14; NPC zap pulses do not. Client status materials, NPC
equipment effects, dialogue/script presentation, local input prediction and
identity-preserving reconnect remains incomplete. Clients show a disconnect notice after 10 seconds without a fresh
valid snapshot and require a new client process to rejoin. Silent clients lose ownership after
10 seconds without heartbeats or input; rejoining requires a new client
process. Inactive body/entity tombstones remain allocated until the server is
restarted. Retail-protocol compatibility is not supported.

The hidden-text allocation fix eliminated the 18 startup `slab_allocator`
diagnostics per client in the two-client verification. The wrapper now rejects
these diagnostics and checks each client's Fire particle history and NPC rendering.
