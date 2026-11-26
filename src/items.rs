//! Items: every passive ability (and the weapons and moves that have no
//! button of their own), bought with scrap (from drones, crystals
//! and experience) in a tree of its own. Gear, movement, weapon upgrades,
//! health and fury, and Ascension at the top, bought again and again.
//!
//! Items only strengthen what you have: the Phase Battery adds dash charges
//! to the Dash skill, it doesn't give you a dash. The hardest items (marked
//! with a trial) ask you to do something first, like the last ranks of
//! skills.

use crate::abilities::{Group, Moveset, Stat, Trial, UNLIMITED, trial};

pub struct Item {
    pub name: &'static str,
    /// Scrap to buy (Ascension: the first level; it rises).
    pub cost: u32,
    /// Items that must all be owned first (indices into [`ITEMS`]).
    pub needs: &'static [usize],
    pub description: &'static str,
    pub group: Group,
    pub trial: Option<Trial>,
}

const fn item(name: &'static str, cost: u32, needs: &'static [usize], group: Group, description: &'static str) -> Item {
    Item { name, cost, needs, description, group, trial: None }
}

const fn capstone(
    name: &'static str,
    cost: u32,
    needs: &'static [usize],
    group: Group,
    trial: Option<Trial>,
    description: &'static str,
) -> Item {
    Item { name, cost, needs, description, group, trial }
}

// Indices (saved in progress files: only ever add at the end).
pub const SPRING_BOOTS: usize = 0;
pub const LIGHT_FRAME: usize = 1;
pub const FOCUS_LENS: usize = 2;
pub const CAPACITOR: usize = 3;
pub const THRUSTER_PACK: usize = 4;
pub const GRIP_GLOVES: usize = 5;
pub const KINETIC_BOOTS: usize = 6;
pub const TARGETING_VISOR: usize = 7;
pub const JETPACK_CORE: usize = 8;
pub const CLIMBING_RIG: usize = 9;
pub const MOMENTUM_COIL: usize = 10;
pub const OVERDRIVE_ENGINE: usize = 11;
pub const SIEGE_MODULE: usize = 12;
pub const PHASE_BATTERY: usize = 13;
pub const SINGULARITY_CORE: usize = 14;
pub const SPEED_I: usize = 15;
pub const SPEED_II: usize = 16;
pub const SPEED_III: usize = 17;
pub const HIGH_JUMP: usize = 18;
pub const MANTLE: usize = 19;
pub const AIR_STRAFE: usize = 20;
pub const BUNNY_HOP: usize = 21;
pub const SURF: usize = 22;
pub const UNSTOPPABLE: usize = 23;
pub const DEEP_RESERVES: usize = 24;
pub const KEEN_EDGE: usize = 25;
pub const CONCUSSION: usize = 26;
pub const ION_COIL: usize = 27;
pub const SLUG_SHELLS: usize = 28;
pub const NAPALM: usize = 29;
pub const OVERCHARGED: usize = 30;
pub const MASTER_GUNSMITH: usize = 31;
pub const ANNIHILATOR: usize = 32;
pub const TOUGH: usize = 33;
pub const REGENERATION: usize = 34;
pub const ARMOR: usize = 35;
pub const OVERSHIELD: usize = 36;
pub const IRON_BODY: usize = 37;
pub const SECOND_WIND: usize = 38;
pub const BLOODTHIRST: usize = 39;
pub const ADRENALINE: usize = 40;
pub const VAMPIRIC: usize = 41;
pub const BERSERKER: usize = 42;
pub const WARDEN: usize = 43;
pub const ASCENSION: usize = 44;
pub const MASONRY: usize = 45;
pub const FIELD_MEDIC: usize = 46;
pub const BLADE: usize = 47;
pub const WARHAMMER: usize = 48;
pub const SCYTHE: usize = 49;
pub const BLASTER: usize = 50;
pub const SCATTER: usize = 51;
pub const ROCKETS: usize = 52;
pub const RAILGUN: usize = 53;
pub const DOUBLE_JUMP: usize = 54;
pub const GROUND_SLAM: usize = 55;
pub const GLIDING: usize = 56;
pub const SKYBORNE: usize = 57;
pub const WALL_RUN: usize = 58;
pub const SPIDER_GOD: usize = 59;
pub const SLIDE: usize = 60;
pub const ARCANE_FOCUS: usize = 61;
pub const UNSHAKABLE_WILL: usize = 62;
pub const ARCANE_MASTERY: usize = 63;
pub const AIR_CASTING: usize = 64;

