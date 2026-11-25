//! Character abilities: seven skill trees of five levels, plus Ascension,
//! which has no cap and keeps stacking. Skill points come from glowing
//! crystals hidden around the valley (see `orbs.rs`) and from experience:
//! distance travelled and time in the air. Progress is saved next to the
//! settings file.

use crate::settings::Settings;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tree {
    Movement,
    Air,
    Clone,
    Parkour,
    Momentum,
    Gravity,
    Time,
    Gadgets,
    Arsenal,
    Ascension,
}

pub struct Level {
    pub name: &'static str,
    /// The control that uses it ("" for passive upgrades).
    pub key: &'static str,
    pub description: &'static str,
}

const fn level(name: &'static str, key: &'static str, description: &'static str) -> Level {
    Level { name, key, description }
}

/// Levels in each regular tree (Ascension has no cap).
pub const MAX_LEVEL: u32 = 5;

const MOVEMENT: [Level; 5] = [
    level("Speed I", "", "Run 25% faster."),
    level("Speed II", "", "Run 50% faster."),
    level("Speed III", "", "Run 80% faster."),
    level("Teleport Dash", "F", "Blink up to 9 m ahead. Recharges in 2 s."),
    level("Phase Shift", "F", "CAPSTONE. Six dash charges recharging every 0.3 s, and each blink goes 25 m wherever you look: up into the sky, down off cliffs, across the lake."),
];

const AIR: [Level; 5] = [
    level("High Jump", "Space", "Jump 60% higher."),
    level("Double Jump", "Space", "Jump again in mid-air."),
    level("Ground Slam", "Ctrl", "In mid-air: slam down, shaking the ground and hitting drones nearby."),
    level(
        "Gliding Flight",
        "Space",
        "In mid-air (after the double jump) spread your wings; again to fold them. Look down to dive for speed, up to pull out. Shift fires a rocket boost (3 per flight).",
    ),
    level("Skyborne", "Shift", "CAPSTONE. Unlimited rocket boosts, and rockets twice as fast. You never have to land."),
];

const CLONE: [Level; 5] = [
    level("Stationary Clone", "C", "Leave a clone of yourself standing here. R dismisses clones."),
    level("Mirror Clone", "M", "Switch to clones that mirror your moves like a reflection."),
    level("Twin Clones", "C", "Keep two clones at once."),
    level("Switch Places", "X", "Swap places with your newest clone."),
    level("Legion", "C", "CAPSTONE. An army of eight clones, and every one of them fires whenever you do."),
];

const PARKOUR: [Level; 5] = [
    level("Wall Slide", "", "Falling against a wall, you slide down it slowly."),
    level("Wall Jump", "Space", "Against a wall in mid-air: kick up and away from it, as often as you like."),
    level("Wall Run", "Shift", "Sprint along a wall in mid-air to run on it for 1.5 s."),
    level("Mantle", "", "Grab and climb onto ledges up to 2.5 m above your feet."),
    level("Spider God", "Shift", "CAPSTONE. Wall runs never end, sprint straight up any wall, mantle ledges up to 20 m, and wall jumps launch twice as hard."),
];

const MOMENTUM: [Level; 5] = [
    level("Slide", "Ctrl", "While running: a fast slide."),
    level("Bunny Hop", "Space", "Jump just as you land to keep your speed and add 12%."),
    level("Air Strafe", "", "Full control of your direction in mid-air."),
    level("Downhill Surf", "", "Slides speed up going downhill and last as long as the slope."),
    level("Unstoppable", "Shift", "CAPSTONE. Momentum up to 10x and it never fades while you keep moving. Sprinting alone builds to 3x."),
];

const GRAVITY: [Level; 5] = [
    level("Low Gravity", "G", "Toggle moon gravity."),
    level("Hover", "RMB", "Hold in mid-air to hang in place (2 s, refills on landing)."),
    level("Levitate", "RMB", "Holding now rises instead."),
    level("Deep Reserves", "", "6 s of hover and levitation, slowly recharging even in the air."),
    level("True Flight", "RMB", "CAPSTONE. Hold to fly freely wherever you look, 2.5x faster than you run, Shift for more. No energy, no limit."),
];

const TIME: [Level; 5] = [
    level("Recall Marker", "B", "Drop a marker; press again to return to it."),
    level("Rewind", "Z", "Jump back to where you were 3 s ago."),
    level("Bullet Time", "T", "For 5 s the world runs at 30% speed while you move at 70%."),
    level("Long Rewind", "Z", "Rewind 6 s. Recall and rewind refill your jumps, boosts and hover."),
    level("Chronomancer", "T", "CAPSTONE. Stop the world entirely for 10 s (shots, drones, grass and water), ready again 4 s later."),
];

