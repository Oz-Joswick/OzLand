//! What the character can do.
//!
//! - **Skills** are the active abilities: each has its own key, some have
//!   ranks (Dash, then Phase Shift). They form one connected tree bought
//!   with skill points, and you take at most [`LOADOUT`] of them into the
//!   world at once: an unequipped skill does nothing.
//! - **Items** are the passives (speed, health, weapon effects...), bought
//!   with scrap in the item tree (see `items.rs`), Ascension included.
//!
//! Skill points come from glowing crystals hidden around the valley (see
//! `orbs.rs`) and from experience: distance travelled, time in the air and
//! drones destroyed. Progress is saved next to the settings file; saves
//! from before skills and items were split are converted on load.

use crate::settings::Settings;

/// Skills you can have equipped at once.
pub const LOADOUT: usize = 10;
/// Saved loadouts.
pub const PRESETS: usize = 4;
/// The key each of the ten slots starts on (rebindable per slot).
pub const SLOT_KEYS: [&str; LOADOUT] = ["Q", "E", "F", "C", "Z", "B", "T", "G", "H", "RMB"];
/// Points for the last rank of a skill that has a trial.
pub const CAPSTONE_COST: u32 = 4;
/// Rocket boosts that never run out.
pub const UNLIMITED: u32 = 9999;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Skill {
    SplitShot,
    Block,
    Barrier,
    Ceiling,
    Dome,
    HealPad,
    Pull,
    Repulse,
    GroundPull,
    GravityWell,
    Dash,
    Recall,
    Rewind,
    TimeWarp,
    Clone,
    LowGravity,
    Hover,
    Pad,
    Grapple,
    Meteor,
    ChainLightning,
    Blizzard,
    ArcaneWard,
    RecallHome,
    GiantBox,
    Dispel,
    ChronoEcho,
    RiftGate,
    Fireball,
    ArcaneMissiles,
    FrostNova,
    ArcaneLance,
    Rejuvenate,
    Haste,
    ShadowStep,
    RapidFire,
    StunGrenade,
    Whirlwind,
    IronSkin,
    Mend,
    VampiricAura,
    Stasis,
    Leap,
    ArcaneClone,
}
pub const SKILLS: usize = Skill::ALL.len();

/// One rank of a skill.
pub struct Rank {
    pub name: &'static str,
    pub description: &'static str,
}

const fn rank(name: &'static str, description: &'static str) -> Rank {
    Rank { name, description }
}

/// The tree's regions, for colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Combat,
    Body,
    Motion,
    Control,
    Arcane,
    Mastery,
}

impl Group {
    pub fn name(self) -> &'static str {
        match self {
            Group::Combat => "Combat",
            Group::Body => "Body",
            Group::Motion => "Motion",
            Group::Control => "Control",
            Group::Arcane => "Arcane",
            Group::Mastery => "Mastery",
        }
    }
}

/// Running totals kept for trials (the hardest ranks and items).
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
    MeleeHits,
    Effects,
    DamageTaken,
    Healing,
    Absorbed,
    SpellsCast,
}

impl Stat {
    pub const ALL: [Stat; 15] = [
        Stat::Dashes,
        Stat::GlideDistance,
        Stat::Swaps,
        Stat::WallRunDistance,
        Stat::FootSpeed,
        Stat::HoverTime,
        Stat::Rewinds,
        Stat::GrappleDistance,
        Stat::Kills,
        Stat::MeleeHits,
        Stat::Effects,
        Stat::DamageTaken,
        Stat::Healing,
        Stat::Absorbed,
        Stat::SpellsCast,
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
            Stat::MeleeHits => "trial_melee_hits",
            Stat::Effects => "trial_effects",
            Stat::DamageTaken => "trial_damage_taken",
            Stat::Healing => "trial_healing",
            Stat::Absorbed => "trial_absorbed",
            Stat::SpellsCast => "trial_spells",
        }
    }
}

/// What the hardest unlocks ask of you first.
#[derive(Clone, Copy, Debug)]
pub struct Trial {
    pub stat: Stat,
    pub goal: f32,
    pub description: &'static str,
    /// Unit shown with the progress ("m", "s", "m/s" or "").
    pub unit: &'static str,
}

pub const fn trial(stat: Stat, goal: f32, description: &'static str, unit: &'static str) -> Option<Trial> {
    Some(Trial { stat, goal, description, unit })
}

impl Skill {
    pub const ALL: [Skill; 44] = [
        Skill::SplitShot,
        Skill::Block,
        Skill::Barrier,
        Skill::Ceiling,
        Skill::Dome,
        Skill::HealPad,
        Skill::Pull,
        Skill::Repulse,
        Skill::GroundPull,
        Skill::GravityWell,
        Skill::Dash,
        Skill::Recall,
        Skill::Rewind,
        Skill::TimeWarp,
        Skill::Clone,
        Skill::LowGravity,
        Skill::Hover,
        Skill::Pad,
        Skill::Grapple,
        Skill::Meteor,
        Skill::ChainLightning,
        Skill::Blizzard,
        Skill::ArcaneWard,
        Skill::RecallHome,
        Skill::GiantBox,
        Skill::Dispel,
        Skill::ChronoEcho,
        Skill::RiftGate,
        Skill::Fireball,
        Skill::ArcaneMissiles,
        Skill::FrostNova,
        Skill::ArcaneLance,
        Skill::Rejuvenate,
        Skill::Haste,
        Skill::ShadowStep,
        Skill::RapidFire,
        Skill::StunGrenade,
        Skill::Whirlwind,
        Skill::IronSkin,
        Skill::Mend,
        Skill::VampiricAura,
        Skill::Stasis,
        Skill::Leap,
        Skill::ArcaneClone,
    ];

    pub fn index(self) -> usize {
        Skill::ALL.iter().position(|s| *s == self).expect("listed")
    }