/// Capstone items (those behind a trial) before Ascension opens.
pub const ASCENSION_CAPSTONES: usize = 2;

const GEAR: Group = Group::Mastery;
const MOTION: Group = Group::Motion;
const COMBAT: Group = Group::Combat;
const BODY: Group = Group::Body;
const CONTROL: Group = Group::Control;

pub const ITEMS: [Item; 65] = [
    // Gear.
    item("Spring Boots", 10, &[], GEAR, "Jump 15% higher."),
    item("Light Frame", 10, &[], GEAR, "Run 10% faster."),
    item("Focus Lens", 10, &[], GEAR, "Weapons hit 15% harder."),
    item("Capacitor", 10, &[], GEAR, "Dashes recharge 20% faster; weapons fire 10% faster."),
    item("Thruster Pack", 10, &[], GEAR, "One more rocket boost per flight (with Wings)."),
    item("Grip Gloves", 10, &[], GEAR, "Slide down walls slowly; wall runs last 1 s longer."),
    item("Kinetic Boots", 25, &[SPRING_BOOTS, LIGHT_FRAME], GEAR, "One more jump in mid-air (with Double Jump)."),
    item("Targeting Visor", 25, &[FOCUS_LENS, CAPACITOR], GEAR, "Weapons fire 25% faster."),
    item("Jetpack Core", 25, &[THRUSTER_PACK, CAPACITOR], GEAR, "3 s more hover (with Hover)."),
    item("Climbing Rig", 25, &[GRIP_GLOVES, SPRING_BOOTS], GEAR, "Wall jumps 30% stronger (with Wall Run)."),
    item("Momentum Coil", 25, &[LIGHT_FRAME, GRIP_GLOVES], GEAR, "Momentum can build one step higher (+1x)."),
    item("Overdrive Engine", 50, &[KINETIC_BOOTS, MOMENTUM_COIL], GEAR, "Run 25% faster, and sprinting builds momentum."),
    item("Siege Module", 50, &[TARGETING_VISOR, JETPACK_CORE], GEAR, "Weapons hit 30% harder; rocket blasts 3 m wider."),
    item("Phase Battery", 50, &[JETPACK_CORE, CLIMBING_RIG, KINETIC_BOOTS], GEAR, "Two more dash charges (with Dash)."),
    item(
        "Singularity Core",
        120,
        &[OVERDRIVE_ENGINE, SIEGE_MODULE, PHASE_BATTERY],
        GEAR,
        "Everything 25% stronger (speed, jumps, damage, fire rate), and drones drop double scrap.",
    ),
    // Movement.
    item("Speed I", 8, &[], MOTION, "Run 25% faster."),
    item("Speed II", 15, &[SPEED_I], MOTION, "Run 50% faster."),
    item("Speed III", 30, &[SPEED_II], MOTION, "Run 80% faster."),
    item("High Jump", 8, &[], MOTION, "Jump 60% higher."),
    item("Mantle", 15, &[GRIP_GLOVES], MOTION, "Grab and climb onto ledges up to 2.5 m above your feet."),
    item("Air Strafe", 15, &[HIGH_JUMP], MOTION, "Full control of your direction in mid-air."),
    item("Bunny Hop", 15, &[SPEED_I], MOTION, "Jump just as you land to keep your speed and add 12%."),
    item("Downhill Surf", 25, &[BUNNY_HOP], MOTION, "Slides speed up going downhill and last as long as the slope."),
    capstone(
        "Unstoppable",
        60,
        &[SPEED_III, SURF],
        MOTION,
        trial(Stat::FootSpeed, 40.0, "Reach 40 m/s on foot", "m/s"),
        "Momentum up to 10x and it never fades while you keep moving. Sprinting alone builds to 3x.",
    ),
    item("Deep Reserves", 20, &[HIGH_JUMP], MOTION, "Hover and levitation last 6 s and slowly recharge in the air (with Hover)."),
    // Weapons.
    item("Keen Edge", 10, &[BLADE], COMBAT, "Blade hits make drones bleed. Hold to extend the blade, then release to send a slash farther based on charge."),
    item("Concussion", 20, &[KEEN_EDGE, WARHAMMER], COMBAT, "Hammer blows send a shockwave through everything around the impact; stuns last 2.5 s."),
    item("Ion Coil", 10, &[BLASTER], COMBAT, "Blaster shocks every 3rd hit, twice as hard, and the shock arcs to a drone nearby."),
    item("Slug Shells", 20, &[ION_COIL, SCATTER], COMBAT, "Scatter pellets hit 50% harder, knock drones twice as far and stagger them."),
    item("Napalm Warheads", 20, &[ION_COIL, ROCKETS], COMBAT, "Rocket fires burn three times as hot and twice as long."),
    item("Overcharged Rail", 30, &[SLUG_SHELLS, NAPALM, RAILGUN], COMBAT, "Railgun criticals deal x3 and the beam shocks everything it passes through."),
    capstone(
        "Master Gunsmith",
        50,
        &[CONCUSSION, OVERCHARGED],
        COMBAT,
        trial(Stat::Effects, 300.0, "Inflict 300 status effects", ""),
        "Every status effect (bleed, burn, shock, stun) is twice as strong, and guns reload 25% faster.",
    ),
    capstone(
        "Annihilator",
        100,
        &[MASTER_GUNSMITH, SIEGE_MODULE],
        COMBAT,
        trial(Stat::Kills, 40.0, "Destroy 40 drones", ""),
        "Triple damage, triple fire rate, 16-pellet scatter, 10 m rocket blasts, and your clones fire with you.",
    ),
    // Health.
    item("Tough", 10, &[], BODY, "+25 max health."),
    item("Regeneration", 20, &[TOUGH], BODY, "Health starts coming back 2 s after a hit instead of 5 s, three times as fast."),
    item("Armor Plating", 20, &[TOUGH], BODY, "Take 25% less damage."),
    item("Overshield", 30, &[ARMOR], BODY, "A 40-point energy shield over your health. It soaks hits first and recharges on its own."),
    item("Iron Body", 30, &[TOUGH, REGENERATION], BODY, "+50 max health."),
    capstone(
        "Second Wind",
        60,
        &[OVERSHIELD, IRON_BODY],
        BODY,
        trial(Stat::DamageTaken, 1000.0, "Survive 1,000 damage in total", ""),
        "+50 max health, and once a minute a hit that would down you leaves you at 40% health with 2 s of invulnerability.",
    ),
    item("Bloodthirst", 15, &[TOUGH, KEEN_EDGE], BODY, "Melee hits heal you for 15% of the damage they deal."),
    item("Adrenaline", 25, &[BLOODTHIRST], BODY, "Below 35% health: +30% damage and +20% speed."),
    item("Vampiric Rounds", 30, &[BLOODTHIRST, ION_COIL], BODY, "Every weapon heals you for 8% of the damage it deals."),
    capstone(
        "Berserker",
        60,
        &[ADRENALINE, VAMPIRIC],
        BODY,
        trial(Stat::Healing, 500.0, "Heal 500 health from your attacks", ""),
        "Adrenaline is always on, every drone destroyed restores 20 health, and melee swings 40% faster.",
    ),
    capstone(
        "Warden",
        60,
        &[OVERSHIELD, MASONRY],
        CONTROL,
        trial(Stat::Absorbed, 1500.0, "Have your structures soak up 1,500 damage", ""),
        "Everything you build has triple health and lasts half as long again; ceilings drop low enough to pin whoever is under them, and domes slowly shrink, squeezing whoever is inside.",
    ),
    capstone(
        "Ascension",
        40,
        &[],
        Group::Mastery,
        None,
        "Buy again and again, without end: +6% speed, +8% jump height, +5% rocket, pad and weapon power and +5 max health per level, extra boosts, dashes, air jumps and clones every few levels, and your barriers, ceilings and domes grow bigger and tougher.",
    ),
    item("Masonry", 15, &[TOUGH], CONTROL, "Barriers, ceilings and domes have 50% more health and last 25% longer."),
    item("Field Medic", 20, &[REGENERATION], BODY, "Healing pads heal 50% faster, and regeneration starts a second sooner."),
    // Weapons: always ready on 1-7 once owned, no skill slot needed.
    item("Blade", 5, &[], COMBAT, "Weapon (1). A quick energy blade: left mouse slashes everything in a wide 100° arc up to 4 m ahead."),
    item("Warhammer", 20, &[BLADE], COMBAT, "Weapon (2). A slow, crushing swing 5 m wide and long: heavy damage that knocks drones flying and stuns them."),
    capstone(
        "Reaper's Scythe",
        60,
        &[WARHAMMER],
        COMBAT,
        trial(Stat::MeleeHits, 150.0, "Land 150 melee hits", ""),
        "Weapon (3). A huge 190° sweep reaching 6 m. Each swing briefly parries; hits bleed, heal you, and deal double melee damage.",
    ),
    item("Blaster", 15, &[BLADE], COMBAT, "Weapon (4). Rapid energy bolts (hold left mouse). Every 4th hit on a drone shocks it."),
    item("Scatter Gun", 25, &[BLASTER, WARHAMMER], COMBAT, "Weapon (5). A spread of pellets, deadly up close. Pellets knock drones back."),
    item("Rocket Launcher", 35, &[SCATTER], COMBAT, "Weapon (6). Rockets explode, set drones burning and shove everything near, you included: aim at your feet to rocket jump."),
    item("Railgun", 45, &[ROCKETS], COMBAT, "Weapon (7). An instant beam that pierces every drone in line. Critical hit (x2) on undamaged targets."),
    // Moves that use the movement keys: always on once owned.
    item("Double Jump", 10, &[HIGH_JUMP], MOTION, "Space in mid-air: jump again."),
    item("Ground Slam", 15, &[DOUBLE_JUMP], MOTION, "Ctrl in mid-air: slam down, shaking the ground, stunning and hitting drones nearby."),
    item(
        "Gliding Flight",
        25,
        &[DOUBLE_JUMP],
        MOTION,
        "Space in mid-air (after your jumps) spreads your wings; again to fold them. Look down to dive, up to pull out. Shift fires a rocket boost (3 per flight).",
    ),
    capstone(
        "Skyborne",
        60,
        &[GLIDING],
        MOTION,
        trial(Stat::GlideDistance, 2000.0, "Glide 2,000 m", "m"),
        "Unlimited rocket boosts, and rockets twice as fast. You never have to land.",
    ),
    item("Wall Run", 20, &[DOUBLE_JUMP, GRIP_GLOVES], MOTION, "Against a wall in mid-air: Space kicks off it, Shift runs along it for 1.5 s."),
    capstone(
        "Spider God",
        60,
        &[WALL_RUN, MANTLE],
        MOTION,
        trial(Stat::WallRunDistance, 500.0, "Wall-run 500 m", "m"),
        "Wall runs never end, sprint straight up any wall, mantle ledges up to 20 m, and wall jumps launch twice as hard.",
    ),
    item("Slide", 10, &[SPEED_I], MOTION, "Ctrl while running: a fast slide that builds momentum."),
    // Arcane.
    item("Arcane Focus", 15, &[], Group::Arcane, "Spells cast 40% faster."),
    item(
        "Unshakable Will",
        30,
        &[ARCANE_FOCUS, TOUGH],
        Group::Arcane,
        "Walk at 60% speed while casting instead of a crawl, and taking hits can't spoil a spell.",
    ),
    capstone(
        "Arcane Mastery",
        70,
        &[UNSHAKABLE_WILL],
        Group::Arcane,
        trial(Stat::SpellsCast, 30.0, "Cast 30 spells", ""),
        "Spells are 50% stronger and cast 20% faster still.",
    ),
    item(
        "Air Casting",
        20,
        &[ARCANE_FOCUS],
        Group::Arcane,
        "Plant spells in mid-air: cast while jumping, falling or flying, not only standing on the ground.",
    ),
];