const GADGETS: [Level; 5] = [
    level("Bounce Pad", "P", "Drop a pad that launches you skyward."),
    level("Grapple Hook", "E", "Hold to hook what you're aiming at (35 m) and get pulled to it."),
    level("Super Pads", "P", "Three pads at once, launching much higher."),
    level("Long Hook", "E", "Hook reaches 70 m and pulls twice as hard."),
    level("Web Slinger", "E", "CAPSTONE. 150 m hook with a huge pull; letting go flings you 2.5x and refills your jumps and boosts; pads launch at 50 m/s."),
];

const ARSENAL: [Level; 5] = [
    level("Blaster", "1", "Rapid energy bolts: hold the left mouse button to keep firing. Practice drones float around the valley."),
    level("Scatter Gun", "2", "A spread of pellets, deadly up close."),
    level("Rocket Launcher", "3", "Rockets explode and shove everything nearby, you included: aim at your feet to rocket jump."),
    level("Railgun", "4", "An instant beam that pierces every drone in its line."),
    level("Annihilator", "", "CAPSTONE. Triple damage, triple fire rate, 16-pellet scatter, 10 m rocket blasts, and your clones fire with you."),
];

/// Ascension's one level, repeated forever.
const ASCENSION: [Level; 1] = [level(
    "Ascend",
    "",
    "+6% speed, +8% jump height, +5% rocket, pad and weapon power per level. Every 2 levels +1 boost, every 3 +1 dash, every 4 +1 air jump, every 5 +1 clone.",
)];

/// Level 5 of every regular tree: a capstone that costs this many points.
pub const CAPSTONE_COST: u32 = 5;

/// Running totals kept for the capstone trials.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stat {
    Dashes,
    GlideDistance,
    Swaps,
    WallRunDistance,
    FootSpeed,
    HoverTime,
    Rewinds,
    GrappleDistance,
    Kills,
}

impl Stat {
    pub const ALL: [Stat; 9] = [
        Stat::Dashes,
        Stat::GlideDistance,
        Stat::Swaps,
        Stat::WallRunDistance,
        Stat::FootSpeed,
        Stat::HoverTime,
        Stat::Rewinds,
        Stat::GrappleDistance,
        Stat::Kills,
    ];

    fn index(self) -> usize {
        Stat::ALL.iter().position(|s| *s == self).expect("listed")
    }

    fn key(self) -> &'static str {
        match self {
            Stat::Dashes => "trial_dashes",
            Stat::GlideDistance => "trial_glide",
            Stat::Swaps => "trial_swaps",
            Stat::WallRunDistance => "trial_wall_run",
            Stat::FootSpeed => "trial_foot_speed",
            Stat::HoverTime => "trial_hover",
            Stat::Rewinds => "trial_rewinds",
            Stat::GrappleDistance => "trial_grapple",
            Stat::Kills => "trial_kills",
        }
    }
}

/// What a capstone asks of you before it can be bought.
pub struct Trial {
    pub stat: Stat,
    pub goal: f32,
    pub description: &'static str,
    /// Unit shown with the progress ("m", "s", "m/s" or "").
    pub unit: &'static str,
}

/// How the trees are grouped in the menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Abilities,
    Weapons,
    Mastery,
}

impl Group {
    pub const ALL: [Group; 3] = [Group::Abilities, Group::Weapons, Group::Mastery];