    pub fn name(self) -> &'static str {
        self.ranks()[0].name
    }

    pub fn group(self) -> Group {
        use Skill::*;
        match self {
            SplitShot | RapidFire | StunGrenade | Whirlwind => Group::Combat,
            Meteor | ChainLightning | Blizzard | ArcaneWard | RecallHome | TimeWarp | Dispel | ChronoEcho | RiftGate | Fireball | ArcaneMissiles
            | FrostNova | ArcaneLance | Rejuvenate | Haste | ShadowStep | ArcaneClone => Group::Arcane,
            Block | HealPad | IronSkin | Mend | VampiricAura => Group::Body,
            Barrier | Ceiling | Dome | GiantBox | Pull | Repulse | GroundPull | GravityWell | Stasis => Group::Control,
            _ => Group::Motion,
        }
    }

    pub fn ranks(self) -> &'static [Rank] {
        use Skill::*;
        match self {
            SplitShot => const {
                &[
                    rank("Split Shot", "For 8 s every gun fires three shots in a fan, each at 60% damage. Ready again 16 s later."),
                    rank("Fan of Five", "Five shots, each at 70%, for 10 s."),
                ]
            },
            Block => const {
                &[
                    rank("Block", "Hold to raise a guard: 70% less damage from everything, but you move slowly and can't attack."),
                    rank("Parry", "Raise your guard just before a hit (0.3 s) and it does nothing at all, heals you 10 and stuns drones nearby."),
                ]
            },
            Barrier => const {
                &[
                    rank("Barrier", "Press the Barrier key once for the first ground corner, then aim and press again for the other end. It snaps to the nearest surface within 20 m, or places the corner 20 m away. The wall blocks players and shots; its reach grows with Ascension."),
                    rank("Bulwark", "Two barriers at once, with double health, lasting twice as long."),
                ]
            },
            Ceiling => const {
                &[rank(
                    "Ceiling",
                    "Drop a huge solid slab (28 m square) over the other player or where you aim: nobody under it can jump or fly out of the top. 10 s.",
                )]
            },
            Dome => const {
                &[rank(
                    "Dome",
                    "Trap the other player (or a spot you aim at) in a dome 12 m across. Nothing gets in or out, shots included, until it breaks or 12 s pass.",
                )]
            },
            HealPad => const {
                &[
                    rank("Healing Pad", "Place a glowing pad that heals anyone standing on it, 12 health a second, for 10 s. The other player too. Ready again 20 s later."),
                    rank("Sanctuary", "20 health a second for 15 s, and it recharges your shield."),
                ]
            },
            Pull => const {
                &[
                    rank("Tether", "Yank whatever you aim at (the other player, or drones in a narrow cone) towards you. Ready again 10 s later."),
                    rank("Reel In", "Twice as hard, and what you pull is stunned for 1 s."),
                ]
            },
            Repulse => const {
                &[
                    rank("Repulse", "A blast that throws everything within 8 m away from you and destroys drone shots in flight. Ready again 12 s later."),
                    rank("Shockwave", "12 m, much harder, and it damages what it throws."),
                ]
            },
            GroundPull => const {
                &[
                    rank("Ground Pull", "Drag whatever you aim at straight down to the ground and pin it there for 2 s: no jumping, flying or gliding. Ready again 14 s later."),
                    rank("Earthbind", "Pins everything within 10 m of where you aim, for 3 s."),
                ]
            },
            GravityWell => const {
                &[
                    rank("Gravity Well", "Open a vortex where you aim: for 6 s it drags players and drones towards its centre. Ready again 18 s later."),
                    rank("Singularity", "Twice the pull for 9 s, and drones caught in it are stunned."),
                ]
            },
            Dash => const {
                &[
                    rank("Teleport Dash", "Blink up to 9 m ahead. Recharges in 2 s."),
                    rank("Phase Shift", "Six dash charges recharging every 0.3 s, and each blink goes 25 m wherever you look: up into the sky, down off cliffs, across the lake."),
                ]
            },
            Recall => const { &[rank("Recall Marker", "Drop a marker; press again to return to it.")] },
            Rewind => const {
                &[
                    rank("Rewind", "Jump back to where you were 3 s ago."),
                    rank("Long Rewind", "Rewind 6 s. Recall and rewind refill your jumps, boosts and hover."),
                ]
            },
            TimeWarp => const {
                &[
                    rank("Bullet Time", "For 5 s the world runs at 30% speed while you move at 70%."),
                    rank("Chronomancer", "Stop the world entirely for 10 s (shots, drones, grass and water), ready again 4 s later."),
                ]
            },
            Clone => const {
                &[
                    rank("Mirror Clone", "Spawn a mirrored clone beside you: you and your clones stand in a circle, all facing its middle, and it mirrors your movement. Drones may target it (40 health); it lasts until destroyed. R dismisses clones."),
                    rank("Twin Clones", "Two mirrored clones; X swaps places with the newest."),
                    rank("Legion", "An army of eight clones, and every one of them fires whenever you do."),
                ]
            },
            LowGravity => const { &[rank("Low Gravity", "Toggle moon gravity.")] },
            Hover => const {
                &[
                    rank("Hover", "Hold in mid-air to hang in place (2 s, refills on landing)."),
                    rank("Levitate", "Holding now rises instead."),
                    rank("True Flight", "Press your Hover key to toggle flight on or off. Fly wherever you look, 2.5x faster than you run. Flight lasts 60 s, then recharges."),
                ]
            },
            Pad => const {
                &[
                    rank("Bounce Pad", "Drop a pad that launches you skyward."),
                    rank("Super Pads", "Three pads at once, launching much higher."),
                ]
            },
            Grapple => const {
                &[
                    rank("Grapple Hook", "Hold to hook what you're aiming at (35 m) and get pulled to it."),
                    rank("Long Hook", "Hook reaches 70 m and pulls twice as hard."),
                    rank("Web Slinger", "150 m hook with a huge pull; letting go flings you 2.5x and refills your jumps and boosts; pads launch at 50 m/s."),
                ]
            },
            Meteor => const {
                &[
                    rank("Meteor", "SPELL. Call a burning meteor down on the other player or where you aim: a 20 m blast that sets everything alight and throws it about."),
                    rank("Meteor Shower", "Five meteors rain down across the area instead of one."),
                ]
            },
            ChainLightning => const {
                &[
                    rank("Chain Lightning", "SPELL. Lightning leaps from your target to everything nearby, jumping up to 22 m: ten strikes that shock and stun drones, and hurt the other player."),
                    rank("Storm Caller", "Sixteen strikes, each harder."),
                ]
            },
            Blizzard => const {
                &[
                    rank("Blizzard", "SPELL. An 8 s storm 44 m across where you aim: players in it are slowed and chilled, drones frozen and battered."),
                    rank("Absolute Zero", "Anyone inside when it lands is frozen solid for 2 s."),
                ]
            },
            ArcaneWard => const {
                &[
                    rank("Arcane Ward", "SPELL. A 150-point magic barrier around you that soaks damage for 8 s."),
                    rank("Mirror Ward", "250 points, and casting it heals you 40."),
                ]
            },
            GiantBox => const {
                &[
                    rank(
                        "Giant Box",
                        "Drop a colossal solid box (30 m wide, 20 m tall, floored and roofed) around the other player or where you aim. Nothing gets in or out until its walls break (1,500 health) or 20 s pass. Grows with Ascension.",
                    ),
                    rank("Citadel", "A third bigger, double the health, and it stands for 30 s."),
                ]
            },
            Dispel => const { &[rank("Dispel", "SPELL. Shatter every structure within 12 m of where you aim, even yours.")] },
            ChronoEcho => const { &[
                rank("Chrono Echo", "SPELL. Your next ability (within 15 s) happens twice: once, then again a split second later, free."),
                rank("Afterimage", "Your next two abilities happen twice."),
            ] },
            RiftGate => const { &[rank("Rift Gate", "SPELL. Aim at two spots to open linked gates. Players and shots that cross one emerge from the other.")] },
            RecallHome => const {
                &[rank("Recall Home", "SPELL. Return to the village square, fully healed. Taking a hit while casting spoils it.")]
            },
            Fireball => const {
                &[
                    rank("Fireball", "SPELL. Hurl a great fireball at the crosshair: it bursts over 14 m and sets everything alight."),
                    rank("Inferno", "Three great fireballs at once, in a wide fan."),
                ]
            },
            ArcaneMissiles => const {
                &[
                    rank("Arcane Missiles", "SPELL. Unleash a swarm of 16 seeking arcane bolts: they fan out wide, then curve onto every drone (or player) ahead and shock what they hit."),
                    rank("Arcane Barrage", "28 bolts, each a little stronger."),
                ]
            },
            FrostNova => const {
                &[
                    rank("Frost Nova", "SPELL. Ice bursts out 18 m around you: drones freeze for 2.5 s and are thrown back, the other player is stunned, and incoming shots shatter."),
                    rank("Deep Freeze", "26 m, and the ice hurts everything it reaches."),
                ]
            },
            ArcaneLance => const {
                &[
                    rank("Arcane Lance", "SPELL. A fan of seven searing beams sweeps the area in front of you, piercing and shocking everything they cross."),
                    rank("Prism", "Thirteen beams, in a wider fan."),
                ]
            },
            Rejuvenate => const {
                &[
                    rank("Rejuvenate", "SPELL. Heal 30 at once, and a healing circle 15 m across spreads round you for 6 s (anyone standing in it heals)."),
                    rank("Renewal", "Heal 50, a stronger circle, and a 60-point ward."),
                ]
            },
            Haste => const {
                &[
                    rank("Haste", "SPELL. For 6 s you move, fire and swing 35% faster."),
                    rank("Quicksilver", "Lasts 10 s."),
                ]
            },
            RapidFire => const {
                &[
                    rank("Rapid Fire", "For 4 s every weapon fires and swings 80% faster."),
                    rank("Bullet Storm", "Lasts 6 s."),
                ]
            },
            StunGrenade => const {
                &[
                    rank("Stun Grenade", "Throw a grenade where you aim (up to 35 m): a tight 3 m burst that hits hard, stunning drones for 2.5 s and the other player for 1.5 s."),
                    rank("Cluster Charge", "4.5 m, and it hits even harder."),
                ]
            },
            Whirlwind => const {
                &[
                    rank("Whirlwind", "Spin with your weapon out: everything right next to you (3.5 m) is struck hard and knocked away."),
                    rank("Cyclone", "Twice the damage, and drones caught in it are stunned."),
                ]
            },
            IronSkin => const {
                &[
                    rank("Iron Skin", "For 5 s you take half damage from everything."),
                    rank("Adamant", "Lasts 7 s, and you take only 35%."),
                ]
            },
            Mend => const {
                &[
                    rank("Mend", "Patch yourself up: heal 35 at once."),
                    rank("Field Surgeon", "Heal 55."),
                ]
            },
            VampiricAura => const {
                &[
                    rank("Vampiric Aura", "For 6 s, 30% of all damage you deal heals you."),
                    rank("Blood Pact", "50%, for 8 s."),
                ]
            },
            Stasis => const {
                &[
                    rank("Stasis Field", "Freeze the drone or player you aim at: a drone stops dead for 5 s, the other player for 2.5 s."),
                    rank("Time Prison", "Everything within 4 m of them is frozen too."),
                ]
            },
            Leap => const {
                &[
                    rank("Leap", "Spring forward and up the way you're facing, on the ground or in the air."),
                    rank("Pounce", "A much bigger leap."),
                ]
            },
            ArcaneClone => const {
                &[
                    rank(
                        "Arcane Clone",
                        "SPELL. Call up a pair of violet clones, one either side of you, that copy every attack you make and your attack spells (Fireball, Meteor, Lightning, Blizzard, Missiles, Lance, Frost Nova), from where they stand. Each fades after 5 of your actions, or when drones destroy it.",
                    ),
                    rank("Arcane Coven", "A second pair, further out: four at once."),
                ]
            },
            ShadowStep => const {
                &[
                    rank("Shadow Step", "SPELL. Vanish and reappear just behind the drone or player you aim at, facing them."),
                    rank("Assassin", "Whatever you step behind is stunned for 1.5 s."),
                ]
            },
        }
    }

    /// Spells take this long to cast (None: instant): deliberately slow,
    /// since a cast plants the spell in the world to set off later.
    pub fn cast_time(self) -> Option<f32> {
        use Skill::*;
        match self {
            Barrier | Ceiling | Dome | GiantBox => Some(1.4),
            Dispel => Some(1.6),
            ChronoEcho => Some(1.5),
            RiftGate => Some(1.8),
            Meteor => Some(3.0),
            ChainLightning => Some(1.9),
            Blizzard => Some(3.2),
            ArcaneWard => Some(2.0),
            RecallHome => Some(4.0),
            TimeWarp => Some(1.9),
            Fireball => Some(1.4),
            ArcaneMissiles => Some(1.8),
            FrostNova => Some(1.2),
            Haste | ShadowStep => Some(1.1),
            ArcaneClone => Some(1.6),
            ArcaneLance => Some(2.2),
            Rejuvenate => Some(2.4),
            _ => None,
        }
    }

    pub fn max_rank(self) -> u32 {
        self.ranks().len() as u32
    }

    pub fn rank_info(self, rank: u32) -> &'static Rank {
        let r = self.ranks();
        &r[(rank.max(1) as usize - 1).min(r.len() - 1)]
    }

    /// What the last rank asks of you before it can be bought.
    pub fn trial(self) -> Option<Trial> {
        use Skill::*;
        match self {
            Dash => trial(Stat::Dashes, 150.0, "Dash 150 times", ""),
            Clone => trial(Stat::Swaps, 40.0, "Switch places with a clone 40 times", ""),
            Hover => trial(Stat::HoverTime, 120.0, "Hover or levitate for 120 s in total", "s"),
            TimeWarp => trial(Stat::Rewinds, 30.0, "Rewind 30 times", ""),
            Grapple => trial(Stat::GrappleDistance, 1000.0, "Ride the grapple 1,000 m", "m"),
            Meteor => trial(Stat::SpellsCast, 25.0, "Cast 25 spells", ""),
            _ => None,
        }
    }

    /// Points for `rank`: one per rank, more for a rank behind a trial.
    pub fn cost(self, rank: u32) -> u32 {
        if self.trial().is_some() && rank == self.max_rank() { CAPSTONE_COST } else { rank }
    }

    /// Skills this one grows out of (rank needed). None: from the centre.
    pub fn needs(self) -> &'static [(Skill, u32)] {
        use Skill::*;
        match self {
            Barrier | HealPad => &[(Block, 1)],
            Ceiling => &[(Barrier, 1), (Pad, 1)],
            Dome => &[(Ceiling, 1)],
            GiantBox => &[(Dome, 1), (Barrier, 2)],
            Repulse => &[(Barrier, 1)],
            Pull => &[(Grapple, 1)],
            GroundPull => &[(Pull, 1), (Repulse, 1)],
            GravityWell => &[(Dome, 1), (Repulse, 1)],
            Recall => &[(Dash, 1)],
            Rewind => &[(Recall, 1)],
            TimeWarp => &[(Rewind, 1)],
            Clone => &[(Recall, 1), (Pad, 1)],
            Hover => &[(LowGravity, 1)],
            Grapple => &[(Pad, 1)],
            ChainLightning | Blizzard => &[(ArcaneWard, 1)],
            Meteor => &[(ChainLightning, 1), (Blizzard, 1)],
            RecallHome => &[(ArcaneWard, 1), (Recall, 1)],
            Dispel => &[(ArcaneWard, 1), (Barrier, 1)],
            ChronoEcho => &[(TimeWarp, 1), (ChainLightning, 1)],
            RiftGate => &[(Dispel, 1), (Recall, 1)],
            Fireball => &[(ArcaneWard, 1)],
            ArcaneMissiles => &[(Fireball, 1)],
            FrostNova => &[(Blizzard, 1)],
            ArcaneLance => &[(ArcaneMissiles, 1), (ChainLightning, 1)],
            Rejuvenate => &[(ArcaneWard, 1), (HealPad, 1)],
            Haste => &[(ArcaneWard, 1), (Dash, 1)],
            ShadowStep => &[(Haste, 1)],
            ArcaneClone => &[(ChronoEcho, 1)],
            RapidFire | Mend => &[],
            StunGrenade => &[(SplitShot, 1)],
            Whirlwind => &[(RapidFire, 1)],
            IronSkin => &[(Block, 1)],
            VampiricAura => &[(Mend, 1), (IronSkin, 1)],
            Stasis => &[(Dome, 1)],
            Leap => &[(Dash, 1)],
            SplitShot | Block | Dash | LowGravity | Pad | ArcaneWard => &[],
        }
    }

    /// Name in the save file.
    pub fn key_name(self) -> &'static str {
        use Skill::*;
        match self {
            SplitShot => "split_shot",
            Block => "block",
            Barrier => "barrier",
            Ceiling => "ceiling",
            Dome => "dome",
            HealPad => "heal_pad",
            Pull => "pull",
            Repulse => "repulse",
            GroundPull => "ground_pull",
            GravityWell => "gravity_well",
            Dash => "dash",
            Recall => "recall",
            Rewind => "rewind",
            TimeWarp => "time_warp",
            Clone => "clone_skill",
            LowGravity => "low_gravity",
            Hover => "hover",
            Pad => "pad",
            Grapple => "grapple",
            Meteor => "meteor",
            ChainLightning => "chain_lightning",
            Blizzard => "blizzard",
            ArcaneWard => "arcane_ward",
            RecallHome => "recall_home",
            GiantBox => "giant_box",
            Dispel => "dispel",
            ChronoEcho => "chrono_echo",
            RiftGate => "rift_gate",
            Fireball => "fireball",
            ArcaneMissiles => "arcane_missiles",
            FrostNova => "frost_nova",
            ArcaneLance => "arcane_lance",
            Rejuvenate => "rejuvenate",
            Haste => "haste",
            ShadowStep => "shadow_step",
            ArcaneClone => "arcane_clone",
            RapidFire => "rapid_fire",
            StunGrenade => "stun_grenade",
            Whirlwind => "whirlwind",
            IronSkin => "iron_skin",
            Mend => "mend",
            VampiricAura => "vampiric_aura",
            Stasis => "stasis",
            Leap => "leap",
        }
    }

    /// What `rank` of this skill gives.
    fn apply(self, rank: u32, m: &mut Moveset) {
        use Skill::*;
        match self {
            SplitShot => (m.split, m.split_damage, m.split_time) = if rank >= 2 { (5, 0.7, 10.0) } else { (3, 0.6, 8.0) },
            Block => {
                m.block = 0.3;
                m.parry = rank >= 2;
            }
            Barrier => {
                m.barriers = rank.min(2);
                if rank >= 2 {
                    m.build_health *= 2.0;
                    m.build_life *= 2.0;
                }
            }
            Ceiling => m.ceiling = true,
            Dome => m.dome = true,
            HealPad => (m.heal_pad, m.heal_pad_life, m.heal_pad_shield) = if rank >= 2 { (20.0, 15.0, true) } else { (12.0, 10.0, false) },
            Pull => (m.pull, m.pull_stun) = if rank >= 2 { (36.0, 1.0) } else { (18.0, 0.0) },
            Repulse => (m.repulse, m.repulse_radius, m.repulse_damage) = if rank >= 2 { (30.0, 12.0, 3.0) } else { (18.0, 8.0, 0.0) },
            GroundPull => (m.ground_pull, m.ground_pull_area) = if rank >= 2 { (3.0, 10.0) } else { (2.0, 0.0) },
            GravityWell => (m.gravity_well, m.well_life) = if rank >= 2 { (2.0, 9.0) } else { (1.0, 6.0) },
            Dash => {
                m.dash_charges = 1;
                if rank >= 2 {
                    m.dash_charges = 6;
                    m.dash_recharge = 0.3;
                    m.dash_distance = 25.0;
                    m.dash_any_direction = true;
                }
            }
            Recall => m.recall = true,
            Rewind => {
                m.rewind = if rank >= 2 { 6.0 } else { 3.0 };
                m.time_refill = rank >= 2;
            }
            TimeWarp => {
                m.bullet_time = true;
                if rank >= 2 {
                    m.time_stop = true;
                    m.time_stop_length = 10.0;
                    m.time_cooldown = 4.0;
                }
            }
            Clone => {
                m.max_clones = [1, 2, crate::clones::MAX_CLONES][(rank.max(1) - 1).min(2) as usize];
                m.switch_places = rank >= 2;
                m.clones_fire |= rank >= 3;
            }
            LowGravity => m.low_gravity = true,
            Hover => {
                m.hover_time = m.hover_time.max(2.0);
                m.levitate = rank >= 2;
                if rank >= 3 {
                    m.free_flight = true;
                    m.fly_speed = 1.0;
                }
            }
            Pad => {
                m.pads = if rank >= 2 { 3 } else { 1 };
                if rank >= 2 {
                    m.pad_launch = 28.0;
                }
            }
            Meteor => m.meteors = if rank >= 2 { 5 } else { 1 },
            ChainLightning => m.lightning_jumps = if rank >= 2 { 16 } else { 10 },
            Blizzard => m.blizzard = rank,
            ArcaneWard => m.ward = if rank >= 2 { 250.0 } else { 150.0 },
            RecallHome => m.recall_home = true,
            GiantBox => m.giant_box = rank,
            Dispel => m.dispel = rank,
            ChronoEcho => m.chrono_echo = rank,
            RiftGate => m.rift_gate = true,
            Fireball => m.fireball = rank,
            ArcaneMissiles => m.missiles = rank,
            FrostNova => m.frost_nova = rank,
            ArcaneLance => m.lance = rank,
            Rejuvenate => m.rejuvenate = rank,
            Haste => m.haste = rank,
            ShadowStep => m.shadow_step = rank,
            ArcaneClone => m.arcane_clones = if rank >= 2 { 4 } else { 2 },
            RapidFire => m.rapid_fire = rank,
            StunGrenade => m.stun_grenade = rank,
            Whirlwind => m.whirlwind = rank,
            IronSkin => m.iron_skin = rank,
            Mend => m.mend = rank,
            VampiricAura => m.vampiric = rank,
            Stasis => m.stasis = rank,
            Leap => m.leap = rank,
            Grapple => {
                m.grapple_range = [35.0, 70.0, 150.0][(rank.max(1) - 1).min(2) as usize];
                if rank >= 2 {
                    m.grapple_pull = 90.0;
                }
                if rank >= 3 {
                    m.slingshot = true;
                    m.fling = 2.5;
                    m.grapple_pull = 160.0;
                    m.pad_launch = m.pad_launch.max(50.0);
                }
            }
        }
    }
}

