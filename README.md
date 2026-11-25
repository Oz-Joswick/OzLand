# 3D Walkabout

A first-person walk through a hand-designed valley, written in Rust and
rendered with [wgpu](https://wgpu.rs).

## The world

A 400 m valley ringed by snow-capped mountains. A lake fills the west,
fed by a river that winds down out of the north-east mountains. Paths link
everything into loops:

- **The village** (you start here): a cobbled square with a well, four
  houses facing it, a market corner and a fenced flower garden. It opens
  north-west towards the lake.
- **The pier**, reaching out into the lake from the village shore.
- **The bridge** over the river, then on into the **northern pine forest**.
- **The watchtower** on a bluff above the north-west shore, reached by a
  path that winds up around the hill. The best view of the valley.
- **The lakeshore walk** round the south and west shore, past the
  **fisher's rest**, and back up to the tower.
- **The stone circle**, ruins on the eastern hilltop, two pillars fallen.
- **The wildflower meadow** below the ruins, ending at a **viewpoint**
  bench looking back over the valley.

The terrain walks naturally. You follow slopes up and down, can wade into
shallow water but not deep water, and steep cliffs and the mountains stop
you. The layout lives in `src/map.rs`: the landmark positions, hills, path
and river routes are named constants and short lists of points, so moving or
adding a place is a small edit there.

```sh
cargo run            # debug build
cargo run --release  # smoother frame rate
cargo test           # model loading + collision tests
```

| Key | Action |
|---|---|
| WASD / arrow keys | Move |
| Mouse | Look around |
| Space | Jump |
| Shift | Sprint |
| V | Switch between first and third person |
| F | Teleport dash (ability) |
| Space (in the air) | Double jump, then spread/fold your wings (abilities) |
| Shift (gliding) | Rocket boost (ability) |
| Ctrl | Ground slam in the air, slide while running (abilities) |
| C / M / X / R | Spawn a clone / switch clone mode / swap places with it / dismiss clones (abilities) |
| E (hold) / P | Grapple hook / drop a bounce pad (abilities) |
| Right mouse (hold) / G | Hover, levitate or fly / toggle low gravity (abilities) |
| B / Z / T | Recall marker / rewind / bullet time or time stop (abilities) |
| Left mouse / 1-4 | Fire / choose weapon (abilities) |
| Esc | Pause and open the settings menu (Esc again or **Resume** to continue) |
| Q | Quit |

## Abilities

Skill points come from fifteen glowing **skill crystals** hidden around the
valley (some need abilities to reach) and from **experience**: distance
travelled and time in the air, each point costing a little more. Spend them
in the **Abilities** panel of the Esc menu. Seven trees have five levels
each; **Ascension** has no cap and makes everything you have stronger.

| Tree | Levels |
|---|---|
| Movement | Speed I, II, III; Teleport Dash (F); **Phase Shift** (6 charges, 25 m in any direction) |
| Air | High Jump; Double Jump; Ground Slam (Ctrl in the air); Gliding Flight (elytra-style: Space in mid-air spreads your wings, look down to dive and up to pull out, Shift fires a rocket); **Skyborne** (unlimited, faster rockets) |
| Clones | Stationary Clone (C); Mirror Clone (M switches mode); Twin Clones; Switch Places (X); **Legion** (8 clones that fire with you). R dismisses them |
| Parkour | Wall Slide; Wall Jump (Space against a wall); Wall Run (sprint along a wall in the air); Mantle (climb 2.5 m ledges); **Spider God** (endless wall runs, run up walls, 20 m mantle, double wall jumps) |
| Momentum | Slide (Ctrl while running); Bunny Hop (jump as you land); Air Strafe; Downhill Surf; **Unstoppable** (momentum to 10x that never fades while moving) |
| Gravity | Low Gravity (G); Hover (hold right mouse); Levitate; Deep Reserves; **True Flight** (fly anywhere at 2.5x speed, no limit) |
| Time | Recall Marker (B); Rewind (Z); Bullet Time (T); Long Rewind (refills air abilities); **Chronomancer** (10 s time stop, 4 s cooldown) |
| Gadgets | Bounce Pad (P); Grapple Hook (hold E); Super Pads; Long Hook; **Web Slinger** (150 m hook, 2.5x fling, 50 m/s pads) |
| Arsenal | Blaster (left mouse, hold to keep firing); Scatter Gun (2); Rocket Launcher (3: blasts shove you too, so rocket jump); Railgun (4: pierces every drone in line); **Annihilator** (triple damage and fire rate, huge blasts, clones fire with you) |
| Ascension | Endless: +6% speed, +8% jump, +5% rocket and pad power and weapon damage per level; extra boosts, dashes, air jumps and clones every few levels |

Twenty **practice drones** float around the valley (a few near the start)
for the weapons. Each one destroyed is worth experience, and they come back
after a while. Shots fly in world time, so Bullet Time slows them and Time
Stop freezes them mid-air, and a Ground Slam's shockwave hits nearby drones.

**The UI** is one model shown two ways. Paused (Esc), the *Abilities &
Weapons* tab lists every tree, grouped into abilities, weapons and mastery,
with progress dots and a dot where you can upgrade. Pick one to see its
levels, keys and Upgrade button; Arsenal adds a weapon stats table, and
Ascension totals what all your trees add up to. In play, the bottom-centre
hotbar has a tile per ability you can trigger (key, cooldown shutter,
charges, a glow while active), the weapon bar sits above it, movement
meters are bottom left, and skill points and experience are top right.

**Capstones.** Level 5 of every tree (in bold above) is a capstone: far
stronger than the rest, and hard to get. It costs 5 skill points instead of
one, and first you must complete that tree's **trial**: dash 150 times,
glide 2,000 m, switch places with clones 40 times, wall-run 500 m, reach
40 m/s on foot, hover or levitate for 120 s, rewind 30 times, ride the
grapple 1,000 m, or destroy 40 drones. Trial progress is saved and shown
under each capstone in the menu.

Progress is saved in `progress.cfg` next to the settings. **Unlock everything
(sandbox)** in the panel turns on every ability, with a slider for the
Ascension level. The code is in `src/abilities.rs` (trees, levels, progress),
`src/player.rs` (movement), `src/clones.rs`, `src/gadgets.rs` (pads, grapple,
marker), `src/weapons.rs`, `src/drones.rs` and `src/orbs.rs` (crystals).
## Items

Gear you unlock with **scrap** (5 from each drone destroyed, 15 from each
skill crystal) on the Esc menu's **Items** tab. Items form a tree like a tech
tree: most need **several** other items first, and each stacks on top of
your abilities.

| Tier | Cost | Items (and what each needs) |
|---|---|---|
| 1 | 10 | Spring Boots, Light Frame, Focus Lens, Capacitor, Thruster Pack, Grip Gloves |
| 2 | 25 | Kinetic Boots (Spring Boots + Light Frame), Targeting Visor (Focus Lens + Capacitor), Jetpack Core (Thruster Pack + Capacitor), Climbing Rig (Grip Gloves + Spring Boots), Momentum Coil (Light Frame + Grip Gloves) |
| 3 | 50 | Overdrive Engine (Kinetic Boots + Momentum Coil), Siege Module (Targeting Visor + Jetpack Core), Phase Battery (Jetpack Core + Climbing Rig + Kinetic Boots) |
| 4 | 120 | Singularity Core (all three tier-3 items): everything 25% stronger, and drones drop double scrap |

The tab draws the tree as a graph: owned items green, items you can unlock
next outlined in gold, the rest grey until their parts are owned. Click one
for its effect, its parts and the Unlock button. Items live in `src/items.rs`.
## Multiplayer

Two players over the network (or two windows on one computer). In the Esc
menu's **Multiplayer** tab, one player **hosts** (port 7777 by default; the
tab shows the address to give out) and the other **joins** with that
address. On the same Wi-Fi, use the LAN address shown in the host menu. For
players on different networks, the host must configure its router to forward
the selected **UDP** port (7777 by default) to the host computer, and allow
that port through its firewall. The joiner then enters the host's public IP
and port. This direct UDP setup cannot work behind carrier-grade NAT or a
router that does not allow port forwarding; use a VPN that connects both
players to the same virtual LAN in that case.
Or from the command line:

```sh
cargo run --release -- --host            # or --host 7778
cargo run --release -- --join 192.168.1.20:7777
```

Each player has their own colour: the host's robot is gold, the joiner's
teal, clones included, and a dot for each player shows top left. Positions,
facing, animation and wing pose are shared, and so is every shot: you see
each other's bolts, rockets and beams, and rocket blasts shove whoever is
near. The host runs the practice drones: all drone damage is decided there,
drone positions are sent to the joiner, and the joiner's kills are credited
back to them as experience. Each player keeps their own abilities and
progress. Clones, pads, the grapple rope, recall markers and time powers
are only seen by the player using them. When hosting, the game tries to open
the selected UDP port on the router with UPnP and displays the public address
if that succeeds. If automatic forwarding is unavailable, forward that UDP
port to the host PC manually and allow the game through the firewall. UPnP
cannot bypass carrier-grade NAT. The networking and router mapping code is in
`src/net.rs`.
### Fights

With two players connected, the game is a match of rounds:

1. **Explore (15 s):** both players are dropped at random spots far apart,
   with full health. Nobody can be hurt yet.
2. **Fight:** shots hurt the other player (bolts 6, scatter pellets 6 each,
   rockets up to 36, railgun 48, slam shockwave up to 24, all scaled by
   your weapon upgrades). Each player has 100 health; a bar over the other
   player's head shows theirs.