    pub fn name(self) -> &'static str {
        match self {
            Group::Abilities => "Abilities",
            Group::Weapons => "Weapons",
            Group::Mastery => "Mastery",
        }
    }
}
impl Tree {
    pub const ALL: [Tree; 10] = [
        Tree::Movement,
        Tree::Air,
        Tree::Clone,
        Tree::Parkour,
        Tree::Momentum,
        Tree::Gravity,
        Tree::Time,
        Tree::Gadgets,
        Tree::Arsenal,
        Tree::Ascension,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Tree::Movement => "Movement",
            Tree::Air => "Air",
            Tree::Clone => "Clones",
            Tree::Parkour => "Parkour",
            Tree::Momentum => "Momentum",
            Tree::Gravity => "Gravity",
            Tree::Time => "Time",
            Tree::Gadgets => "Gadgets",
            Tree::Arsenal => "Arsenal",
            Tree::Ascension => "Ascension",
        }
    }

    pub fn group(self) -> Group {
        match self {
            Tree::Arsenal => Group::Weapons,
            Tree::Ascension => Group::Mastery,
            _ => Group::Abilities,
        }
    }

    /// One line on what the tree is about.
    pub fn tagline(self) -> &'static str {
        match self {
            Tree::Movement => "Run faster, then blink.",
            Tree::Air => "Jump higher, slam down, take wing.",
            Tree::Clone => "Copies of yourself.",
            Tree::Parkour => "Walls are paths.",
            Tree::Momentum => "Speed that builds on itself.",
            Tree::Gravity => "Hover, rise, fly.",
            Tree::Time => "Bend the clock.",
            Tree::Gadgets => "Pads and a grapple hook.",
            Tree::Arsenal => "Four weapons, unlocked in turn.",
            Tree::Ascension => "Everything you have, stronger. No limit.",
        }
    }

    /// The trial guarding this tree's capstone (level 5).
    pub fn trial(self) -> Option<Trial> {
        let t = |stat, goal, description, unit| Some(Trial { stat, goal, description, unit });
        match self {
            Tree::Movement => t(Stat::Dashes, 150.0, "Dash 150 times", ""),
            Tree::Air => t(Stat::GlideDistance, 2000.0, "Glide 2,000 m", "m"),
            Tree::Clone => t(Stat::Swaps, 40.0, "Switch places with a clone 40 times", ""),
            Tree::Parkour => t(Stat::WallRunDistance, 500.0, "Wall-run 500 m", "m"),
            Tree::Momentum => t(Stat::FootSpeed, 40.0, "Reach 40 m/s on foot", "m/s"),
            Tree::Gravity => t(Stat::HoverTime, 120.0, "Hover or levitate for 120 s in total", "s"),
            Tree::Time => t(Stat::Rewinds, 30.0, "Rewind 30 times", ""),
            Tree::Gadgets => t(Stat::GrappleDistance, 1000.0, "Ride the grapple 1,000 m", "m"),
            Tree::Arsenal => t(Stat::Kills, 40.0, "Destroy 40 drones", ""),
            Tree::Ascension => None,
        }
    }

    /// Points to buy level (capstones cost more).
    pub fn cost(self, level: u32) -> u32 {
        if self.max_level() == Some(level) { CAPSTONE_COST } else { 1 }
    }

    /// Key used in the save file.
    fn key(self) -> &'static str {
        match self {
            Tree::Clone => "clone",
            Tree::Movement => "movement",
            Tree::Air => "air",
            Tree::Parkour => "parkour",
            Tree::Momentum => "momentum",
            Tree::Gravity => "gravity",
            Tree::Time => "time",
            Tree::Gadgets => "gadgets",
            Tree::Arsenal => "arsenal",
            Tree::Ascension => "ascension",
        }
    }

    pub fn levels(self) -> &'static [Level] {
        match self {
            Tree::Movement => &MOVEMENT,
            Tree::Air => &AIR,
            Tree::Clone => &CLONE,
            Tree::Parkour => &PARKOUR,
            Tree::Momentum => &MOMENTUM,
            Tree::Gravity => &GRAVITY,
            Tree::Time => &TIME,
            Tree::Gadgets => &GADGETS,
            Tree::Arsenal => &ARSENAL,
            Tree::Ascension => &ASCENSION,
        }
    }

    /// The highest level, or `None` for Ascension (endless).
    pub fn max_level(self) -> Option<u32> {
        (self != Tree::Ascension).then_some(MAX_LEVEL)
    }

    /// What unlocking `level` (1-based) gives.
    pub fn level_info(self, level: u32) -> &'static Level {
        let levels = self.levels();
        &levels[(level.max(1) as usize - 1).min(levels.len() - 1)]
    }

    pub fn index(self) -> usize {
        Tree::ALL.iter().position(|t| *t == self).expect("listed")
    }
}

pub const TREES: usize = Tree::ALL.len();

/// Rocket boosts that never run out.
pub const UNLIMITED: u32 = 9999;