/// Where each node of a tree sits, in a circle around the centre: one
/// slice per group, and rings outward by how many steps a node is from the
/// centre (what it needs sits on an inner ring). `outer` nodes go on a ring
/// of their own outside everything. Returns (angle in radians, ring).
pub fn radial_layout(groups: &[Group], needs: &[Vec<usize>], outer: &[bool]) -> Vec<(f32, f32)> {
    let n = groups.len();
    // Ring: one more than the deepest thing needed.
    let mut depth = vec![0u32; n];
    for _ in 0..n {
        for i in 0..n {
            let d = needs[i].iter().map(|&j| depth[j]).max().unwrap_or(0) + 1;
            depth[i] = depth[i].max(d);
        }
    }
    let deepest = depth.iter().copied().max().unwrap_or(1);
    for i in 0..n {
        if outer[i] {
            depth[i] = deepest + 1;
        }
    }
    // A slice of the circle per group, sized by how many nodes it holds.
    let order = [Group::Combat, Group::Body, Group::Control, Group::Arcane, Group::Motion, Group::Mastery];
    let mut slices = Vec::new();
    let mut start = -std::f32::consts::FRAC_PI_2;
    for g in order {
        let count = groups.iter().filter(|x| **x == g).count();
        if count == 0 {
            continue;
        }
        let width = std::f32::consts::TAU * count as f32 / n as f32;
        slices.push((g, start, width));
        start += width;
    }
    let mut angle = vec![0.0f32; n];
    for &(g, start, width) in &slices {
        for d in 1..=deepest + 1 {
            let mut ring: Vec<usize> = (0..n).filter(|&i| groups[i] == g && depth[i] == d).collect();
            // Near what they grow out of: sort by their parents' angles.
            let key = |i: usize| {
                let p: Vec<f32> = needs[i].iter().map(|&j| angle[j]).collect();
                if p.is_empty() { i as f32 * 1e-3 } else { p.iter().sum::<f32>() / p.len() as f32 }
            };
            ring.sort_by(|&a, &b| key(a).total_cmp(&key(b)));
            let count = ring.len().max(1) as f32;
            for (k, &i) in ring.iter().enumerate() {
                angle[i] = start + width * (k as f32 + 0.5) / count;
            }
        }
    }
    (0..n).map(|i| (angle[i], depth[i] as f32)).collect()
}

