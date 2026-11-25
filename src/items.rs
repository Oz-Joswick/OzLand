//! Items: a gear tree bought with Scrap (from drones and crystals).
//!
//! Like a tech tree, most items need **several** others first: every tier-2
//! item needs two tier-1 items, tier 3 needs two or three tier-2 items, and
//! the Singularity Core needs all of tier 3. Items stack on top of the
//! abilities (see [`apply`]).

use crate::abilities::{Moveset, UNLIMITED};

pub struct Item {
    pub name: &'static str,
    /// 1 (basic) to 4 (the core).
    pub tier: u32,
    /// Scrap to unlock.
    pub cost: u32,
    /// Items that must all be owned first (indices into [`ITEMS`]).
    pub needs: &'static [usize],
    pub description: &'static str,
}

const fn item(name: &'static str, tier: u32, cost: u32, needs: &'static [usize], description: &'static str) -> Item {
    Item { name, tier, cost, needs, description }
}

// Indices, for the requirements below.
const SPRING_BOOTS: usize = 0;
const LIGHT_FRAME: usize = 1;
const FOCUS_LENS: usize = 2;
const CAPACITOR: usize = 3;
const THRUSTER_PACK: usize = 4;
const GRIP_GLOVES: usize = 5;
const KINETIC_BOOTS: usize = 6;
const TARGETING_VISOR: usize = 7;
const JETPACK_CORE: usize = 8;
const CLIMBING_RIG: usize = 9;
const MOMENTUM_COIL: usize = 10;
const OVERDRIVE_ENGINE: usize = 11;
const SIEGE_MODULE: usize = 12;
const PHASE_BATTERY: usize = 13;
const SINGULARITY_CORE: usize = 14;

pub const ITEMS: [Item; 15] = [
    // Tier 1: no requirements.
    item("Spring Boots", 1, 10, &[], "Jump 15% higher."),
    item("Light Frame", 1, 10, &[], "Run 10% faster."),
    item("Focus Lens", 1, 10, &[], "Weapons hit 15% harder."),
    item("Capacitor", 1, 10, &[], "Dashes recharge 20% faster; weapons fire 10% faster."),
    item("Thruster Pack", 1, 10, &[], "One more rocket boost per flight."),
    item("Grip Gloves", 1, 10, &[], "Slide down walls slowly; wall runs last 1 s longer."),
    // Tier 2: two tier-1 items each.
    item("Kinetic Boots", 2, 25, &[SPRING_BOOTS, LIGHT_FRAME], "One more jump in mid-air."),
    item("Targeting Visor", 2, 25, &[FOCUS_LENS, CAPACITOR], "Weapons fire 25% faster."),
    item("Jetpack Core", 2, 25, &[THRUSTER_PACK, CAPACITOR], "3 s more hover (hold right mouse in mid-air), even without the Gravity tree."),
    item("Climbing Rig", 2, 25, &[GRIP_GLOVES, SPRING_BOOTS], "Wall jumps (Space against a wall in mid-air), 30% stronger."),
    item("Momentum Coil", 2, 25, &[LIGHT_FRAME, GRIP_GLOVES], "Momentum can build one step higher (+1x)."),
    // Tier 3: two or three tier-2 items each.
    item("Overdrive Engine", 3, 50, &[KINETIC_BOOTS, MOMENTUM_COIL], "Run 25% faster, and sprinting builds momentum."),
    item("Siege Module", 3, 50, &[TARGETING_VISOR, JETPACK_CORE], "Weapons hit 30% harder; rocket blasts 3 m wider."),
    item("Phase Battery", 3, 50, &[JETPACK_CORE, CLIMBING_RIG, KINETIC_BOOTS], "Two more dash charges (F), even without the Movement tree."),
    // Tier 4: all of tier 3.
    item(
        "Singularity Core",
        4,
        120,
        &[OVERDRIVE_ENGINE, SIEGE_MODULE, PHASE_BATTERY],
        "Everything 25% stronger (speed, jumps, damage, fire rate), two more air jumps and rockets, and drones drop double scrap.",
    ),
];

/// Scrap a destroyed drone drops (before the Singularity Core doubles it).
pub const DRONE_SCRAP: u32 = 5;
/// Scrap a skill crystal gives.
pub const CRYSTAL_SCRAP: u32 = 15;

/// Adds the effects of the `owned` items to `m`.
pub fn apply(m: &mut Moveset, owned: &[usize]) {
    let has = |i: usize| owned.contains(&i);
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
    if has(THRUSTER_PACK) && m.boosts < UNLIMITED {
        m.boosts += 1;
    }
    if has(GRIP_GLOVES) {
        m.wall_slide = true;
        if m.wall_run > 0.0 {
            m.wall_run += 1.0;
        }
    }
    if has(KINETIC_BOOTS) {
        m.air_jumps += 1;
    }
    if has(TARGETING_VISOR) {
        m.fire_rate *= 1.25;
    }
    if has(JETPACK_CORE) {
        m.hover_time += 3.0;
    }
    if has(CLIMBING_RIG) {
        m.wall_jump = true;
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
    if has(PHASE_BATTERY) {
        m.dash_charges += 2;
    }
    if has(SINGULARITY_CORE) {
        m.speed *= 1.25;
        m.jump *= 1.25f32.sqrt();
        m.weapon_damage *= 1.25;
        m.fire_rate *= 1.25;
        m.air_jumps += 2;
        if m.boosts < UNLIMITED {
            m.boosts += 2;
        }
        m.scrap_multiplier = 2;
    }
}

/// Every item index (for sandbox mode).
pub fn all() -> Vec<usize> {
    (0..ITEMS.len()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tree_is_well_formed() {
        for (i, it) in ITEMS.iter().enumerate() {
            for &n in it.needs {
                assert!(n < i && ITEMS[n].tier < it.tier, "{} needs a later or same-tier item", it.name);
            }
            if it.tier > 1 {
                assert!(it.needs.len() >= 2, "{} should need several items", it.name);
            }
        }
        assert_eq!(ITEMS[SINGULARITY_CORE].needs.len(), 3);
    }

    #[test]
    fn items_stack_onto_abilities() {
        let base = Moveset::default();
        let mut m = base;
        apply(&mut m, &all());
        assert!(m.speed > base.speed * 1.7, "speed {}", m.speed);
        assert_eq!(m.air_jumps, 3);
        assert_eq!(m.dash_charges, 2, "Phase Battery gives dashes even without Movement");
        assert!(m.hover_time >= 3.0 && m.wall_jump && m.run_up);
        assert_eq!(m.scrap_multiplier, 2);
    }
}