3. The first player to fall loses the round; the other scores. After a
   few seconds the next round starts.

The score, round and phase are at the top of the screen. **Reset all
players** (Multiplayer tab, click twice) resets every player's abilities,
items, experience and scrap and the score to 0-0; the old progress is kept
in `progress.backup.cfg`.

## Updates for your friends

Copies you give out update themselves. Publish a new version with:

```powershell
.\tools\release.ps1 -Repo yourname/walkabout   # first time; after that just .\tools\release.ps1
```

It bumps the version, builds, and uploads a GitHub release (needs the GitHub
CLI: install it from https://cli.github.com, run `gh auth login` once, and
create the repository on GitHub first). Friends download
`walkabout-full.zip` from the release page once, unzip it and run
`walkabout.exe`; after that, each start checks for a newer version, downloads
it in the background (the 60 MB of assets only when they changed) and offers
**Restart now** in the Multiplayer tab. Any other web host works too: run
`.\tools\release.ps1 -Url https://your.host/walkabout/` and upload the files
from `release\` there. Both players must run the same version to play
together; the game says so if they don't. The code is in `src/updater.rs`.
## Settings menu

Press **Esc** in game to pause and open the settings menu:

- **Controls**: mouse sensitivity, invert mouse Y, field of view.
- **Graphics**: V-Sync, 3D grass on/off with its draw distance and density,
  surface detail (parallax texture depth), view distance, FPS counter.
- **World**: brightness, sun height and direction (shadows move with it),
  and wind strength.

Changes take effect immediately. They're saved when you leave the menu or
quit, to `%APPDATA%\walkabout\settings.cfg` on Windows
(`~/.config/walkabout/settings.cfg` elsewhere), a plain `key = value` file.
**Reset to defaults** restores everything. The game also pauses on its own
if the window loses focus while you're playing.

The menu and HUD are drawn with [egui](https://github.com/emilk/egui)
(`src/ui.rs`); settings and the save file live in `src/settings.rs`.

Set `RUST_LOG=info` to log the GPU and each model as it loads. Model load
errors are always logged, and the scene starts without the model that failed.

Dev options (environment variables):

- `WALKABOUT_VIEW="x,z,yaw,pitch"` starts at a fixed spot (angles in
  degrees) without capturing the mouse.
- `WALKABOUT_SCREENSHOT=shot.png` saves the 30th frame to that file and
  quits. Combine it with `WALKABOUT_VIEW` to compare looks before and after a
  change.
- `WALKABOUT_GRASS=0` turns the 3D grass off.
- `WALKABOUT_MENU=1` starts with the settings menu open.

## Rendering

- **Terrain**: a 1 m heightfield mesh, splat-mapped with grass, dirt paths,
  rock on steep slopes, sand at the shoreline and cobbles, blended by each
  texture's height map. Rock cliffs use triplanar mapping; snow settles on
  high ledges. Every shader can look up the ground height under any point.
- **Water**: one sheet at the water level showing wherever the land dips
  below it, with rolling waves, sky reflections, depth-based colour and
  shoreline foam.
- **Sun shadows**: two 4096² cascades, a sharp one that follows the player
  and a coarse one covering the whole world, with soft (filtered) edges.
  Depth uses reverse-Z, which keeps distant hills from flickering.
- **Sky occlusion**: a depth map rendered from straight above darkens
  anything under a roof or tree canopy, so interiors and the ground under
  trees aren't lit by the open sky.
- **Lighting**: sun + hemispheric ambient (blue sky light from above, warm
  bounce from below) + Blinn-Phong specular, ACES tone mapping.
- **Normal maps**: on the procedural surfaces and on glTF models that have them.
- **3D grass**: real blades of geometry, about 270k drawn around the
  player, generated on the GPU (no vertex buffers). They sway in the wind,
  catch sun shadows, take their color from the ground texture under them
  and thin out with distance. A mask built from the scene keeps them off
  paths, floors and anything standing on the ground. Tune it with the
  `GrassField` settings in `map::build()`, or turn it off with
  `WALKABOUT_GRASS=0`.
- **Texture extrusion (parallax occlusion mapping)**: any material with a
  normal map can make its texture look raised out of the surface. The
  height map lives in the normal texture's alpha channel (white = top), and
  `MaterialData::parallax_depth` sets how deep it goes, in UV units. For
  example, 0.02 on a texture that repeats every 2 m gives 4 cm of relief.
  The cobbles, bricks, stone walls, tiles, crate frames and roof tiles all
  use it. It changes how a surface looks, not its outline or collision.
- **Procedural sky**: gradient, sun disc and glow, drifting clouds. Distance
  fog uses the sky color so far objects melt into the horizon.
- **No flat colors**: built-in surfaces get generated textures (grass, stone
  wall, brick, floor tiles, sandstone, wooden crates, planks, roof
  shingles, cobblestones). See `src/textures.rs`. Untextured model
  materials get world-space noise, every object darkens slightly near the
  ground, and each placed copy of a model can have its own tint.

## How it is put together

| Module | Responsibility |
|---|---|
| `src/main.rs` | Window and event loop (winit), frame update |
| `src/renderer.rs` | wgpu device, pipelines (shadow, sky, mesh), depth/MSAA/shadow targets, uniforms, draw batches |
| `src/shader.wgsl` | MVP transform, lighting, shadows, sky occlusion, normal mapping, sky, fog, tone mapping |
| `src/mesh.rs` | `Vertex` / `MeshData` (vertex + index buffers on the CPU), box/quad/ground builders, tangents |
| `src/textures.rs` | Procedural albedo + normal-map textures for the built-in surfaces |
| `src/model.rs` | Loading `.glb` / `.gltf` / `.obj` files into meshes + materials |
| `src/camera.rs` | First-person camera, view and perspective projection matrices |
| `src/input.rs` | Keyboard/mouse state from window events |
| `src/settings.rs` | User settings, their ranges, and the save file |
| `src/ui.rs` | Escape settings menu and HUD (crosshair, FPS) with egui |
| `src/body.rs` | The player's animated body: picks and cross-fades Idle / Walking / Running / Jump from how you move |
| `src/animation.rs` | Skeletal animation for glTF characters: skins, clips, blending, CPU skinning |
| `src/player.rs` | Walking, jumping, collision response |
| `src/collision.rs` | Axis-aligned bounding boxes |
| `src/map.rs` | The world design: terrain shape, lake, river, paths, landmarks, forests, ground textures |
| `src/terrain.rs` | Heightfield terrain, exact height queries, distance fields, path splines |
| `src/noise.rs` | World-space noise for shaping and placing things |
| `src/scene.rs` | Building blocks: placing boxes and models on the terrain, houses, roofs, fences, benches |

Every object is a triangle mesh with its own vertex and index buffers.
Each copy of a model in the scene is drawn as an instance with its own
model matrix, so a model drawn 40 times costs one draw call per part.

## Adding a model to the scene

1. Put the file under `assets/`, e.g. `assets/models/statue.glb`.
   Supported formats:
   - **glTF 2.0** (`.glb`, or `.gltf` with its `.bin` and textures next to it).
     The loader reads the node hierarchy, normals, tangents, UVs, vertex
     colors, base color factor and texture, normal map, roughness (as
     specular), and alpha-mask cut-outs.
   - **OBJ** (`.obj`, with an optional `.mtl` next to it). The loader reads
     the diffuse color (`Kd`) and texture (`map_Kd`).

   Normals are computed (flat shaded) if the file has none.
2. Place it in `build()` in `src/map.rs` (or in one of the landmark
   functions, like `village()`):

   ```rust
   w.place(
       "models/statue.glb",        // path relative to assets/
       Placement::at(5.0, -8.0)    // x, z: stands on the terrain there
           .height(2.5)            // optional: scale it to be 2.5 m tall
           .yaw(45.0)              // optional: rotate, in degrees
           .tint(Vec4::new(1.0, 0.9, 0.8, 1.0)) // optional: color variation
           .collider(Collider::Bounds),
   );
   ```

   The model's bounding box is set down so its bottom center sits at the
   given point, so it doesn't matter where the file's origin is or how big
   it was authored. Call `place` as many times as you like. The file is only
   loaded once and every copy is drawn instanced.

   Collider options:
   - `Collider::Bounds` (the default): the whole bounding box is solid.
   - `Collider::Box(size)`: a box of `size` meters around the model's
     origin. Use it for a trunk or post when the bounding box is much bigger
     than the solid part.
   - `Collider::None`: you can walk through it.

Material features that are not supported yet: metallic/roughness,
occlusion and emissive *maps*; blended transparency (it is drawn as a
cut-out); `KHR_texture_transform`; animations and skinning. Draco-compressed
files won't load.

To add a built-in surface instead, add a variant to `Surface` in
`src/textures.rs` with a pattern function (its `height` output drives both
the normal map and the parallax relief), give it shading settings and a
parallax depth in `surface_material` in `src/scene.rs`, and build boxes with
it in `src/map.rs`.

## Sample models

- `assets/models/pine_tree.glb` and `assets/models/rock.obj` are generated by
  `cargo run --example generate_sample_models`.
- `assets/polyhaven/` holds photo-scanned CC0 assets from [Poly Haven](https://polyhaven.com):
  ground and building textures (dirt, rock, sand, cobbles, brick, stone, planks,
  roof tiles, sandstone) and models (boulders, mossy rocks, ferns, shrubs, stumps,
  a fallen trunk). Fetch or refresh them with
  `powershell -ExecutionPolicy Bypass -File tools/fetch_assets.ps1`. Without them the
  game falls back to its generated textures and low-poly rocks. Poly Haven's trees
  aren't used: they're film-quality scans of 7-17 million triangles each.
- `assets/characters/robot/RobotExpressive.glb`: the player character, by Tomás Laulhé
  (Quaternius), CC0, via the [three.js examples](https://github.com/mrdoob/three.js/tree/dev/examples/models/gltf/RobotExpressive).
- `assets/trees/`: [flo-bit's low poly nature pack](https://github.com/flo-bit/low-poly-asset-packs), CC0.
- `assets/models/lantern/Lantern.glb` comes from the
  [Khronos glTF Sample Assets](https://github.com/KhronosGroup/glTF-Sample-Assets/tree/main/Models/Lantern),
  © 2017 Microsoft, CC0 (see the `LICENSE.md` next to it).