/// What the character can currently do: built from equipped skills, owned
/// items and Ascension.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Moveset {
    // Movement
    /// Multiplier on walking/running speed.
    pub speed: f32,
    /// Ascension level, also used to tune enemy pressure.
    pub ascension: u32,
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
    pub switch_places: bool,
    /// Clones fire whenever you do (Legion, Annihilator).
    pub clones_fire: bool,
    // Parkour
    pub wall_slide: bool,
    pub wall_jump: bool,
    /// Seconds you can run along a wall (infinite with Spider God).
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
    // Weapons
    /// One bit per weapon in `Weapon::ALL` that's equipped.
    pub weapons: u8,
    /// Blade and Scythe hits bleed (Keen Edge).
    pub bleed: bool,
    /// Keen Edge charges the blade into a ranged fan while held.
    pub blade_cone: bool,
    /// Hammer blows shock the area and stun longer.
    pub concussion: bool,
    /// Multiplier on melee damage.
    pub melee_damage: f32,
    /// Multiplier on melee swing speed.
    pub melee_speed: f32,
    pub ion_coil: bool,
    pub slug_shells: bool,
    pub napalm: bool,
    pub overcharge: bool,
    /// Multiplier on every status effect.
    pub effect_power: f32,
    pub weapon_damage: f32,
    /// Multiplier on fire rate.
    pub fire_rate: f32,
    /// Rocket blast radius, m.
    pub blast_radius: f32,
    /// Scatter Gun pellets.
    pub pellets: u32,
    /// Split Shot: shots per trigger pull (0 = no skill), their damage and
    /// how long it lasts.
    pub split: u32,
    pub split_damage: f32,
    pub split_time: f32,
    // Health
    pub max_health: f32,
    /// Seconds after a hit before health comes back, and how fast (per s).
    pub regen_delay: f32,
    pub regen_rate: f32,
    /// Multiplier on damage taken.
    pub armor: f32,
    /// Energy shield on top of health.
    pub shield: f32,
    pub second_wind: bool,
    /// Fraction of melee damage healed, and of all weapon damage.
    pub melee_lifesteal: f32,
    pub lifesteal: f32,
    pub adrenaline: bool,
    /// Adrenaline always on; kills heal.
    pub berserker: bool,
    /// Block: damage multiplier while guarding (0 = no skill), and Parry.
    pub block: f32,
    pub parry: bool,
    /// Healing pad: health per second (0 = no skill), lifetime, shield.
    pub heal_pad: f32,
    pub heal_pad_life: f32,
    pub heal_pad_shield: bool,
    // Control
    /// Barriers you can have up at once (0 = none).
    pub barriers: u32,
    pub ceiling: bool,
    pub dome: bool,
    /// Structures: triple health, shrinking domes, pinning ceilings.
    pub warden: bool,
    /// Multipliers on structure health, lifetime and size.
    pub build_health: f32,
    pub build_life: f32,
    pub build_scale: f32,
    /// Pull: launch speed (0 = no skill), and stun on arrival.
    pub pull: f32,
    pub pull_stun: f32,
    /// Repulse: launch speed (0 = no skill), reach and damage.
    pub repulse: f32,
    pub repulse_radius: f32,
    pub repulse_damage: f32,
    /// Ground Pull: seconds pinned (0 = no skill), and area (0 = one target).
    pub ground_pull: f32,
    pub ground_pull_area: f32,
    /// Gravity Well: strength (0 = no skill) and lifetime.
    pub gravity_well: f32,
    pub well_life: f32,
    // Spells
    /// Meteors per cast (0 = no spell), lightning strikes, Blizzard rank,
    /// Arcane Ward strength, Recall Home.
    pub meteors: u32,
    pub lightning_jumps: u32,
    pub blizzard: u32,
    pub ward: f32,
    pub recall_home: bool,
    /// Rank of the structure-breaking dispel spell.
    pub dispel: u32,
    /// Rank of the delayed repeat attack spell.
    pub chrono_echo: u32,
    /// Whether linked dimensional gates can be placed.
    pub rift_gate: bool,
    /// Ranks of the newer spells (0 = not learned).
    pub fireball: u32,
    pub missiles: u32,
    pub frost_nova: u32,
    pub lance: u32,
    pub rejuvenate: u32,
    pub haste: u32,
    pub shadow_step: u32,
    /// Arcane clones you can have at once (0 = no Arcane Clone).
    pub arcane_clones: usize,
    /// Ranks of the newer combat, body, control and motion skills.
    pub rapid_fire: u32,
    pub stun_grenade: u32,
    pub whirlwind: u32,
    pub iron_skin: u32,
    pub mend: u32,
    pub vampiric: u32,
    pub stasis: u32,
    pub leap: u32,
    /// Giant Box rank (0 = no skill).
    pub giant_box: u32,
    /// Multiplier on cast speed and spell strength; casts that a hit can't
    /// spoil and that don't slow you.
    pub cast_speed: f32,
    pub spell_power: f32,
    pub steady_cast: bool,
    /// Spells can be planted in mid-air, not only on the ground.
    pub air_cast: bool,
    // Items
    /// Multiplier on scrap from drones (Singularity Core).
    pub scrap_multiplier: u32,
}