/// What the character can currently do, derived from the tree levels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Moveset {
    // Movement
    /// Multiplier on walking/running speed.
    pub speed: f32,
    pub dash_charges: u32,
    /// Seconds to recharge one dash.
    pub dash_recharge: f32,
    /// How far a dash goes, m.
    pub dash_distance: f32,
    /// Dashes follow your view up and down too (Phase Shift).
    pub dash_any_direction: bool,
    // Air
    /// Multiplier on jump velocity.
    pub jump: f32,
    /// Extra jumps allowed in mid-air.
    pub air_jumps: u32,
    pub slam: bool,
    pub glide: bool,
    /// Rocket boosts per flight ([`UNLIMITED`] with Skyborne).
    pub boosts: u32,
    /// Speed a rocket boost pushes you towards, in m/s.
    pub rocket_speed: f32,
    // Clones
    pub max_clones: usize,
    pub mirror_clones: bool,
    pub switch_places: bool,
    /// Clones fire whenever you do (Legion, Annihilator).
    pub clones_fire: bool,
    // Parkour
    pub wall_slide: bool,
    pub wall_jump: bool,
    /// Seconds you can run along a wall (infinite with Spider).
    pub wall_run: f32,
    /// How high a ledge you can climb onto (0 = only normal steps).
    pub mantle: f32,
    pub wall_climb: bool,
    /// Multiplier on wall jumps.
    pub wall_jump_power: f32,
    // Momentum
    pub slide: bool,
    pub bunny_hop: bool,
    pub air_strafe: bool,
    pub surf: bool,
    pub run_up: bool,
    /// Highest speed multiplier momentum can build.
    pub momentum_cap: f32,
    /// What sprinting alone builds momentum up to (Run-Up).
    pub run_up_cap: f32,
    /// Momentum doesn't fade while you keep moving (Unstoppable).
    pub momentum_hold: bool,
    // Gravity
    pub low_gravity: bool,
    /// Seconds of hover/levitation per flight.
    pub hover_time: f32,
    pub levitate: bool,
    /// Hover energy recharges in the air too.
    pub hover_recharge: bool,
    pub free_flight: bool,
    /// Multiplier on free-flight speed.
    pub fly_speed: f32,
    // Time
    pub recall: bool,
    /// How far back Rewind goes (0 = no rewind).
    pub rewind: f32,
    pub bullet_time: bool,
    /// Recall and rewind refill jumps, boosts and hover.
    pub time_refill: bool,
    pub time_stop: bool,
    /// How long Time Stop lasts, and the wait after a time power ends.
    pub time_stop_length: f32,
    pub time_cooldown: f32,
    // Gadgets
    pub pads: usize,
    /// Upward speed a bounce pad gives, m/s.
    pub pad_launch: f32,
    /// Grapple range in meters (0 = no grapple).
    pub grapple_range: f32,
    /// Grapple pull acceleration, m/s².
    pub grapple_pull: f32,
    pub slingshot: bool,
    /// How much letting go of the hook speeds you up (Slingshot).
    pub fling: f32,
    // Arsenal
    /// How many weapons are unlocked (Blaster, Scatter, Rockets, Railgun).
    pub weapons: u32,
    pub weapon_damage: f32,
    /// Multiplier on fire rate.
    pub fire_rate: f32,
    /// Rocket blast radius, m.
    pub blast_radius: f32,
    /// Scatter Gun pellets.
    pub pellets: u32,
    // Items
    /// Multiplier on scrap from drones (Singularity Core).
    pub scrap_multiplier: u32,
}

impl Default for Moveset {
    /// No abilities: the plain character.
    fn default() -> Self {
        Self {
            speed: 1.0,
            dash_charges: 0,
            dash_recharge: 2.0,
            dash_distance: 9.0,
            dash_any_direction: false,
            jump: 1.0,
            air_jumps: 0,
            slam: false,
            glide: false,
            boosts: 0,
            rocket_speed: 30.0,
            max_clones: 0,
            mirror_clones: false,
            switch_places: false,
            clones_fire: false,
            wall_slide: false,
            wall_jump: false,
            wall_run: 0.0,
            mantle: 0.0,
            wall_climb: false,
            wall_jump_power: 1.0,
            slide: false,
            bunny_hop: false,
            air_strafe: false,
            surf: false,
            run_up: false,
            momentum_cap: 1.0,
            run_up_cap: 2.0,
            momentum_hold: false,
            low_gravity: false,
            hover_time: 0.0,
            levitate: false,
            hover_recharge: false,
            free_flight: false,
            fly_speed: 1.0,
            recall: false,
            rewind: 0.0,
            bullet_time: false,
            time_refill: false,
            time_stop: false,
            time_stop_length: 10.0,
            time_cooldown: 8.0,
            pads: 0,
            pad_launch: 18.0,
            grapple_range: 0.0,
            grapple_pull: 45.0,
            slingshot: false,
            fling: 1.0,
            weapons: 0,
            weapon_damage: 1.0,
            fire_rate: 1.0,
            blast_radius: 4.5,
            pellets: 8,
            scrap_multiplier: 1,
        }
    }
}