/// Scrap a destroyed drone drops (before the Singularity Core doubles it).
pub const DRONE_SCRAP: u32 = 5;
/// Scrap a skill crystal gives.
pub const CRYSTAL_SCRAP: u32 = 15;
/// Scrap for each skill point earned from experience.
pub const XP_SCRAP: u32 = 10;

/// Adds the effects of the `owned` items to `m` (after the skills).
pub fn apply(m: &mut Moveset, owned: &[usize]) {
    let has = |i: usize| owned.contains(&i);
    // Weapons.
    use crate::weapons::Weapon;
    for (item, weapon) in [
        (BLADE, Weapon::Blade),
        (WARHAMMER, Weapon::Hammer),
        (SCYTHE, Weapon::Scythe),
        (BLASTER, Weapon::Blaster),
        (SCATTER, Weapon::Scatter),
        (ROCKETS, Weapon::Rockets),
        (RAILGUN, Weapon::Railgun),
    ] {
        if has(item) {
            m.weapons |= 1 << weapon.index();
        }
    }
    if has(SCYTHE) {
        m.melee_damage *= 2.0;
        m.melee_lifesteal = m.melee_lifesteal.max(0.25);
    }
    // Moves on the movement keys.
    if has(DOUBLE_JUMP) {
        m.air_jumps += 1;
    }
    m.slam |= has(GROUND_SLAM);
    if has(GLIDING) {
        m.glide = true;
        m.boosts = 3;
    }
    if has(SKYBORNE) && has(GLIDING) {
        m.boosts = UNLIMITED;
        m.rocket_speed = 55.0;
    }
    if has(WALL_RUN) {
        m.wall_jump = true;
        m.wall_run = 1.5;
    }
    if has(SPIDER_GOD) && has(WALL_RUN) {
        m.wall_run = f32::INFINITY;
        m.wall_climb = true;
        m.mantle = m.mantle.max(20.0);
        m.wall_jump_power = 2.0;
    }
    if has(SLIDE) {
        m.slide = true;
        m.momentum_cap = m.momentum_cap.max(1.6);
    }
    // Movement.
    let speed = [SPEED_I, SPEED_II, SPEED_III].into_iter().filter(|&i| has(i)).count();
    m.speed *= [1.0, 1.25, 1.5, 1.8][speed];
    if has(HIGH_JUMP) {
        m.jump *= 1.6f32.sqrt();
    }
    if has(MANTLE) {
        m.mantle = m.mantle.max(2.5);
    }
    m.air_strafe |= has(AIR_STRAFE);
    if has(BUNNY_HOP) {
        m.bunny_hop = true;
        m.momentum_cap = m.momentum_cap.max(3.0);
    }
    if has(SURF) {
        m.surf = true;
        m.momentum_cap = m.momentum_cap.max(3.0);
    }
    if has(UNSTOPPABLE) {
        m.run_up = true;
        m.run_up_cap = 3.0;
        m.momentum_hold = true;
        m.momentum_cap = 10.0;
    }
    if has(DEEP_RESERVES) && m.hover_time > 0.0 {
        m.hover_time = m.hover_time.max(6.0);
        m.hover_recharge = true;
    }
    // Weapons.
    m.bleed |= has(KEEN_EDGE);
    m.blade_cone |= has(KEEN_EDGE);
    m.concussion |= has(CONCUSSION);
    m.ion_coil |= has(ION_COIL);
    m.slug_shells |= has(SLUG_SHELLS);
    m.napalm |= has(NAPALM);
    m.overcharge |= has(OVERCHARGED);
    if has(MASTER_GUNSMITH) {
        m.effect_power = 2.0;
        m.fire_rate *= 1.25;
    }
    // Health.
    if has(TOUGH) {
        m.max_health += 25.0;
    }
    if has(REGENERATION) {
        m.regen_delay = 2.0;
        m.regen_rate *= 3.0;
    }
    if has(ARMOR) {
        m.armor = 0.75;
    }
    if has(OVERSHIELD) {
        m.shield = 40.0;
    }
    if has(IRON_BODY) {
        m.max_health += 50.0;
    }
    if has(SECOND_WIND) {
        m.max_health += 50.0;
        m.second_wind = true;
    }
    if has(BLOODTHIRST) {
        m.melee_lifesteal = m.melee_lifesteal.max(0.15);
    }
    m.adrenaline |= has(ADRENALINE);
    if has(VAMPIRIC) {
        m.lifesteal = 0.08;
    }
    if has(BERSERKER) {
        m.berserker = true;
        m.adrenaline = true;
        m.melee_speed *= 1.4;
    }
    if has(FIELD_MEDIC) {
        m.heal_pad *= 1.5;
        m.regen_delay = (m.regen_delay - 1.0).max(1.0);
    }
    // Arcane.
    if has(ARCANE_FOCUS) {
        m.cast_speed *= 1.4;
    }
    m.steady_cast |= has(UNSHAKABLE_WILL);
    m.air_cast |= has(AIR_CASTING);
    if has(ARCANE_MASTERY) {
        m.spell_power *= 1.5;
        m.cast_speed *= 1.2;
    }
    // Building.
    if has(MASONRY) {
        m.build_health *= 1.5;
        m.build_life *= 1.25;
    }
    if has(WARDEN) {
        m.warden = true;
        m.build_health *= 3.0;
        m.build_life *= 1.5;
    }
    // Gear.
    if has(SPRING_BOOTS) {
        m.jump *= 1.15f32.sqrt();
    }
    if has(LIGHT_FRAME) {
        m.speed *= 1.1;
    }
    if has(FOCUS_LENS) {
        m.weapon_damage *= 1.15;
    }
    if has(CAPACITOR) {
        m.dash_recharge *= 0.8;
        m.fire_rate *= 1.1;
    }
    if has(THRUSTER_PACK) && m.glide && m.boosts < UNLIMITED {
        m.boosts += 1;
    }
    if has(GRIP_GLOVES) {
        m.wall_slide = true;
        if m.wall_run > 0.0 {
            m.wall_run += 1.0;
        }
    }
    if has(KINETIC_BOOTS) && m.air_jumps > 0 {
        m.air_jumps += 1;
    }
    if has(TARGETING_VISOR) {
        m.fire_rate *= 1.25;
    }
    if has(JETPACK_CORE) && m.hover_time > 0.0 {
        m.hover_time += 3.0;
    }
    if has(CLIMBING_RIG) && m.wall_jump {
        m.wall_jump_power *= 1.3;
    }
    if has(MOMENTUM_COIL) {
        m.momentum_cap += 1.0;
    }
    if has(OVERDRIVE_ENGINE) {
        m.speed *= 1.25;
        m.run_up = true;
    }
    if has(SIEGE_MODULE) {
        m.weapon_damage *= 1.3;
        m.blast_radius += 3.0;
    }
    if has(PHASE_BATTERY) && m.dash_charges > 0 {
        m.dash_charges += 2;
    }
    if has(SINGULARITY_CORE) {
        m.speed *= 1.25;
        m.jump *= 1.25f32.sqrt();
        m.weapon_damage *= 1.25;
        m.fire_rate *= 1.25;
        m.scrap_multiplier = 2;
    }
    if has(ANNIHILATOR) {
        m.clones_fire = true;
        m.fire_rate *= 3.0;
        m.weapon_damage *= 3.0;
        m.blast_radius = m.blast_radius.max(12.0);
        m.pellets = 16;
    }
}