impl Default for Moveset {
    /// Nothing equipped: the plain character.
    fn default() -> Self {
        Self {
            speed: 1.0,
            ascension: 0,
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
            bleed: false,
            blade_cone: false,
            concussion: false,
            melee_damage: 1.0,
            melee_speed: 1.0,
            ion_coil: false,
            slug_shells: false,
            napalm: false,
            overcharge: false,
            effect_power: 1.0,
            weapon_damage: 1.0,
            fire_rate: 1.0,
            blast_radius: 6.0,
            pellets: 8,
            split: 0,
            split_damage: 1.0,
            split_time: 0.0,
            max_health: 100.0,
            regen_delay: 5.0,
            regen_rate: 6.0,
            armor: 1.0,
            shield: 0.0,
            second_wind: false,
            melee_lifesteal: 0.0,
            lifesteal: 0.0,
            adrenaline: false,
            berserker: false,
            block: 0.0,
            parry: false,
            heal_pad: 0.0,
            heal_pad_life: 10.0,
            heal_pad_shield: false,
            barriers: 0,
            ceiling: false,
            dome: false,
            warden: false,
            build_health: 1.0,
            build_life: 1.0,
            build_scale: 1.0,
            pull: 0.0,
            pull_stun: 0.0,
            repulse: 0.0,
            repulse_radius: 8.0,
            repulse_damage: 0.0,
            ground_pull: 0.0,
            ground_pull_area: 0.0,
            gravity_well: 0.0,
            well_life: 6.0,
            meteors: 0,
            lightning_jumps: 0,
            blizzard: 0,
            ward: 0.0,
            recall_home: false,
            dispel: 0,
            chrono_echo: 0,
            rift_gate: false,
            fireball: 0,
            missiles: 0,
            frost_nova: 0,
            lance: 0,
            rejuvenate: 0,
            haste: 0,
            shadow_step: 0,
            arcane_clones: 0,
            rapid_fire: 0,
            stun_grenade: 0,
            whirlwind: 0,
            iron_skin: 0,
            mend: 0,
            vampiric: 0,
            stasis: 0,
            leap: 0,
            giant_box: 0,
            cast_speed: 1.0,
            spell_power: 1.0,
            steady_cast: false,
            air_cast: false,
            scrap_multiplier: 1,
        }
    }
}

impl Moveset {
    /// Equipped skills (with ranks), then items, then Ascension on top.
    pub fn build(skills: &[(Skill, u32)], items: &[usize], ascension: u32) -> Self {
        let mut m = Self::default();
        for &(s, r) in skills {
            if r > 0 {
                s.apply(r, &mut m);
            }
        }
        crate::items::apply(&mut m, items);
        m.ascend(ascension);
        m
    }

    /// Ascension: everything you have, stronger, without end, structures
    /// bigger and tougher.
    fn ascend(&mut self, a: u32) {
        if a == 0 {
            return;
        }
        let f = a as f32;
        self.ascension = a;
        self.speed *= 1.0 + 0.06 * f;
        self.jump *= (1.0 + 0.08 * f).sqrt();
        self.rocket_speed *= 1.0 + 0.05 * f;
        self.pad_launch *= 1.0 + 0.05 * f;
        self.weapon_damage *= 1.0 + 0.05 * f;
        if self.boosts < UNLIMITED && self.boosts > 0 {
            self.boosts += a / 2;
        }
        if self.air_jumps > 0 {
            self.air_jumps += a / 4;
        }
        if self.dash_charges > 0 {
            self.dash_charges += a / 3;
        }
        if self.max_clones > 0 {
            self.max_clones = (self.max_clones + (a / 5) as usize).min(crate::clones::MAX_CLONES);
        }
        if self.grapple_range > 0.0 {
            self.grapple_range += 5.0 * f;
        }
        if self.hover_time > 0.0 {
            self.hover_time *= 1.0 + 0.1 * f;
        }
        self.max_health += 5.0 * f;
        // Structures grow (up to twice the size) and toughen without limit.
        self.build_scale *= (1.0 + 0.04 * f).min(2.0);
        self.build_health *= 1.0 + 0.1 * f;
        self.spell_power *= 1.0 + 0.04 * f;
    }

    /// Whether `weapon` is equipped.
    pub fn has_weapon(&self, weapon: crate::weapons::Weapon) -> bool {
        self.weapons & (1 << weapon.index()) != 0
    }

    /// Any weapon at all.
    pub fn armed(&self) -> bool {
        self.weapons != 0
    }
}

/// Something a trial unlocks: a skill's last rank or an item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unlock {
    Skill(Skill),
    Item(usize),
}

impl Unlock {
    pub fn name(self) -> &'static str {
        match self {
            Unlock::Skill(s) => s.rank_info(s.max_rank()).name,
            Unlock::Item(i) => crate::items::ITEMS[i].name,
        }
    }
}