impl Moveset {
    pub fn from_levels(levels: [u32; TREES]) -> Self {
        let at = |t: Tree| levels[t.index()];
        let mut m = Self::default();

        let movement = at(Tree::Movement);
        m.speed = [1.0, 1.25, 1.5, 1.8][movement.min(3) as usize];
        if movement >= 4 {
            m.dash_charges = 1;
        }
        if movement >= 5 {
            // Phase Shift.
            m.dash_charges = 6;
            m.dash_recharge = 0.3;
            m.dash_distance = 25.0;
            m.dash_any_direction = true;
        }

        let air = at(Tree::Air);
        if air >= 1 {
            m.jump = 1.6f32.sqrt(); // 60% higher
        }
        if air >= 2 {
            m.air_jumps = 1;
        }
        m.slam = air >= 3;
        if air >= 4 {
            m.glide = true;
            m.boosts = 3;
        }
        if air >= 5 {
            // Skyborne.
            m.boosts = UNLIMITED;
            m.rocket_speed = 55.0;
        }

        let clone = at(Tree::Clone);
        m.max_clones = [0, 1, 1, 2, 2, crate::clones::MAX_CLONES][clone.min(5) as usize];
        m.clones_fire = clone >= 5; // Legion
        m.mirror_clones = clone >= 2;
        m.switch_places = clone >= 4;

        let parkour = at(Tree::Parkour);
        m.wall_slide = parkour >= 1;
        m.wall_jump = parkour >= 2;
        if parkour >= 3 {
            m.wall_run = 1.5;
        }
        if parkour >= 4 {
            m.mantle = 2.5;
        }
        if parkour >= 5 {
            // Spider God.
            m.wall_run = f32::INFINITY;
            m.wall_climb = true;
            m.mantle = 20.0;
            m.wall_jump_power = 2.0;
        }

        let momentum = at(Tree::Momentum);
        m.slide = momentum >= 1;
        m.bunny_hop = momentum >= 2;
        m.air_strafe = momentum >= 3;
        m.surf = momentum >= 4;
        m.momentum_cap = match momentum {
            0 => 1.0,
            1 => 1.6,
            2..=4 => 3.0,
            _ => 10.0,
        };
        if momentum >= 5 {
            // Unstoppable.
            m.run_up = true;
            m.run_up_cap = 3.0;
            m.momentum_hold = true;
        }

        let gravity = at(Tree::Gravity);
        m.low_gravity = gravity >= 1;
        if gravity >= 2 {
            m.hover_time = 2.0;
        }
        m.levitate = gravity >= 3;
        if gravity >= 4 {
            m.hover_time = 6.0;
            m.hover_recharge = true;
        }
        if gravity >= 5 {
            // True Flight.
            m.free_flight = true;
            m.fly_speed = 2.5;
        }

        let time = at(Tree::Time);
        m.recall = time >= 1;
        if time >= 2 {
            m.rewind = 3.0;
        }
        m.bullet_time = time >= 3;
        if time >= 4 {
            m.rewind = 6.0;
            m.time_refill = true;
        }
        if time >= 5 {
            // Chronomancer.
            m.time_stop = true;
            m.time_stop_length = 10.0;
            m.time_cooldown = 4.0;
        }

        let gadgets = at(Tree::Gadgets);
        if gadgets >= 1 {
            m.pads = 1;
        }
        if gadgets >= 2 {
            m.grapple_range = 35.0;
        }
        if gadgets >= 3 {
            m.pads = 3;
            m.pad_launch = 28.0;
        }
        if gadgets >= 4 {
            m.grapple_range = 70.0;
            m.grapple_pull = 90.0;
        }
        if gadgets >= 5 {
            // Web Slinger.
            m.slingshot = true;
            m.fling = 2.5;
            m.grapple_range = 150.0;
            m.grapple_pull = 160.0;
            m.pad_launch = 50.0;
        }

        let arsenal = at(Tree::Arsenal);
        m.weapons = arsenal.min(4);
        if arsenal >= 5 {
            // Annihilator.
            m.clones_fire = true;
            m.fire_rate = 3.0;
            m.weapon_damage = 3.0;
            m.blast_radius = 10.0;
            m.pellets = 16;
        }

        // Ascension: everything you have, stronger, without end.
        let a = at(Tree::Ascension);
        if a > 0 {
            let f = a as f32;
            m.speed *= 1.0 + 0.06 * f;
            m.jump *= (1.0 + 0.08 * f).sqrt();
            m.rocket_speed *= 1.0 + 0.05 * f;
            m.pad_launch *= 1.0 + 0.05 * f;
            m.weapon_damage *= 1.0 + 0.05 * f;
            if m.boosts < UNLIMITED {
                m.boosts += a / 2;
            }
            m.air_jumps += a / 4;
            if m.dash_charges > 0 {
                m.dash_charges += a / 3;
            }
            if m.max_clones > 0 {
                m.max_clones = (m.max_clones + (a / 5) as usize).min(crate::clones::MAX_CLONES);
            }
            if m.grapple_range > 0.0 {
                m.grapple_range += 5.0 * f;
            }
            if m.hover_time > 0.0 {
                m.hover_time *= 1.0 + 0.1 * f;
            }
        }
        m
    }
}