/// Every item (for sandbox mode; Ascension is a level, not an item).
pub fn all() -> Vec<usize> {
    (0..ITEMS.len()).filter(|&i| i != ASCENSION).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abilities::Skill;

    #[test]
    fn every_item_can_be_bought() {
        let mut p = crate::abilities::Progress::default();
        p.scrap = 1_000_000;
        for stat in crate::abilities::Stat::ALL {
            p.record(stat, 1e9);
        }
        while (0..ITEMS.len()).any(|i| i != ASCENSION && p.buy_item(i)) {}
        for i in all() {
            assert!(p.has_item(i), "{} can't be bought", ITEMS[i].name);
        }
        assert!(p.buy_item(ASCENSION) && p.buy_item(ASCENSION), "Ascension is bought again and again");
        assert_eq!(p.ascension, 2);
    }

    #[test]
    fn weapons_and_movement_moves_are_items() {
        let m = Moveset::build(&[], &[BLADE, BLASTER, DOUBLE_JUMP, GLIDING, KINETIC_BOOTS], 0);
        assert!(m.has_weapon(crate::weapons::Weapon::Blade) && m.has_weapon(crate::weapons::Weapon::Blaster));
        assert!(!m.has_weapon(crate::weapons::Weapon::Railgun));
        assert_eq!(m.air_jumps, 2, "Double Jump plus Kinetic Boots");
        assert!(m.glide && m.boosts == 3);
    }

    #[test]
    fn items_only_strengthen_skills_you_have() {
        let plain = Moveset::build(&[], &all(), 0);
        assert_eq!(plain.dash_charges, 0, "no dash from items alone");
        assert!(plain.speed > 2.5 && plain.max_health >= 225.0 && plain.shield > 0.0);
        let skilled = Moveset::build(&[(Skill::Dash, 1)], &all(), 0);
        assert_eq!(skilled.dash_charges, 3);
        assert_eq!(skilled.scrap_multiplier, 2);
    }
}