/// Skill points and ranks, the loadout, experience, items and scrap, which
/// crystals were found, and totals for the trials.
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    pub points: u32,
    ranks: [u32; SKILLS],
    /// Equipped skills, in the order equipped (at most [`LOADOUT`]).
    loadout: [Option<Skill>; LOADOUT],
    /// Saved loadouts to switch between, slot by slot.
    presets: [[Option<Skill>; LOADOUT]; PRESETS],
    /// Indices of crystals already collected.
    pub found: Vec<usize>,
    /// Experience towards the next point.
    pub xp: f32,
    /// Points earned from experience so far (each costs a little more).
    pub xp_points: u32,
    stats: [f32; Stat::ALL.len()],
    /// Currency for items (from drones and crystals).
    pub scrap: u32,
    /// Items owned (indices into `items::ITEMS`).
    items: Vec<usize>,
    /// Ascension level (an item bought again and again).
    pub ascension: u32,
}

// By hand: arrays of more than 32 ranks have no derived Default.
impl Default for Progress {
    fn default() -> Self {
        Progress {
            points: 0,
            ranks: [0; SKILLS],
            loadout: Default::default(),
            presets: Default::default(),
            found: Vec::new(),
            xp: 0.0,
            xp_points: 0,
            stats: Default::default(),
            scrap: 0,
            // Everyone starts with the Blade.
            items: vec![crate::items::BLADE],
            ascension: 0,
        }
    }
}

impl Progress {
    pub fn rank(&self, skill: Skill) -> u32 {
        self.ranks[skill.index()]
    }

    pub fn ranks(&self) -> [u32; SKILLS] {
        self.ranks
    }

    /// The ten slots, each holding a skill or nothing.
    pub fn slots(&self) -> &[Option<Skill>; LOADOUT] {
        &self.loadout
    }

    /// Equipped skills, slot by slot.
    pub fn loadout(&self) -> Vec<Skill> {
        self.loadout.iter().flatten().copied().collect()
    }

    /// Which slot a skill is in.
    pub fn slot_of(&self, skill: Skill) -> Option<usize> {
        self.loadout.iter().position(|s| *s == Some(skill))
    }

    pub fn equipped(&self, skill: Skill) -> bool {
        self.slot_of(skill).is_some()
    }

    /// Puts a skill you own into the first empty slot.
    pub fn equip(&mut self, skill: Skill) -> Result<(), String> {
        if self.rank(skill) == 0 {
            return Err("Unlock it first".into());
        }
        self.equip_any(skill)
    }

    /// Puts any skill (sandbox) into the first empty slot.
    pub fn equip_any(&mut self, skill: Skill) -> Result<(), String> {
        if self.equipped(skill) {
            return Ok(());
        }
        let Some(free) = self.loadout.iter().position(Option::is_none) else {
            return Err(format!("All {LOADOUT} slots are full: clear one first"));
        };
        self.loadout[free] = Some(skill);
        Ok(())
    }

    /// Puts a skill into slot `slot` (moving it from any other slot, and
    /// replacing what was there). `any`: sandbox, owned or not.
    pub fn place(&mut self, skill: Skill, slot: usize, any: bool) -> Result<(), String> {
        if !any && self.rank(skill) == 0 {
            return Err("Unlock it first".into());
        }
        if slot >= LOADOUT {
            return Err("No such slot".into());
        }
        self.unequip(skill);
        self.loadout[slot] = Some(skill);
        Ok(())
    }

    pub fn unequip(&mut self, skill: Skill) {
        for s in &mut self.loadout {
            if *s == Some(skill) {
                *s = None;
            }
        }
    }

    pub fn clear_slot(&mut self, slot: usize) {
        if let Some(s) = self.loadout.get_mut(slot) {
            *s = None;
        }
    }

    /// A saved loadout, slot by slot.
    pub fn preset(&self, i: usize) -> [Option<Skill>; LOADOUT] {
        self.presets.get(i).copied().unwrap_or_default()
    }

    /// Saves the current slots as saved loadout `i`.
    pub fn save_preset(&mut self, i: usize) {
        if let Some(p) = self.presets.get_mut(i) {
            *p = self.loadout;
        }
    }

    /// Swaps to saved loadout `i`.
    pub fn load_preset(&mut self, i: usize) {
        if let Some(p) = self.presets.get(i).filter(|p| p.iter().any(Option::is_some)) {
            self.loadout = *p;
        }
    }

    /// Equipped skills with their ranks (what the Moveset is built from).
    pub fn equipped_ranks(&self) -> Vec<(Skill, u32)> {
        self.loadout().into_iter().map(|s| (s, self.rank(s))).collect()
    }

    pub fn items(&self) -> &[usize] {
        &self.items
    }

    pub fn has_item(&self, item: usize) -> bool {
        if item == crate::items::ASCENSION {
            return self.ascension > 0;
        }
        self.items.contains(&item)
    }

    /// Items behind a trial that you own.
    pub fn capstone_items(&self) -> usize {
        self.items.iter().filter(|&&i| crate::items::ITEMS[i].trial.is_some()).count()
    }

    /// Skill ranks owned across the tree.
    pub fn nodes(&self) -> u32 {
        self.ranks.iter().sum()
    }

    /// What it costs to buy `item` now (Ascension rises each time).
    pub fn item_cost(&self, item: usize) -> u32 {
        let it = &crate::items::ITEMS[item];
        if item == crate::items::ASCENSION { it.cost + 20 * self.ascension } else { it.cost }
    }

    /// What's stopping you from buying `item`, if anything.
    pub fn item_blocker(&self, item: usize) -> Option<String> {
        let it = crate::items::ITEMS.get(item)?;
        if item != crate::items::ASCENSION && self.has_item(item) {
            return Some("Owned".into());
        }
        let missing: Vec<&str> = it.needs.iter().filter(|n| !self.has_item(**n)).map(|n| crate::items::ITEMS[*n].name).collect();
        if !missing.is_empty() {
            return Some(format!("Needs {}", missing.join(" + ")));
        }
        if item == crate::items::ASCENSION && self.capstone_items() < crate::items::ASCENSION_CAPSTONES {
            return Some(format!("Needs {} capstone items ({} so far)", crate::items::ASCENSION_CAPSTONES, self.capstone_items()));
        }
        if let Some(t) = it.trial
            && !self.trial_done(&t)
        {
            return Some(format!("Trial: {}", t.description));
        }
        let cost = self.item_cost(item);
        (self.scrap < cost).then(|| format!("Needs {cost} scrap"))
    }

    /// Spends scrap on `item`; false if it can't be bought yet.
    pub fn buy_item(&mut self, item: usize) -> bool {
        if self.item_blocker(item).is_some() {
            return false;
        }
        self.scrap -= self.item_cost(item);
        if item == crate::items::ASCENSION {
            self.ascension += 1;
        } else {
            self.items.push(item);
        }
        true
    }

    /// What's stopping the next rank of `skill`, if anything.
    pub fn blocker(&self, skill: Skill) -> Option<String> {
        let next = self.rank(skill) + 1;
        if next > skill.max_rank() {
            return Some("Fully ranked".into());
        }
        let missing: Vec<&str> = skill.needs().iter().filter(|&&(s, r)| self.rank(s) < r).map(|(s, _)| s.name()).collect();
        if !missing.is_empty() {
            return Some(format!("Needs {}", missing.join(" + ")));
        }
        if next == skill.max_rank()
            && let Some(t) = skill.trial()
            && !self.trial_done(&t)
        {
            return Some(format!("Trial: {}", t.description));
        }
        let cost = skill.cost(next);
        (self.points < cost).then(|| format!("Needs {cost} point{}", if cost == 1 { "" } else { "s" }))
    }

    pub fn can_upgrade(&self, skill: Skill) -> bool {
        self.blocker(skill).is_none()
    }

    /// Spends points on the next rank; a newly learned skill is equipped
    /// if there's a free slot.
    pub fn upgrade(&mut self, skill: Skill) -> Option<&'static Rank> {
        if !self.can_upgrade(skill) {
            return None;
        }
        let next = self.rank(skill) + 1;
        self.points -= skill.cost(next);
        self.ranks[skill.index()] = next;
        if next == 1 {
            let _ = self.equip(skill);
        }
        Some(skill.rank_info(next))
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

    /// Adds to a trial total. Returns what this just made unlockable.
    pub fn record(&mut self, stat: Stat, amount: f32) -> Option<Unlock> {
        self.set_stat(stat, self.stat(stat) + amount.max(0.0))
    }