/// Skill points, tree levels, experience and which crystals were found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Progress {
    pub points: u32,
    levels: [u32; TREES],
    /// Indices of crystals already collected.
    pub found: Vec<usize>,
    /// Experience towards the next point.
    pub xp: f32,
    /// Points earned from experience so far (each costs a little more).
    pub xp_points: u32,
    /// Totals for the capstone trials (see [`Stat`]).
    stats: [f32; Stat::ALL.len()],
    /// Currency for items (from drones and crystals).
    pub scrap: u32,
    /// Items owned (indices into `items::ITEMS`).
    items: Vec<usize>,
}

impl Progress {
    pub fn level(&self, tree: Tree) -> u32 {
        self.levels[tree.index()]
    }

    pub fn levels(&self) -> [u32; TREES] {
        self.levels
    }

    pub fn items(&self) -> &[usize] {
        &self.items
    }

    pub fn has_item(&self, item: usize) -> bool {
        self.items.contains(&item)
    }

    /// What's stopping you from unlocking `item`, if anything.
    pub fn item_blocker(&self, item: usize) -> Option<String> {
        let it = crate::items::ITEMS.get(item)?;
        if self.has_item(item) {
            return Some("Owned".into());
        }
        let missing: Vec<&str> = it
            .needs
            .iter()
            .filter(|n| !self.has_item(**n))
            .map(|n| crate::items::ITEMS[*n].name)
            .collect();
        if !missing.is_empty() {
            return Some(format!("Needs {}", missing.join(" + ")));
        }
        (self.scrap < it.cost).then(|| format!("Needs {} scrap", it.cost))
    }

    /// Spends scrap on `item`; false if it can't be unlocked yet.
    pub fn buy_item(&mut self, item: usize) -> bool {
        if self.item_blocker(item).is_some() {
            return false;
        }
        self.scrap -= crate::items::ITEMS[item].cost;
        self.items.push(item);
        true
    }

    /// What's stopping the next level of `tree`, if anything.
    pub fn blocker(&self, tree: Tree) -> Option<String> {
        let next = self.level(tree) + 1;
        if tree.max_level().is_some_and(|max| next > max) {
            return Some("Fully upgraded".into());
        }
        if tree.max_level() == Some(next)
            && let Some(trial) = tree.trial()
            && !self.trial_done(&trial)
        {
            return Some(format!("Trial: {}", trial.description));
        }
        let cost = tree.cost(next);
        (self.points < cost).then(|| format!("Needs {cost} point{}", if cost == 1 { "" } else { "s" }))
    }

    pub fn can_upgrade(&self, tree: Tree) -> bool {
        self.blocker(tree).is_none()
    }

    /// Progress on a trial, 0..1.
    pub fn trial_progress(&self, trial: &Trial) -> f32 {
        (self.stat(trial.stat) / trial.goal).clamp(0.0, 1.0)
    }

    pub fn trial_done(&self, trial: &Trial) -> bool {
        self.stat(trial.stat) >= trial.goal
    }

    pub fn stat(&self, stat: Stat) -> f32 {
        self.stats[stat.index()]
    }

    /// Adds to a trial total. Returns the tree whose trial this completes.
    pub fn record(&mut self, stat: Stat, amount: f32) -> Option<Tree> {
        self.set_stat(stat, self.stat(stat) + amount.max(0.0))
    }

    /// Raises a best-so-far trial value (top speed).
    pub fn record_best(&mut self, stat: Stat, value: f32) -> Option<Tree> {
        self.set_stat(stat, self.stat(stat).max(value))
    }

    fn set_stat(&mut self, stat: Stat, value: f32) -> Option<Tree> {
        let before = self.stat(stat);
        self.stats[stat.index()] = value;
        Tree::ALL.into_iter().find(|t| {
            t.trial()
                .is_some_and(|trial| trial.stat == stat && before < trial.goal && value >= trial.goal)
        })
    }

    /// Spends points on `tree`; returns the newly unlocked level.
    pub fn upgrade(&mut self, tree: Tree) -> Option<&'static Level> {
        if !self.can_upgrade(tree) {
            return None;
        }
        self.points -= tree.cost(self.level(tree) + 1);
        self.levels[tree.index()] += 1;
        Some(tree.level_info(self.levels[tree.index()]))
    }

    /// Records a found crystal (once) and grants its point.
    pub fn collect(&mut self, crystal: usize) -> bool {
        if self.found.contains(&crystal) {
            return false;
        }
        self.found.push(crystal);
        self.points += 1;
        true
    }

    /// Experience needed for the next point.
    pub fn xp_needed(&self) -> f32 {
        60.0 + 15.0 * self.xp_points as f32
    }

    /// Adds experience; returns true if it earned a skill point.
    pub fn add_xp(&mut self, amount: f32) -> bool {
        self.xp += amount.max(0.0);
        if self.xp >= self.xp_needed() {
            self.xp -= self.xp_needed();
            self.xp_points += 1;
            self.points += 1;
            return true;
        }
        false
    }

    fn path() -> std::path::PathBuf {
        Settings::file_path().with_file_name("progress.cfg")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .map(|t| Self::parse(&t))
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let path = Self::path();
        let result = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| std::fs::write(&path, self.serialize()));
        if let Err(e) = result {
            log::warn!("could not save progress to {}: {e}", path.display());
        }
    }

    pub fn serialize(&self) -> String {
        let mut out = format!("# 3D Walkabout progress\npoints = {}\n", self.points);
        for tree in Tree::ALL {
            out += &format!("{} = {}\n", tree.key(), self.level(tree));
        }
        let found: Vec<String> = self.found.iter().map(|i| i.to_string()).collect();
        out += &format!("found = {}\nxp = {}\nxp_points = {}\n", found.join(","), self.xp, self.xp_points);
        for stat in Stat::ALL {
            out += &format!("{} = {}\n", stat.key(), self.stat(stat));
        }
        let items: Vec<String> = self.items.iter().map(|i| i.to_string()).collect();
        out += &format!("scrap = {}\nitems = {}\n", self.scrap, items.join(","));
        out
    }

    pub fn parse(text: &str) -> Self {
        let mut p = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let (key, value) = (key.trim(), value.trim());
            if let Some(stat) = Stat::ALL.into_iter().find(|s| s.key() == key) {
                if let Ok(v) = value.parse::<f32>()
                    && v.is_finite()
                {
                    p.stats[stat.index()] = v.max(0.0);
                }
                continue;
            }
            if let Some(tree) = Tree::ALL.into_iter().find(|t| t.key() == key) {
                let cap = tree.max_level().unwrap_or(1000);
                p.levels[tree.index()] = value.parse::<u32>().map(|v| v.min(cap)).unwrap_or(0);
                continue;
            }
            match key {
                "points" => p.points = value.parse().unwrap_or(0).min(10_000),
                "found" => p.found = value.split(',').filter_map(|s| s.trim().parse().ok()).collect(),
                "xp" => p.xp = value.parse::<f32>().ok().filter(|v| v.is_finite()).unwrap_or(0.0).max(0.0),
                "xp_points" => p.xp_points = value.parse().unwrap_or(0),
                "scrap" => p.scrap = value.parse().unwrap_or(0),
                "items" => {
                    p.items = value
                        .split(',')
                        .filter_map(|s| s.trim().parse().ok())
                        .filter(|i| *i < crate::items::ITEMS.len())
                        .collect()
                }
                _ => {}
            }
        }
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(tree: Tree, level: u32) -> Moveset {
        let mut levels = [0; TREES];
        levels[tree.index()] = level;
        Moveset::from_levels(levels)
    }

    #[test]
    fn upgrading_spends_points_and_unlocks_in_order() {
        let mut p = Progress::default();
        assert!(p.upgrade(Tree::Air).is_none(), "no points yet");
        assert!(p.collect(3));
        assert!(!p.collect(3), "a crystal only counts once");
        assert_eq!(p.upgrade(Tree::Air).unwrap().name, "High Jump");
        assert_eq!(p.points, 0);
        assert_eq!(Moveset::from_levels(p.levels()).air_jumps, 0);
        p.points = 1;
        p.upgrade(Tree::Air);
        assert_eq!(Moveset::from_levels(p.levels()).air_jumps, 1);
    }

    #[test]
    fn max_levels_grant_everything() {
        let mut levels = [MAX_LEVEL; TREES];
        levels[Tree::Ascension.index()] = 0;
        let m = Moveset::from_levels(levels);
        assert!(m.speed > 1.7 && m.dash_charges == 6 && m.slam && m.glide);
        assert!(m.mirror_clones && m.switch_places);
        assert!(m.wall_climb && m.wall_run.is_infinite() && m.mantle > 2.0);
        assert!(m.run_up && m.free_flight && m.time_stop && m.slingshot && m.pads == 3);
        assert!(m.weapons == 4 && m.clones_fire && m.weapon_damage >= 3.0);
        assert!(m.boosts >= UNLIMITED && m.dash_any_direction && m.momentum_hold && m.max_clones == crate::clones::MAX_CLONES);
        assert_eq!(Moveset::from_levels([0; TREES]), Moveset::default());
    }

    #[test]
    fn capstones_cost_five_points_and_need_their_trial() {
        let mut p = Progress { points: 4, ..Default::default() };
        for _ in 0..4 {
            p.upgrade(Tree::Movement);
        }
        assert_eq!((p.level(Tree::Movement), p.points), (4, 0));
        p.points = 10;
        // Points alone aren't enough...
        assert!(p.blocker(Tree::Movement).unwrap().starts_with("Trial"));
        assert!(p.upgrade(Tree::Movement).is_none());
        // ...the trial has to be done too.
        for _ in 0..149 {
            assert_eq!(p.record(Stat::Dashes, 1.0), None);
        }
        assert_eq!(p.record(Stat::Dashes, 1.0), Some(Tree::Movement), "reports the trial completing once");
        assert_eq!(p.record(Stat::Dashes, 1.0), None);
        assert_eq!(p.upgrade(Tree::Movement).unwrap().name, "Phase Shift");
        assert_eq!(p.points, 10 - CAPSTONE_COST);
        assert_eq!(p.blocker(Tree::Movement).as_deref(), Some("Fully upgraded"));
        // Best-so-far trials only go up, and progress is saved.
        p.record_best(Stat::FootSpeed, 30.0);
        p.record_best(Stat::FootSpeed, 12.0);
        assert_eq!(p.stat(Stat::FootSpeed), 30.0);
        assert_eq!(Progress::parse(&p.serialize()), p);
    }

    #[test]
    fn items_need_all_their_parts_and_scrap() {
        let mut p = Progress { scrap: 1000, ..Default::default() };
        // Kinetic Boots (6) needs Spring Boots (0) and Light Frame (1).
        assert_eq!(p.item_blocker(6).as_deref(), Some("Needs Spring Boots + Light Frame"));
        assert!(p.buy_item(0));
        assert_eq!(p.item_blocker(6).as_deref(), Some("Needs Light Frame"));
        assert!(!p.buy_item(6));
        assert!(p.buy_item(1) && p.buy_item(6));
        assert_eq!(p.scrap, 1000 - 10 - 10 - 25);
        assert_eq!(p.item_blocker(6).as_deref(), Some("Owned"));
        let mut poor = Progress::default();
        assert_eq!(poor.item_blocker(0).as_deref(), Some("Needs 10 scrap"));
        assert!(!poor.buy_item(0));
        assert_eq!(Progress::parse(&p.serialize()), p);
    }

    #[test]
    fn ascension_keeps_stacking() {
        let mut p = Progress { points: 100, ..Default::default() };
        for _ in 0..40 {
            assert!(p.upgrade(Tree::Ascension).is_some(), "Ascension has no cap");
        }
        let mut levels = p.levels();
        levels[Tree::Movement.index()] = 5;
        let m = Moveset::from_levels(levels);
        assert!(m.speed > 1.8 * 3.0, "speed {}", m.speed);
        assert!(m.dash_charges >= 2 + 13);
        assert!(m.air_jumps >= 10);
        // Ascension alone doesn't grant abilities you don't have.
        assert_eq!(with(Tree::Ascension, 40).max_clones, 0);
        assert_eq!(with(Tree::Ascension, 40).grapple_range, 0.0);
    }

    #[test]
    fn experience_earns_points_at_a_rising_cost() {
        let mut p = Progress::default();
        assert!(!p.add_xp(59.0));
        assert!(p.add_xp(1.0));
        assert_eq!(p.points, 1);
        assert_eq!(p.xp_needed(), 75.0);
    }

    #[test]
    fn progress_round_trips_and_reads_old_files() {
        let mut p = Progress { points: 2, xp: 12.5, xp_points: 3, ..Default::default() };
        p.levels[Tree::Air.index()] = 3;
        p.levels[Tree::Ascension.index()] = 17;
        p.found = vec![0, 4, 7];
        assert_eq!(Progress::parse(&p.serialize()), p);
        let old = Progress::parse("points = 1\nmovement = 2\nair = 3\nclone = 0\nfound = 1,2\n");
        assert_eq!(old.level(Tree::Air), 3);
        assert_eq!(old.found, vec![1, 2]);
    }
}