    /// Raises a best-so-far trial value (top speed).
    pub fn record_best(&mut self, stat: Stat, value: f32) -> Option<Unlock> {
        self.set_stat(stat, self.stat(stat).max(value))
    }

    fn set_stat(&mut self, stat: Stat, value: f32) -> Option<Unlock> {
        let before = self.stat(stat);
        self.stats[stat.index()] = value;
        let crossed = |t: Option<Trial>| t.is_some_and(|t| t.stat == stat && before < t.goal && value >= t.goal);
        Skill::ALL
            .into_iter()
            .find(|s| crossed(s.trial()))
            .map(Unlock::Skill)
            .or_else(|| (0..crate::items::ITEMS.len()).find(|&i| crossed(crate::items::ITEMS[i].trial)).map(Unlock::Item))
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
        std::fs::read_to_string(Self::path()).map(|t| Self::parse(&t)).unwrap_or_default()
    }

    pub fn save(&self) {
        let path = Self::path();
        let result = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&path, self.serialize()));
        if let Err(e) = result {
            log::warn!("could not save progress to {}: {e}", path.display());
        }
    }

    pub fn serialize(&self) -> String {
        let mut out = format!("# OzLand progress\nformat = 2\npoints = {}\n", self.points);
        for s in Skill::ALL {
            if self.rank(s) > 0 {
                out += &format!("skill_{} = {}\n", s.key_name(), self.rank(s));
            }
        }
        let slots = |l: &[Option<Skill>; LOADOUT]| l.iter().map(|s| s.map_or("", |s| s.key_name())).collect::<Vec<_>>().join(",");
        out += &format!("loadout = {}\n", slots(&self.loadout));
        for (i, p) in self.presets.iter().enumerate() {
            out += &format!("preset_{} = {}\n", i + 1, slots(p));
        }
        let found: Vec<String> = self.found.iter().map(|i| i.to_string()).collect();
        out += &format!("found = {}\nxp = {}\nxp_points = {}\n", found.join(","), self.xp, self.xp_points);
        for stat in Stat::ALL {
            out += &format!("{} = {}\n", stat.key(), self.stat(stat));
        }
        let items: Vec<String> = self.items.iter().map(|i| i.to_string()).collect();
        out += &format!("scrap = {}\nitems = {}\nascension = {}\n", self.scrap, items.join(","), self.ascension);
        out
    }

    pub fn parse(text: &str) -> Self {
        let mut p = Self::default();
        let mut had_loadout = false;
        let mut old_items: Vec<usize> = Vec::new();
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
            if let Some(name) = key.strip_prefix("skill_") {
                if let Some(s) = Skill::ALL.into_iter().find(|s| s.key_name() == name) {
                    p.ranks[s.index()] = value.parse::<u32>().unwrap_or(0).min(s.max_rank());
                } else {
                    // Weapons and moves without their own button became items.
                    use crate::items as it;
                    let became: &[usize] = match name {
                        "blade" => &[it::BLADE],
                        "hammer" => &[it::WARHAMMER],
                        "scythe" => &[it::SCYTHE],
                        "blaster" => &[it::BLASTER],
                        "scatter" => &[it::SCATTER],
                        "rockets" => &[it::ROCKETS],
                        "railgun" => &[it::RAILGUN],
                        "double_jump" => &[it::DOUBLE_JUMP],
                        "slam" => &[it::GROUND_SLAM],
                        "wings" => &[it::GLIDING, it::SKYBORNE],
                        "wall_run" => &[it::WALL_RUN, it::SPIDER_GOD],
                        "slide" => &[it::SLIDE],
                        _ => &[],
                    };
                    old_items.extend(became.iter().take(value.parse::<usize>().unwrap_or(0)));
                }
                continue;
            }
            // Saves from before skills and items were split: tree levels.
            if let Some(grants) = old_tree(key) {
                let level = value.parse::<usize>().unwrap_or(0).min(grants.len());
                for g in &grants[..level] {
                    match *g {
                        Old::Skill(s, r) => p.ranks[s.index()] = p.ranks[s.index()].max(r),
                        Old::Item(i) => old_items.push(i),
                    }
                }
                continue;
            }
            if let Some(n) = key.strip_prefix("preset_").and_then(|n| n.parse::<usize>().ok())
                && (1..=PRESETS).contains(&n)
            {
                p.presets[n - 1] = parse_slots(value);
                continue;
            }
            match key {
                "points" => p.points = value.parse().unwrap_or(0).min(10_000),
                "loadout" => {
                    had_loadout = true;
                    p.loadout = parse_slots(value);
                }
                "found" => p.found = value.split(',').filter_map(|s| s.trim().parse().ok()).collect(),
                "xp" => p.xp = value.parse::<f32>().ok().filter(|v| v.is_finite()).unwrap_or(0.0).max(0.0),
                "xp_points" => p.xp_points = value.parse().unwrap_or(0),
                "scrap" => p.scrap = value.parse().unwrap_or(0),
                "ascension" => p.ascension = value.parse::<u32>().unwrap_or(0).min(1000),
                "items" => {
                    p.items = value
                        .split(',')
                        .filter_map(|s| s.trim().parse().ok())
                        .filter(|i| *i < crate::items::ITEMS.len() && *i != crate::items::ASCENSION)
                        .collect()
                }
                _ => {}
            }
        }
        for i in old_items {
            if !p.items.contains(&i) {
                p.items.push(i);
            }
        }
        // The Blade is where everyone starts (saves from before it was free too).
        if !p.items.contains(&crate::items::BLADE) {
            p.items.insert(0, crate::items::BLADE);
        }
        // Old saves (no loadout yet): equip what you have, up to the limit.
        if !had_loadout {
            for s in Skill::ALL {
                if p.rank(s) > 0 {
                    let _ = p.equip(s);
                }
            }
        }
        p
    }
}

/// "dash,,pad": a skill (or nothing) per slot, in order.
fn parse_slots(value: &str) -> [Option<Skill>; LOADOUT] {
    let mut out = [None; LOADOUT];
    for (slot, name) in value.split(',').take(LOADOUT).enumerate() {
        out[slot] = Skill::ALL.into_iter().find(|s| s.key_name() == name.trim());
    }
    out
}

/// What a level of an old tree became.
enum Old {
    Skill(Skill, u32),
    Item(usize),
}

/// The old trees, level by level, mapped onto skills and items.
fn old_tree(key: &str) -> Option<Vec<Old>> {
    use Old::{Item as I, Skill as S};
    use Skill::*;
    use crate::items as it;
    Some(match key {
        "movement" => vec![I(it::SPEED_I), I(it::SPEED_II), I(it::SPEED_III), S(Dash, 1), S(Dash, 2)],
        "air" => vec![I(it::HIGH_JUMP), I(it::DOUBLE_JUMP), I(it::GROUND_SLAM), I(it::GLIDING), I(it::SKYBORNE)],
        "clone" => vec![S(Clone, 1), S(Clone, 2), S(Clone, 2), S(Clone, 2), S(Clone, 3)],
        "parkour" => vec![I(it::GRIP_GLOVES), I(it::WALL_RUN), I(it::WALL_RUN), I(it::MANTLE), I(it::SPIDER_GOD)],
        "momentum" => vec![I(it::SLIDE), I(it::BUNNY_HOP), I(it::AIR_STRAFE), I(it::SURF), I(it::UNSTOPPABLE)],
        "gravity" => vec![S(LowGravity, 1), S(Hover, 1), S(Hover, 2), I(it::DEEP_RESERVES), S(Hover, 3)],
        "time" => vec![S(Recall, 1), S(Rewind, 1), S(TimeWarp, 1), S(Rewind, 2), S(TimeWarp, 2)],
        "gadgets" => vec![S(Pad, 1), S(Grapple, 1), S(Pad, 2), S(Grapple, 2), S(Grapple, 3)],
        "arsenal" => vec![I(it::BLASTER), I(it::SCATTER), I(it::ROCKETS), I(it::RAILGUN), I(it::ANNIHILATOR)],
        "melee" => vec![I(it::BLADE), I(it::KEEN_EDGE), I(it::WARHAMMER), I(it::CONCUSSION), I(it::SCYTHE)],
        "gunsmith" => vec![I(it::ION_COIL), I(it::SLUG_SHELLS), I(it::NAPALM), I(it::OVERCHARGED), I(it::MASTER_GUNSMITH)],
        "vitality" => vec![I(it::TOUGH), I(it::REGENERATION), I(it::ARMOR), I(it::OVERSHIELD), I(it::SECOND_WIND)],
        "fury" => vec![I(it::BLOODTHIRST), I(it::ADRENALINE), I(it::IRON_BODY), I(it::VAMPIRIC), I(it::BERSERKER)],
        "control" => vec![S(Barrier, 1), S(Barrier, 2), S(Ceiling, 1), S(Dome, 1), I(it::WARDEN)],
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upgrading_spends_points_and_equips() {
        let mut p = Progress::default();
        assert!(p.upgrade(Skill::Dash).is_none(), "no points yet");
        assert!(p.collect(3));
        assert!(!p.collect(3), "a crystal only counts once");
        assert_eq!(p.upgrade(Skill::Dash).unwrap().name, "Teleport Dash");
        assert_eq!(p.points, 0);
        assert!(p.equipped(Skill::Dash), "learned skills equip themselves");
        assert_eq!(Moveset::build(&p.equipped_ranks(), &[], 0).dash_charges, 1);
        p.unequip(Skill::Dash);
        assert_eq!(Moveset::build(&p.equipped_ranks(), &[], 0).dash_charges, 0, "unequipped skills do nothing");
    }

    #[test]
    fn only_ten_skills_fit_in_the_loadout_and_presets_remember_them() {
        let mut p = Progress::default();
        for s in Skill::ALL {
            p.ranks[s.index()] = 1;
        }
        for s in Skill::ALL.into_iter().take(LOADOUT) {
            p.equip(s).unwrap();
        }
        assert!(p.equip(Skill::Grapple).is_err());
        p.save_preset(0);
        p.unequip(Skill::SplitShot);
        assert!(p.equip(Skill::Grapple).is_ok());
        assert_eq!(p.loadout().len(), LOADOUT);
        p.save_preset(2);
        p.load_preset(0);
        assert!(p.equipped(Skill::SplitShot) && !p.equipped(Skill::Grapple));
        // Placing a skill in a slot moves it there (replacing what was there).
        p.place(Skill::Grapple, 3, false).unwrap();
        assert_eq!(p.slots()[3], Some(Skill::Grapple));
        p.place(Skill::Grapple, 7, false).unwrap();
        assert_eq!((p.slots()[3], p.slots()[7]), (None, Some(Skill::Grapple)), "a skill sits in one slot only");
        p.clear_slot(7);
        assert!(!p.equipped(Skill::Grapple));
        assert_eq!(Progress::parse(&p.serialize()), p);
    }

    #[test]
    fn last_ranks_need_their_trial() {
        let mut p = Progress { points: 20, ..Default::default() };
        p.upgrade(Skill::Dash);
        assert!(p.blocker(Skill::Dash).unwrap().starts_with("Trial"));
        for _ in 0..149 {
            assert_eq!(p.record(Stat::Dashes, 1.0), None);
        }
        assert_eq!(p.record(Stat::Dashes, 1.0), Some(Unlock::Skill(Skill::Dash)));
        assert_eq!(p.upgrade(Skill::Dash).unwrap().name, "Phase Shift");
        assert_eq!(p.points, 20 - 1 - CAPSTONE_COST);
        assert_eq!(p.blocker(Skill::Dash).as_deref(), Some("Fully ranked"));
        assert_eq!(Progress::parse(&p.serialize()), p);
    }

    #[test]
    fn the_skill_tree_is_one_connected_circle() {
        let mut p = Progress { points: 10_000, ..Default::default() };
        p.stats = [1e9; Stat::ALL.len()];
        while Skill::ALL.into_iter().any(|s| p.upgrade(s).is_some()) {}
        for s in Skill::ALL {
            assert_eq!(p.rank(s), s.max_rank(), "{s:?} unreachable");
        }
        // Every slot starts on its own key, and each can be bound.
        let mut keys = SLOT_KEYS.to_vec();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), LOADOUT, "two slots share a key");
        assert!(SLOT_KEYS.iter().all(|k| crate::input::Bind::parse(k).is_some()));
        // The circle: what a skill needs is on an inner ring.
        let groups: Vec<Group> = Skill::ALL.iter().map(|s| s.group()).collect();
        let needs: Vec<Vec<usize>> = Skill::ALL.iter().map(|s| s.needs().iter().map(|(n, _)| n.index()).collect()).collect();
        let at = radial_layout(&groups, &needs, &vec![false; groups.len()]);
        for (i, s) in Skill::ALL.iter().enumerate() {
            for &(n, _) in s.needs() {
                assert!(at[n.index()].1 < at[i].1, "{s:?} is not outside {n:?}");
            }
        }
    }

    #[test]
    fn everything_at_once() {
        let skills: Vec<(Skill, u32)> = Skill::ALL.into_iter().map(|s| (s, s.max_rank())).collect();
        let m = Moveset::build(&skills, &crate::items::all(), 0);
        assert!(m.dash_charges >= 6 && m.slam && m.glide && m.free_flight && m.time_stop && m.slingshot);
        assert!(m.weapons == 0b111_1111 && m.clones_fire && m.split == 5 && m.parry);
        assert!(m.max_health >= 200.0 && m.shield > 0.0 && m.ground_pull > 0.0 && m.gravity_well > 0.0);
        assert_eq!(Moveset::build(&[], &[], 0), Moveset::default());
        // Ascension makes structures bigger, up to double.
        assert!(Moveset::build(&[], &[], 10).build_scale > 1.3);
        assert_eq!(Moveset::build(&[], &[], 500).build_scale, 2.0);
    }

    #[test]
    fn old_saves_become_skills_and_items() {
        use crate::items as it;
        let old = Progress::parse("points = 1\nmovement = 4\nair = 2\nmelee = 1\nvitality = 1\nfound = 1,2\nascension = 3\n");
        assert_eq!(old.rank(Skill::Dash), 1);
        assert!(old.has_item(it::SPEED_III) && old.has_item(it::HIGH_JUMP) && old.has_item(it::DOUBLE_JUMP));
        assert!(old.has_item(it::TOUGH) && old.has_item(it::BLADE));
        assert!(old.equipped(Skill::Dash));
        assert_eq!((old.ascension, old.found.clone()), (3, vec![1, 2]));
        assert_eq!(Progress::parse(&old.serialize()), old);
        // Last version's skills that are items now.
        let mid = Progress::parse("skill_blaster = 1\nskill_wings = 2\nskill_dash = 1\nloadout = blaster,dash\n");
        assert!(mid.has_item(it::BLASTER) && mid.has_item(it::GLIDING) && mid.has_item(it::SKYBORNE));
        assert_eq!((mid.loadout(), mid.slot_of(Skill::Dash)), (vec![Skill::Dash], Some(1)), "slots keep their places");
    }

    #[test]
    fn spells_are_skills_with_a_cast_time() {
        let spells: Vec<Skill> = Skill::ALL.into_iter().filter(|s| s.group() == Group::Arcane).collect();
        assert_eq!(spells.len(), 17);
        for s in &spells {
            assert!(s.cast_time().is_some_and(|t| t >= 0.8), "{s:?} should take a moment to cast");
        }
        assert!(Skill::Dash.cast_time().is_none(), "ordinary skills are instant");
        let m = Moveset::build(&[(Skill::Meteor, 2), (Skill::ChainLightning, 1), (Skill::ArcaneWard, 2)], &[crate::items::ARCANE_FOCUS], 0);
        assert_eq!((m.meteors, m.lightning_jumps), (5, 10));
        assert!(m.ward >= 250.0 && m.cast_speed > 1.3 && !m.steady_cast);
        // Spells take loadout slots like any skill.
        let mut p = Progress::default();
        p.ranks[Skill::Meteor.index()] = 1;
        assert!(p.equip(Skill::Meteor).is_ok() && p.equipped(Skill::Meteor));
    }

    #[test]
    fn experience_earns_points_at_a_rising_cost() {
        let mut p = Progress::default();
        assert!(!p.add_xp(59.0));
        assert!(p.add_xp(1.0));
        assert_eq!(p.points, 1);
        assert_eq!(p.xp_needed(), 75.0);
    }
}
