//! A first-person 3D scene rendered with wgpu.
//!
//! Controls:
//!   WASD / arrows  move
//!   Mouse          look around
//!   Space          jump
//!   Shift          sprint
//!   Esc            pause / settings menu (Esc again to resume)
//!   Q              quit
//!
//! Set `RUST_LOG=info` to see which GPU and models were loaded.

mod abilities;
mod audio;
mod arena;
mod animation;
mod body;
mod camera;
mod clones;
mod collision;
mod drones;
mod gadgets;
mod input;
mod items;
mod map;
mod mesh;
mod net;
mod model;
mod noise;
mod orbs;
mod placement;
mod player;
mod renderer;
mod scene;
mod settings;
mod steam;
mod structures;
mod terrain;
mod textures;
mod tutorial;
mod ui;
mod updater;
mod weapons;

use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{DeviceEvent, DeviceId, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

use camera::Camera;
use input::Input;
use player::{Controls, Player};
use renderer::{Instance, Renderer, UiFrame};
use scene::Scene;
use settings::Settings;
use ui::MenuAction;

/// Radians per mouse unit at sensitivity 1.0.
const MOUSE_SENSITIVITY: f32 = 0.0025;
const TITLE: &str = "OzLand";
/// Effectively the whole playable valley for spells aimed at terrain.
const SPELL_AIM_RANGE: f32 = 1_000.0;
/// Seconds to finish a two-press ability (barrier, rift gate) before it cancels.
const PENDING_TIME: f32 = 12.0;
/// Chrono Echo: how long it waits for an ability, and the tiny gap before
/// that ability happens again.
const ECHO_WINDOW: f32 = 15.0;
const ECHO_DELAY: f32 = 0.12;
/// Overcharge: holding a spell's key past a full cast bar boosts it, up to
/// double over this many seconds, and it goes off by itself this long
/// after that.
const OVERCHARGE_TIME: f32 = 1.5;
const OVERCHARGE_HOLD: f32 = 0.75;
/// Runes: every finished elemental spell is planted as a rune and waits
/// for its key to be pressed again. How fast its overcharge fades once
/// the cast is done (boost lost per second), how long one lies unused
/// before it fizzles, how much stronger each is for every other one going
/// off with it, how close different elements must be to react, and how
/// soon after each other runes set off with different keys still react.
const OVERCHARGE_FADE: f32 = 0.2;
const RUNE_LIFE: f32 = 60.0;
const COMBO_WINDOW: f32 = 1.5;
const RESONANCE: f32 = 0.2;
const REACTION_REACH: f32 = 15.0;

/// A planted spell: where its circle lies (its shots leave from here),
/// what it was aimed at during the cast (self-centred spells: the circle
/// itself), and the way its shots fly.
#[derive(Clone, Copy)]
struct SpellPlace {
    origin: glam::Vec3,
    target: glam::Vec3,
    dir: glam::Vec3,
}

/// Spells that go off around their circle rather than where you aimed.
fn centred(spell: abilities::Skill) -> bool {
    matches!(element(spell), Element::Life) || spell == abilities::Skill::FrostNova
}

/// Spells arcane clones cast too, from where they stand: the ones that
/// strike something.
fn clone_copies(spell: abilities::Skill) -> bool {
    use abilities::Skill::*;
    matches!(spell, Meteor | ChainLightning | Blizzard | Fireball | ArcaneMissiles | ArcaneLance | FrostNova)
}

/// Spells whose shots fly from the circle towards the target.
fn shot_spell(spell: abilities::Skill) -> bool {
    use abilities::Skill::*;
    matches!(spell, Fireball | ArcaneMissiles | ArcaneLance)
}

/// A spell held back, waiting to go off.
struct Rune {
    spell: abilities::Skill,
    boost: f32,
    at: SpellPlace,
    fuse: f32,
}

/// What a spell is made of, for reactions between runes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Element {
    Fire,
    Frost,
    Storm,
    Life,
    None,
}

impl Element {
    /// A rune's glow.
    fn color(self) -> glam::Vec4 {
        match self {
            Element::Fire => glam::Vec4::new(3.0, 1.0, 0.2, 1.0),
            Element::Frost => glam::Vec4::new(0.5, 1.8, 3.2, 1.0),
            Element::Storm => glam::Vec4::new(2.2, 0.8, 3.2, 1.0),
            Element::Life => glam::Vec4::new(0.6, 3.0, 0.8, 1.0),
            Element::None => glam::Vec4::ONE,
        }
    }
}

fn element(spell: abilities::Skill) -> Element {
    use abilities::Skill::*;
    match spell {
        Fireball | Meteor => Element::Fire,
        FrostNova | Blizzard => Element::Frost,
        ChainLightning | ArcaneLance | ArcaneMissiles => Element::Storm,
        Rejuvenate | ArcaneWard | Haste => Element::Life,
        _ => Element::None,
    }
}

/// Spells that can be held back as runes (not the ones that move you,
/// change time, or place things).
fn rune_able(spell: abilities::Skill) -> bool {
    element(spell) != Element::None
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Reaction {
    Steam,
    Superconduct,
    Overload,
    Cataclysm,
}

/// What runes going off together set off: different elements within
/// [`REACTION_REACH`] of each other react (where they meet); all three
/// together make one Cataclysm instead.
fn reactions(runes: &[(Element, glam::Vec3)]) -> Vec<(Reaction, glam::Vec3)> {
    let near = |e: Element| runes.iter().filter(|r| r.0 == e).map(|r| r.1).collect::<Vec<_>>();
    let (fire, frost, storm) = (near(Element::Fire), near(Element::Frost), near(Element::Storm));
    let meet = |a: &[glam::Vec3], b: &[glam::Vec3]| {
        a.iter().flat_map(|p| b.iter().map(move |q| (*p, *q))).find(|(p, q)| p.distance(*q) < REACTION_REACH).map(|(p, q)| (p + q) * 0.5)
    };
    let (steam, superconduct, overload) = (meet(&fire, &frost), meet(&storm, &frost), meet(&fire, &storm));
    if let (Some(a), Some(b), Some(c)) = (steam, superconduct, overload) {
        return vec![(Reaction::Cataclysm, (a + b + c) / 3.0)];
    }
    [(Reaction::Steam, steam), (Reaction::Superconduct, superconduct), (Reaction::Overload, overload)]
        .into_iter()
        .filter_map(|(kind, at)| at.map(|at| (kind, at)))
        .collect()
}

#[cfg(test)]
mod rune_tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn elements_close_together_react() {
        let (here, near, far) = (Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0), Vec3::new(40.0, 0.0, 0.0));
        let fire_frost = reactions(&[(Element::Fire, here), (Element::Frost, near)]);
        assert_eq!(fire_frost, vec![(Reaction::Steam, Vec3::new(5.0, 0.0, 0.0))], "meeting halfway");
        assert!(reactions(&[(Element::Fire, here), (Element::Frost, far)]).is_empty(), "too far apart");
        assert!(reactions(&[(Element::Fire, here), (Element::Fire, near)]).is_empty(), "the same element doesn't react");
        assert!(reactions(&[(Element::Life, here), (Element::Frost, near)]).is_empty(), "healing doesn't react");
        assert_eq!(reactions(&[(Element::Storm, here), (Element::Frost, near)])[0].0, Reaction::Superconduct);
        assert_eq!(reactions(&[(Element::Fire, here), (Element::Storm, near)])[0].0, Reaction::Overload);
        let all = reactions(&[(Element::Fire, here), (Element::Frost, near), (Element::Storm, Vec3::new(5.0, 0.0, 6.0))]);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].0, Reaction::Cataclysm, "all three: one cataclysm, not three reactions");
    }

    #[test]
    fn only_area_spells_become_runes() {
        use abilities::Skill;
        for s in [Skill::Meteor, Skill::Fireball, Skill::FrostNova, Skill::Blizzard, Skill::ChainLightning, Skill::Rejuvenate] {
            assert!(rune_able(s), "{s:?}");
        }
        for s in [Skill::ShadowStep, Skill::RecallHome, Skill::RiftGate, Skill::ChronoEcho, Skill::Dash, Skill::Barrier] {
            assert!(!rune_able(s), "{s:?}");
        }
    }
}

/// Walking speed while casting (a crawl), and with Unshakable Will.
const CAST_WALK: f32 = 0.25;
const CAST_WALK_STEADY: f32 = 0.6;

/// Whirlwind reaches only what's right beside you.
const WHIRLWIND_REACH: f32 = 3.5;

/// Hit-stop lengths (seconds): destroying a drone, a hard ground slam,
/// taking a big hit. Scaled by the screen shake setting (0 turns it off).
const HITSTOP_KILL: f32 = 0.045;
const HITSTOP_SLAM: f32 = 0.07;
const HITSTOP_HURT: f32 = 0.06;

/// The threat level rises every this many seconds played, or every this
/// many drones destroyed.
const THREAT_TIME: f32 = 150.0;
const THREAT_KILLS: u32 = 8;

/// How hard and often drones attack at a threat level: gentle at first,
/// full strength at 3, then a little more each level after.
fn threat_strength(level: u32) -> f32 {
    if level <= 3 { 0.55 + 0.15 * level as f32 } else { (1.0 + 0.08 * (level - 3) as f32).min(2.0) }
}

/// The banner when the threat rises.
fn threat_news(level: u32) -> (String, &'static str) {
    let line = match level {
        1 => "Swarmers and Divers have joined the hunt",
        2 => "Snipers and Mortars are watching from afar",
        3 => "Armoured Brutes are on the move",
        _ => "The drones grow stronger",
    };
    (format!("THREAT LEVEL {level}"), line)
}
/// The "weapon" number that marks a fire message as an ability's blast
/// (254 is Chain Lightning, 255 a ground slam).
const ABILITY_BLAST: u8 = 253;
/// Haste: how much quicker, and for how long.
const HASTE: f32 = 1.35;
fn haste_time(moves: &abilities::Moveset) -> f32 {
    if moves.haste >= 2 { 10.0 } else { 6.0 }
}

/// How long a timed buff skill lasts at its rank.
fn buff_time(skill: abilities::Skill, moves: &abilities::Moveset) -> f32 {
    use abilities::Skill;
    match skill {
        Skill::RapidFire => if moves.rapid_fire >= 2 { 6.0 } else { 4.0 },
        Skill::IronSkin => if moves.iron_skin >= 2 { 7.0 } else { 5.0 },
        Skill::VampiricAura => if moves.vampiric >= 2 { 8.0 } else { 6.0 },
        _ => 1.0,
    }
}

/// Abilities Chrono Echo can repeat: not itself, and not ones placed in
/// two parts or that can't stack (time effects, going home, toggles).
fn echoes(skill: abilities::Skill) -> bool {
    use abilities::Skill::*;
    matches!(
        skill,
        Dash | Pad | Clone | Dome | GiantBox | HealPad | GravityWell | Pull | Repulse | GroundPull | Meteor | ChainLightning | Blizzard | ArcaneWard | Dispel
            | Fireball | ArcaneMissiles | FrostNova | ArcaneLance | Rejuvenate | StunGrenade | Whirlwind | Mend | Stasis | Leap
    )
}

/// After `skill` goes off: if an echo is armed, spend it on repeating
/// `skill` a moment later (`dir`: a dash's direction).
fn arm_echo(charges: &mut u32, repeat: &mut Option<(f32, abilities::Skill, glam::Vec3)>, skill: abilities::Skill, dir: glam::Vec3) {
    if *charges > 0 && repeat.is_none() && echoes(skill) {
        *charges -= 1;
        *repeat = Some((ECHO_DELAY, skill, dir));
    }
}
/// Custom ceilings: how far away you can set a corner, and the smallest side.
const CEILING_REACH: f32 = 30.0;
const CEILING_MIN: f32 = 2.0;
/// Past the last slot (for ordering the hotbar).
const LOADOUT_END: usize = abilities::LOADOUT + 1;

/// Where the game is: the title screen or playing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Title,
    Playing,
}

/// Sound and camera feel: footsteps, head bob, landing dip, speed FOV,
/// hit markers and the red flash when hurt.
#[derive(Default)]
struct Feel {
    /// Distance walked since the last footstep.
    stride: f32,
    bob_phase: f32,
    bob_amount: f32,
    /// Landing dip: a damped spring pulling the camera back up.
    dip: f32,
    dip_velocity: f32,
    /// Extra field of view at speed, in degrees.
    fov_kick: f32,
    /// Hit-stop: seconds the world nearly freezes for, so a kill, a heavy
    /// slam or a big hit taken lands with weight.
    hitstop: f32,
    hit: f32,
    kill: f32,
    damage: f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TimeEffect {
    Bullet,
    Stop,
}

/// Seconds before a skill can be used again.
fn skill_cooldown(skill: abilities::Skill) -> f32 {
    use abilities::Skill::*;
    match skill {
        Ceiling => 12.0,
        Dome | GravityWell => 18.0,
        GiantBox => 45.0,
        HealPad => 20.0,
        SplitShot => 16.0,
        Pull => 10.0,
        Meteor => 20.0,
        ChainLightning => 10.0,
        Blizzard => 24.0,
        ArcaneWard => 25.0,
        Dispel => 16.0,
        ChronoEcho => 18.0,
        RiftGate => 8.0,
        Fireball => 7.0,
        ArcaneMissiles => 12.0,
        FrostNova => 16.0,
        ArcaneLance => 14.0,
        Rejuvenate => 28.0,
        Haste => 24.0,
        ShadowStep => 10.0,
        ArcaneClone => 20.0,
        RapidFire => 18.0,
        StunGrenade => 10.0,
        Whirlwind => 8.0,
        IronSkin => 20.0,
        Mend => 18.0,
        VampiricAura => 24.0,
        Stasis => 18.0,
        Leap => 6.0,
        RecallHome => 40.0,
        Repulse => 12.0,
        GroundPull => 14.0,
        // Movement and utility: short, but enough that none can be spammed.
        Barrier => 10.0,
        Recall => 8.0,
        Rewind => 8.0,
        Clone => 6.0,
        Pad => 5.0,
        Grapple => 3.0,
        LowGravity => 2.0,
        Block => 1.5,
        _ => 1.0,
    }
}

/// The sound of a weapon firing.
fn weapon_sound(weapon: weapons::Weapon) -> audio::Sfx {
    match weapon {
        weapons::Weapon::Blade | weapons::Weapon::Scythe => audio::Sfx::Swing,
        weapons::Weapon::Hammer => audio::Sfx::HeavySwing,
        weapons::Weapon::Blaster => audio::Sfx::Blaster,
        weapons::Weapon::Scatter => audio::Sfx::Scatter,
        weapons::Weapon::Rockets => audio::Sfx::RocketFire,
        weapons::Weapon::Railgun => audio::Sfx::Railgun,
    }
}

/// `--host [port]` / `--join address` (or the WALKABOUT_HOST /
/// WALKABOUT_JOIN environment variables).
fn startup_net_action() -> Option<MenuAction> {
    let args: Vec<String> = std::env::args().collect();
    let after = |flag: &str| args.iter().position(|a| a == flag).map(|i| args.get(i + 1).cloned());
    if let Some(port) = after("--host") {
        let port = port.and_then(|p| p.parse().ok()).unwrap_or(net::DEFAULT_PORT);
        return Some(MenuAction::Host(port));
    }
    if let Some(Some(address)) = after("--join") {
        return Some(MenuAction::Join(address));
    }
    if args.iter().any(|a| a == "--host-steam") || std::env::var_os("WALKABOUT_HOST_STEAM").is_some() {
        return Some(MenuAction::HostSteam);
    }
    if let Ok(port) = std::env::var("WALKABOUT_HOST") {
        return Some(MenuAction::Host(port.parse().unwrap_or(net::DEFAULT_PORT)));
    }
    std::env::var("WALKABOUT_JOIN").ok().map(MenuAction::Join)
}

/// The other player in multiplayer: their robot and latest state.
struct Remote {
    body: body::Body,
    state: Option<net::PlayerState>,
    /// Smoothed position and heading (packets arrive ~30 times a second).
    pos: glam::Vec3,
    yaw: f32,
}

impl Remote {
    fn draw(&mut self, renderer: &mut Renderer, dt: f32) {
        let Some(s) = self.state.filter(|s| s.health > 0) else {
            // Not here, or out of the round.
            self.body.hide(renderer);
            return;
        };
        let k = (dt * 15.0).min(1.0);
        if self.pos.distance(s.pos) > 20.0 {
            self.pos = s.pos; // teleported: don't slide across the map
            self.body.reset_motion();
        } else {
            self.pos += (s.pos - self.pos) * k;
        }
        let turn = (s.yaw - self.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        self.yaw += turn * k;
        self.body.update(renderer, dt, self.pos, self.yaw, s.grounded, s.flight, true);
    }
}

struct Game {
    window: Arc<Window>,
    renderer: Renderer,
    body: body::Body,
    clones: clones::Clones,
    orbs: orbs::Orbs,
    progress: abilities::Progress,
    /// Dash charges ready, and time towards the next one.
    dash_ready: u32,
    dash_timer: f32,
    /// A short on-screen message and how long it has left.
    toast: Option<(String, f32)>,
    /// Camera shake left (from a ground slam).
    shake: f32,
    gadgets: gadgets::Gadgets,
    /// The multiplayer fight: rounds, score, health.
    arena: arena::Arena,
    health: f32,
    /// Out of the current round.
    dead: bool,
    /// Damage taken this frame, applied once.
    pending_damage: f32,
    /// Host: where the joiner spawns this round.
    joiner_spawn: glam::Vec3,
    /// For random spawns.
    rng: u32,
    /// Checks for and installs updates (released copies only).
    updater: Option<updater::Updater>,
    /// The "update installed" notice has been shown.
    update_announced: bool,
    /// Multiplayer: the connection (hosting or joined), and the other player.
    net: Option<net::Net>,
    /// Steam (as Spacewar, App ID 480), when it's running: internet play
    /// with no router setup. See `steam.rs`.
    steam: Option<steam::Steam>,
    /// Hosting through Steam: online friends (refreshed every few seconds
    /// while the menu is open) and who you've invited.
    steam_friends: Vec<steam::SteamFriend>,
    friends_refresh: f32,
    invited: Vec<u64>,
    remote: Remote,
    lan_ip: Option<String>,
    /// The pause menu's tab and selected tree.
    menu: ui::MenuState,
    weapons: weapons::Weapons,
    drones: drones::Drones,
    /// Where the grapple is hooked.
    hook: Option<glam::Vec3>,
    /// The recall marker (B).
    marker: Option<glam::Vec3>,
    /// Recent positions (time, place) for Rewind.
    trail: std::collections::VecDeque<(f32, glam::Vec3)>,
    play_time: f32,
    rewind_cooldown: f32,
    /// Bullet time or time stop, and seconds left.
    time_effect: Option<(TimeEffect, f32)>,
    time_cooldown: f32,
    /// Clock for the world (shaders, wind): slows and stops with Time powers.
    world_time: f32,
    /// Where the player was last frame (for experience).
    last_pos: glam::Vec3,
    /// V toggles a camera behind the player (the body becomes fully visible).
    third_person: bool,
    scene: Scene,
    camera: Camera,
    player: Player,
    input: Input,
    mouse_captured: bool,
    /// The Escape menu is open; the game is paused.
    menu_open: bool,
    settings: Settings,
    /// Settings as last applied to the renderer/camera, and as last saved.
    applied: Option<Settings>,
    saved: Settings,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    last_frame: Instant,
    fps: f32,
    fps_timer: f32,
    fps_frames: u32,
    frame_count: u32,
    /// Dev aid: WALKABOUT_SCREENSHOT=file.png saves a frame there and quits.
    screenshot: Option<std::path::PathBuf>,
    audio: audio::Audio,
    screen: Screen,
    /// Seconds on the title screen (for its animation and camera).
    title_time: f32,
    /// Black over the screen (fading in at start and between screens), and
    /// a switch to playing waiting for the fade to finish.
    fade: f32,
    starting: bool,
    feel: Feel,
    banner: Option<ui::Banner>,
    /// The toast last shown and how long ago (for its animation and sound).
    toast_shown: String,
    toast_age: f32,
    /// The first-time controls card is up, and for how long.
    /// The first-time tutorial, while it runs.
    tutorial: Option<tutorial::Tutorial>,
    /// The threat level (drones get more varied and dangerous): seconds
    /// played and drones destroyed since it was last reset, and the level.
    threat_time: f32,
    threat_kills: u32,
    threat: u32,
    /// Real seconds since start.
    clock: f32,
    /// The match phase last frame and the countdown second last beeped.
    last_phase: arena::Phase,
    last_beep: i32,
    vitals: Vitals,
    /// Damage drones did to you this frame (applied with PvP damage).
    pve_damage: f32,
    /// Floating damage numbers.
    popups: Vec<FloatText>,
    /// Barriers, ceilings and domes (yours and the other player's).
    structures: structures::Structures,
    /// The scene's colliders plus the structures' boxes.
    colliders: Vec<collision::Aabb>,
    /// Seconds until you can build another barrier, ceiling, dome.
    cooldowns: [f32; abilities::SKILLS],
    /// First endpoint selected for a custom two-point barrier.
    barrier_first: Option<glam::Vec3>,
    /// Seconds left to finish a half-placed barrier or rift gate before
    /// it's cancelled, and the previews drawn meanwhile.
    barrier_wait: f32,
    rift_wait: f32,
    /// First corner of a custom ceiling (at the slab's height).
    ceiling_first: Option<glam::Vec3>,
    ceiling_wait: f32,
    preview: placement::Preview,
    /// A left click just confirmed a placement: no firing until it's let go.
    click_used: bool,
    /// Block: guarding now, and for how long (parries need it fresh).
    blocking: bool,
    block_time: f32,
    /// Split Shot: seconds left.
    split_left: f32,
    /// Time spent charging the Keen Edge blade cone.
    blade_charge: f32,
    /// A spell being cast: (which, seconds so far, seconds needed).
    casting: Option<(abilities::Skill, f32, f32)>,
    /// Where the spell being cast is planted (its circle).
    cast_origin: glam::Vec3,
    /// Last local attack and a queued delayed echo of it.
    /// Chrono Echo: abilities left that will happen twice, seconds before
    /// the echo fades, and the repeat about to go off (with a dash's
    /// direction).
    echo_charges: u32,
    echo_left: f32,
    echo_repeat: Option<(f32, abilities::Skill, glam::Vec3)>,
    /// Haste: seconds of quickness left.
    haste_left: f32,
    /// The spell going off now: how boosted (overcharge, resonance) and,
    /// for a rune, where it goes off instead of your crosshair.
    spell_boost: f32,
    spell_at: Option<SpellPlace>,
    /// An arcane clone casting your spell: where it stands.
    spell_from: Option<glam::Vec3>,
    /// Spells held back as runes, waiting to go off.
    runes: Vec<Rune>,
    /// Runes set off in the last moments (element, where, seconds ago), so
    /// ones set off with different keys close together still react.
    recent_runes: Vec<(Element, glam::Vec3, f32)>,
    rune_marks: placement::RuneMarks,
    /// Craters the other player blasted, to dig this frame.
    their_craters: Vec<(glam::Vec3, f32, f32)>,
    /// Seconds left on each timed buff skill (Rapid Fire, Iron Skin,
    /// Vampiric Aura), by skill.
    buff_left: [f32; abilities::SKILLS],
    rift_gates: [Option<net::GatePair>; 2],
    rift_first: Option<net::GatePoint>,
    /// Time to the next Blizzard damage tick.
    storm_tick: f32,
    /// Drones destroyed outside the weapons this frame (Repulse), counted next frame.
    extra_kills: u32,
    /// Trials completed and healing, applied once the frame's moves are done.
    unlocked: Vec<abilities::Unlock>,
    heal_queue: Vec<(f32, bool)>,
    /// A key or button pressed while binding a skill (None inside: cancelled).
    captured_bind: Option<Option<input::Bind>>,
    /// A key or button pressed on the Skills tab: put the chosen skill on it.
    assign_bind: Option<input::Bind>,
    /// Damage to structures waiting to be applied: (id, amount).
    struct_damage: Vec<(u32, f32)>,
}

/// Health beyond the number: shield, regeneration, the bar's trail.
#[derive(Default)]
struct Vitals {
    shield: f32,
    /// Seconds since last hurt (regeneration waits for this).
    since_hurt: f32,
    /// Where the bar's pale "just lost" trail is.
    trail: f32,
    /// Green flash when healed.
    heal_flash: f32,
    invulnerable: f32,
    /// Seconds until Second Wind can save you again.
    second_wind: f32,
    /// Downed outside a match: seconds until you're back on your feet.
    downed: Option<f32>,
    /// Arcane Ward: magic barrier points, and seconds left.
    ward: f32,
    ward_time: f32,
}

/// A number floating up from a hit.
struct FloatText {
    pos: glam::Vec3,
    text: String,
    color: egui::Color32,
    age: f32,
    big: bool,
}

impl Game {
    fn new(event_loop: &ActiveEventLoop) -> Result<Self> {
        let screenshot = std::env::var_os("WALKABOUT_SCREENSHOT").map(std::path::PathBuf::from);
        let window = Arc::new(
            event_loop.create_window(
                Window::default_attributes()
                    .with_title(TITLE)
                    .with_inner_size(LogicalSize::new(1280.0, 720.0))
                    // Don't steal focus when only taking a screenshot.
                    .with_active(screenshot.is_none()),
            )?,
        );
        let mut renderer = Renderer::new(window.clone())?;

        let started = Instant::now();
        let scene = scene::build();
        let terrain_mesh = scene.terrain.mesh();
        renderer.set_terrain(&scene.terrain_data(&terrain_mesh));
        drop(terrain_mesh);
        for part in &scene.static_parts {
            renderer.add_mesh(&part.mesh, &part.material, &[Instance::IDENTITY]);
        }
        for placed in &scene.models {
            for part in &placed.model.parts {
                renderer.add_mesh(&part.mesh, &part.material, &placed.instances);
            }
        }
        // WALKABOUT_GRASS=0 turns the 3D grass off (e.g. on slow GPUs).
        let grass_enabled = std::env::var("WALKABOUT_GRASS").map_or(true, |v| v != "0");
        if let Some(grass) = scene.grass.as_ref().filter(|_| grass_enabled) {
            renderer.set_grass(grass);
        }
        log::info!("scene built and uploaded in {:.2?}", started.elapsed());

        let character = body::Body::load_model();
        let mut body = body::Body::new(&mut renderer, character.clone());
        let mut remote_body = body::Body::new(&mut renderer, character.clone());
        body.set_color(net::PLAYER_COLORS[0]);
        remote_body.set_color(net::PLAYER_COLORS[1]);
        let mut clones = clones::Clones::new(&mut renderer, character);
        clones.set_color(net::PLAYER_COLORS[0]);
        let progress = abilities::Progress::load();
        let gadgets = gadgets::Gadgets::new(&mut renderer);
        let weapons = weapons::Weapons::new(&mut renderer);
        let mut drones = drones::Drones::new(&mut renderer, &scene.terrain, scene.spawn);
        drones.walls = scene.colliders.clone();
        let preview = placement::Preview::new(&mut renderer);
        let rune_marks = placement::RuneMarks::new(&mut renderer);
        let orbs = orbs::Orbs::new(&mut renderer, scene.crystals.clone(), &progress.found);
        let structures = structures::Structures::new(&mut renderer);
        let size = window.inner_size();
        let aspect = size.width.max(1) as f32 / size.height.max(1) as f32;
        let mut player = Player::new(scene.spawn);
        let mut camera = Camera::new(player.eye(), scene.spawn_yaw, aspect);

        // Dev aid: WALKABOUT_VIEW="x,z,yaw_degrees,pitch_degrees" starts at a
        // fixed viewpoint and leaves the mouse free (handy for screenshots).
        let fixed_view = std::env::var("WALKABOUT_VIEW").ok().and_then(|v| {
            let n: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
            (n.len() == 4).then(|| (n[0], n[1], n[2], n[3]))
        });
        if let Some((x, z, yaw, pitch)) = fixed_view {
            player = Player::new(glam::Vec3::new(x, scene.terrain.height_at(x, z), z));
            camera.yaw = yaw.to_radians();
            camera.pitch = pitch.to_radians();
            camera.position = player.eye();
        }

        let egui_ctx = egui::Context::default();
        ui::apply_theme(&egui_ctx);
        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );

        let settings = Settings::load();
        let mut game = Self {
            window,
            renderer,
            body,
            clones,
            orbs,
            progress,
            dash_ready: u32::MAX, // clamped to the charges available: starts full
            gadgets,
            arena: arena::Arena::new(),
            health: arena::MAX_HEALTH,
            dead: false,
            pending_damage: 0.0,
            joiner_spawn: glam::Vec3::ZERO,
            rng: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0x2545_f491, |d| d.subsec_nanos() | 1),
            updater: updater::Updater::start(),
            update_announced: false,
            net: None,
            steam: steam::Steam::init(),
            steam_friends: Vec::new(),
            friends_refresh: 0.0,
            invited: Vec::new(),
            remote: Remote { body: remote_body, state: None, pos: glam::Vec3::ZERO, yaw: 0.0 },
            lan_ip: net::lan_address(),
            // Dev aid: WALKABOUT_MENU=<tree name> opens the menu on that tree.
            menu: ui::MenuState {
                selected: std::env::var("WALKABOUT_MENU")
                    .ok()
                    .and_then(|v| abilities::Skill::ALL.into_iter().find(|t| t.name().eq_ignore_ascii_case(&v)))
                    .unwrap_or(abilities::Skill::Dash),
                tab: match std::env::var("WALKABOUT_MENU").as_deref() {
                    Ok("settings") => ui::Tab::Settings,
                    Ok("items") => ui::Tab::Items,
                    Ok("multiplayer") => ui::Tab::Multiplayer,
                    _ => ui::Tab::Abilities,
                },
                ..Default::default()
            },
            weapons,
            drones,
            hook: None,
            marker: None,
            trail: std::collections::VecDeque::new(),
            play_time: 0.0,
            rewind_cooldown: 0.0,
            time_effect: None,
            time_cooldown: 0.0,
            world_time: 0.0,
            last_pos: player.pos,
            dash_timer: 0.0,
            toast: None,
            shake: 0.0,
            third_person: std::env::var_os("WALKABOUT_THIRD_PERSON").is_some(),
            scene,
            camera,
            player,
            input: Input::default(),
            mouse_captured: false,
            // Dev aid: WALKABOUT_MENU=1 starts with the menu open.
            menu_open: std::env::var_os("WALKABOUT_MENU").is_some(),
            saved: settings.clone(),
            settings,
            applied: None,
            egui_ctx,
            egui_state,
            last_frame: Instant::now(),
            fps: 0.0,
            fps_timer: 0.0,
            fps_frames: 0,
            frame_count: 0,
            screenshot,
            audio: audio::Audio::new(),
            screen: Screen::Title,
            title_time: 0.0,
            fade: 1.0,
            starting: false,
            feel: Feel::default(),
            banner: None,
            toast_shown: String::new(),
            toast_age: 0.0,
            tutorial: None,
            threat_time: 0.0,
            threat_kills: 0,
            threat: 0,
            clock: 0.0,
            last_phase: arena::Arena::new().state.phase,
            last_beep: -1,
            vitals: Vitals { trail: arena::MAX_HEALTH, ..Default::default() },
            pve_damage: 0.0,
            popups: Vec::new(),
            structures,
            colliders: Vec::new(),
            cooldowns: [0.0; abilities::SKILLS],
            barrier_first: None,
            barrier_wait: 0.0,
            rift_wait: 0.0,
            ceiling_first: None,
            ceiling_wait: 0.0,
            preview,
            click_used: false,
            blocking: false,
            block_time: 0.0,
            split_left: 0.0,
            blade_charge: 0.0,
            casting: None,
            cast_origin: glam::Vec3::ZERO,
            echo_charges: 0,
            echo_left: 0.0,
            echo_repeat: None,
            haste_left: 0.0,
            spell_boost: 1.0,
            spell_at: None,
            spell_from: None,
            runes: Vec::new(),
            recent_runes: Vec::new(),
            rune_marks,
            their_craters: Vec::new(),
            buff_left: [0.0; abilities::SKILLS],
            rift_gates: [None, None],
            rift_first: None,
            storm_tick: 0.0,
            extra_kills: 0,
            unlocked: Vec::new(),
            heal_queue: Vec::new(),
            struct_damage: Vec::new(),
            captured_bind: None,
            assign_bind: None,
        };
        game.colliders = game.scene.colliders.clone();
        // Dev aid: WALKABOUT_STRUCTURES=1 puts a barrier, ceiling and dome
        // in front of the start (for screenshots).
        if std::env::var_os("WALKABOUT_STRUCTURES").is_some() {
            let (p, yaw) = (game.player.pos, game.camera.yaw);
            let f = glam::Vec3::new(yaw.cos(), 0.0, yaw.sin());
            let side = glam::Vec3::new(-f.z, 0.0, f.x);
            let at = |d: glam::Vec3| {
                let q = p + d;
                glam::Vec3::new(q.x, game.scene.terrain.height_at(q.x, q.z), q.z)
            };
            for (i, (kind, pos)) in [
                (structures::Kind::Barrier, at(f * 10.0 - side * 12.0)),
                (structures::Kind::Ceiling, at(f * 12.0) + glam::Vec3::Y * 6.0),
                (structures::Kind::Dome, at(f * 14.0 + side * 14.0)),
                (structures::Kind::Box, at(f * 45.0)),
            ]
            .into_iter()
            .enumerate()
            {
                let id = game.structures.new_id(0);
                let health = if i == 0 { 30.0 } else { 100.0 };
                game.structures.add(structures::Structure { id, kind, pos, yaw, health, max_health: 100.0, life: 1e6, warden: false, scale: 1.0, power: 0.0 });
            }
        }
        game.apply_settings();
        game.health = game.moves().max_health;
        game.vitals.trail = game.health;
        // Multiplayer from the command line: `--host [port]` or `--join address`
        // (or WALKABOUT_HOST / WALKABOUT_JOIN).
        let net_action = startup_net_action();
        if let Some(action) = &net_action {
            game.start_net(action);
        }
        // Dev views and command-line multiplayer skip the title screen
        // (WALKABOUT_TITLE=1 keeps it, e.g. for a screenshot of it).
        let dev_start = fixed_view.is_some() || game.screenshot.is_some() || game.menu_open || net_action.is_some();
        if dev_start && std::env::var_os("WALKABOUT_TITLE").is_none() {
            game.screen = Screen::Playing;
            game.fade = 0.0;
            let capture = fixed_view.is_none() && game.screenshot.is_none() && !game.menu_open;
            game.set_mouse_captured(capture);
        } else {
            game.set_mouse_captured(false);
        }
        if game.screenshot.is_some() {
            game.fade = 0.0;
        }
        Ok(game)
    }

    /// Leaves the title screen for the valley.
    fn start_playing(&mut self) {
        self.screen = Screen::Playing;
        self.starting = false;
        self.camera.yaw = self.scene.spawn_yaw;
        self.camera.pitch = 0.0;
        self.camera.position = self.player.eye();
        self.menu_open = false;
        self.set_mouse_captured(true);
        self.input.release_all();
        if !self.settings.seen_intro {
            self.tutorial = Some(tutorial::Tutorial::new());
        }
        self.banner = Some(ui::Banner::new("OzLand", "The valley awaits", ui::BANNER_GOLD));
    }

    /// Back to the title screen.
    fn to_title(&mut self) {
        self.progress.save();
        self.save_settings_if_changed();
        if self.net.is_some() {
            self.stop_net();
        }
        self.menu_open = false;
        self.screen = Screen::Title;
        self.title_time = 0.0;
        self.fade = 1.0;
        self.banner = None;
        self.toast = None;
        self.tutorial = None;
        self.set_mouse_captured(false);
    }

    /// The camera on the title screen: a slow circle over the village.
    fn title_camera(&mut self) {
        let t = self.title_time * 0.035 + 0.6;
        let center = self.scene.spawn + glam::Vec3::Y * 3.0;
        let radius = 38.0 + 6.0 * (t * 1.7).sin();
        let mut pos = center + glam::Vec3::new(t.cos() * radius, 14.0 + 3.0 * (t * 1.3).sin(), t.sin() * radius);
        let ground = self.scene.terrain.height_at(pos.x, pos.z).max(self.scene.terrain.water_level);
        pos.y = pos.y.max(ground + 4.0);
        let look = (center - pos).normalize();
        self.camera.position = pos;
        self.camera.yaw = look.z.atan2(look.x);
        self.camera.pitch = look.y.asin();
        self.camera.fov_y = self.settings.fov_degrees.to_radians();
    }

    /// What's worth doing next, for the goals list.
    fn goals(&self, moves: &abilities::Moveset) -> Vec<String> {
        let mut goals = Vec::new();
        // How long until the drones get tougher.
        let next = self.threat + 1;
        let secs = (next as f32 * THREAT_TIME - self.threat_time).max(0.0) as u32;
        let kills = (next * THREAT_KILLS).saturating_sub(self.threat_kills);
        goals.push(format!("Threat {}: rises in {}:{:02} or {kills} more drones", self.threat, secs / 60, secs % 60));
        let p = &self.progress;
        if p.points > 0 {
            let s = if p.points == 1 { "" } else { "s" };
            goals.push(format!("Spend {} skill point{s}: press Esc", p.points));
        }
        if (0..items::ITEMS.len()).any(|i| p.item_blocker(i).is_none()) {
            goals.push("New gear ready: Esc > Items".into());
        }
        let (found, total) = (p.found.len(), self.orbs.total());
        if found < total {
            goals.push(format!("Find the skill crystals ({found}/{total})"));
        }
        if moves.armed() && goals.len() < 3 {
            goals.push(format!("Destroy practice drones ({} so far)", self.weapons.kills));
        } else if goals.len() < 2 && p.nodes() == 0 {
            goals.push("Run and jump to earn experience".into());
        }
        goals.truncate(4);
        goals
    }

    /// The tutorial saw you do something (a chime when a step's done).
    fn tutorial_note(&mut self, did: tutorial::Did) {
        if self.tutorial.as_mut().is_some_and(|t| t.note(did)) {
            self.audio.play(audio::Sfx::Toast);
        }
    }

    /// The threat level due now: one up every 2.5 minutes played or every
    /// 8 drones destroyed, whichever comes first; in a match, at least one
    /// up per round played.
    fn threat_level(&self) -> u32 {
        let by_time = (self.threat_time / THREAT_TIME) as u32;
        let by_kills = self.threat_kills / THREAT_KILLS;
        let by_rounds = if self.arena.fighting() || self.net.is_some() { self.arena.state.round.saturating_sub(1) } else { 0 };
        by_time.max(by_kills).max(by_rounds)
    }

    /// A big announcement, with its sound.
    fn announce(&mut self, title: &str, subtitle: impl Into<String>, color: egui::Color32, sound: audio::Sfx) {
        self.banner = Some(ui::Banner::new(title, subtitle, color));
        self.audio.play(sound);
    }

    /// Camera shake, scaled by the setting.
    fn add_shake(&mut self, amount: f32) {
        self.shake = self.shake.max(amount * self.settings.screen_shake);
    }

    /// Pushes changed settings into the camera, lighting and renderer.
    fn apply_settings(&mut self) {
        if self.applied.as_ref() == Some(&self.settings) {
            return;
        }
        let s = &self.settings;
        self.camera.fov_y = s.fov_degrees.to_radians();

        let lighting = &mut self.scene.lighting;
        lighting.exposure = s.brightness;
        lighting.fog_end = s.view_distance;
        lighting.fog_start = s.view_distance * 0.19;
        lighting.sun_direction = s.sun_direction();
        lighting.sun_color = s.sun_color();
        let sun_moved = self
            .applied
            .as_ref()
            .is_none_or(|a| a.sun_elevation != s.sun_elevation || a.sun_azimuth != s.sun_azimuth);
        if sun_moved {
            self.renderer.invalidate_shadows();
        }

        self.renderer
            .set_grass_options(s.grass, s.grass_distance, s.grass_density, s.wind);
        self.renderer.set_surface_detail(s.surface_detail);
        self.renderer.set_vsync(s.vsync);
        self.applied = Some(s.clone());
    }

    /// What the character can do right now.
    fn moves(&self) -> abilities::Moveset {
        let mut m = if self.settings.unlock_all {
            // Sandbox: whatever you equip works at its top rank; every item.
            let equipped: Vec<(abilities::Skill, u32)> = self.progress.loadout().iter().map(|&s| (s, s.max_rank())).collect();
            abilities::Moveset::build(&equipped, &items::all(), self.settings.sandbox_ascension as u32)
        } else {
            let p = &self.progress;
            abilities::Moveset::build(&p.equipped_ranks(), p.items(), p.ascension)
        };
        // Haste: everything quicker for a while.
        if self.haste_left > 0.0 {
            m.speed *= HASTE;
            m.fire_rate *= HASTE;
            m.melee_speed *= HASTE;
        }
        {
            use abilities::Skill;
            let on = |s: Skill| self.buff_left[s.index()] > 0.0;
            if on(Skill::RapidFire) {
                m.fire_rate *= 1.8;
                m.melee_speed *= 1.8;
            }
            if on(Skill::IronSkin) {
                m.armor *= if m.iron_skin >= 2 { 0.35 } else { 0.5 };
            }
            if on(Skill::VampiricAura) {
                let drain = if m.vampiric >= 2 { 0.5 } else { 0.3 };
                m.lifesteal += drain;
                m.melee_lifesteal += drain;
            }
        }
        // Adrenaline: stronger and faster when hurt (always, as a Berserker).
        if m.adrenaline && (m.berserker || self.health < m.max_health * 0.35) {
            m.speed *= 1.2;
            m.weapon_damage *= 1.3;
        }
        m
    }

    /// A skill's tile while it recharges (or Ready).
    fn cooling_state(&self, s: abilities::Skill) -> ui::SlotState {
        let (left, full) = (self.cooldowns[s.index()], skill_cooldown(s));
        if left > 0.0 { ui::SlotState::Cooldown((left / full).min(1.0)) } else { ui::SlotState::Ready }
    }

    /// The HUD's model: an ability tile for everything you can trigger, the
    /// weapon bar, and movement meters.
    fn hud_parts(&self, moves: &abilities::Moveset) -> (Vec<ui::Slot>, Vec<ui::WeaponSlot>, ui::Meters) {
        use ui::{Slot, SlotState};
        let p = &self.player;
        let mut slots = Vec::new();
        let binds = self.binds();
        let key = |s: abilities::Skill| binds[s.index()].map_or_else(String::new, |b| b.label());
        let mut slot = |key: String, name: &str, state: SlotState, charges: Option<(u32, u32)>| {
            slots.push(Slot { key, name: name.to_owned(), state, charges });
        };
        if moves.dash_charges > 0 && self.progress.equipped(abilities::Skill::Dash) {
            let ready = self.dash_ready.min(moves.dash_charges);
            let state = if ready > 0 {
                SlotState::Ready
            } else {
                SlotState::Cooldown(1.0 - self.dash_timer / moves.dash_recharge)
            };
            slot(key(abilities::Skill::Dash), "Dash", state, Some((ready, moves.dash_charges)));
        }
        if moves.grapple_range > 0.0 && self.progress.equipped(abilities::Skill::Grapple) {
            let state = if p.hooked() { SlotState::Active(1.0) } else { self.cooling_state(abilities::Skill::Grapple) };
            slot(key(abilities::Skill::Grapple), "Grapple", state, None);
        }
        if moves.pads > 0 && self.progress.equipped(abilities::Skill::Pad) {
            let placed = self.gadgets.pad_count() as u32;
            slot(key(abilities::Skill::Pad), "Pad", self.cooling_state(abilities::Skill::Pad), Some((placed, moves.pads as u32)));
        }
        if moves.max_clones > 0 && self.progress.equipped(abilities::Skill::Clone) {
            let out = self.clones.count_of(clones::Kind::Mirror) as u32;
            slot(key(abilities::Skill::Clone), "Clone", self.cooling_state(abilities::Skill::Clone), Some((out, moves.max_clones as u32)));
            if moves.switch_places {
                let state = if out > 0 { SlotState::Ready } else { SlotState::Unavailable };
                slot("X".into(), "Swap", state, None);
            }
        }
        if moves.recall && self.progress.equipped(abilities::Skill::Recall) {
            let state = if self.marker.is_some() { SlotState::Active(1.0) } else { self.cooling_state(abilities::Skill::Recall) };
            slot(key(abilities::Skill::Recall), if self.marker.is_some() { "Recall" } else { "Marker" }, state, None);
        }
        if moves.rewind > 0.0 && self.progress.equipped(abilities::Skill::Rewind) {
            let state = if self.rewind_cooldown > 0.0 {
                SlotState::Cooldown(self.rewind_cooldown / skill_cooldown(abilities::Skill::Rewind))
            } else {
                SlotState::Ready
            };
            slot(key(abilities::Skill::Rewind), "Rewind", state, None);
        }
        if moves.bullet_time && self.progress.equipped(abilities::Skill::TimeWarp) {
            let (name, length) = if moves.time_stop { ("Stop", moves.time_stop_length) } else { ("Slow-mo", 5.0) };
            let state = match self.time_effect {
                Some((_, left)) => SlotState::Active(left / length),
                None if self.time_cooldown > 0.0 => SlotState::Cooldown(self.time_cooldown / moves.time_cooldown),
                None => SlotState::Ready,
            };
            slot(key(abilities::Skill::TimeWarp), name, state, None);
        }
        if moves.low_gravity && self.progress.equipped(abilities::Skill::LowGravity) {
            let state = if self.cooldowns[abilities::Skill::LowGravity.index()] > 0.0 {
                self.cooling_state(abilities::Skill::LowGravity)
            } else if p.low_gravity() {
                SlotState::Active(1.0)
            } else {
                SlotState::Ready
            };
            slot(key(abilities::Skill::LowGravity), "Low grav", state, None);
        }
        if (moves.free_flight || moves.hover_time > 0.0) && self.progress.equipped(abilities::Skill::Hover) {
            let name = if moves.free_flight {
                "Fly"
            } else if moves.levitate {
                "Levitate"
            } else {
                "Hover"
            };
            let state = if moves.free_flight && p.flight_cooldown() > 0.0 {
                SlotState::Cooldown(p.flight_cooldown() / 30.0)
            } else if moves.free_flight && p.flying() {
                SlotState::Active(p.flight_left() / 60.0)
            } else if moves.free_flight || p.hover_left() > 0.05 || p.grounded() {
                SlotState::Ready
            } else {
                SlotState::Unavailable
            };
            slot(key(abilities::Skill::Hover), name, state, None);
        }

        // Building and control skills, with their cooldowns.
        use abilities::Skill;
        let cooling = |s: Skill| {
            let (left, full) = (self.cooldowns[s.index()], skill_cooldown(s));
            if left > 0.0 { SlotState::Cooldown(left / full) } else { SlotState::Ready }
        };
        if moves.block > 0.0 && self.progress.equipped(Skill::Block) {
            let state = if self.blocking { SlotState::Active(1.0) } else { cooling(Skill::Block) };
            slots.push(Slot { key: key(Skill::Block), name: if moves.parry { "Parry" } else { "Block" }.into(), state, charges: None });
        }
        if moves.split > 0 && self.progress.equipped(Skill::SplitShot) {
            let state = if self.split_left > 0.0 { SlotState::Active(self.split_left / moves.split_time) } else { cooling(Skill::SplitShot) };
            slots.push(Slot { key: key(Skill::SplitShot), name: "Split".into(), state, charges: None });
        }
        if moves.barriers > 0 && self.progress.equipped(Skill::Barrier) {
            let up = self.structures.mine(structures::Kind::Barrier, self.me()).len() as u32;
            let state = if self.barrier_first.is_some() { SlotState::Active(1.0) } else { cooling(Skill::Barrier) };
            let name = if self.barrier_first.is_some() { "Corner set" } else { "Barrier" };
            slots.push(Slot { key: key(Skill::Barrier), name: name.into(), state, charges: Some((up, moves.barriers)) });
        }
        for (have, skill, _, name) in [
            (moves.ceiling, Skill::Ceiling, "J", "Ceiling"),
            (moves.dome, Skill::Dome, "K", "Dome"),
            (moves.giant_box > 0, Skill::GiantBox, "", "Giant box"),
            (moves.heal_pad > 0.0, Skill::HealPad, "I", "Heal pad"),
            (moves.pull > 0.0, Skill::Pull, "L", "Pull"),
            (moves.repulse > 0.0, Skill::Repulse, "O", "Repulse"),
            (moves.ground_pull > 0.0, Skill::GroundPull, "U", "Grnd pull"),
            (moves.gravity_well > 0.0, Skill::GravityWell, "Y", "Well"),
            (moves.meteors > 0, Skill::Meteor, "", "Meteor"),
            (moves.lightning_jumps > 0, Skill::ChainLightning, "", "Lightning"),
            (moves.blizzard > 0, Skill::Blizzard, "", "Blizzard"),
            (moves.ward > 0.0, Skill::ArcaneWard, "", "Ward"),
            (moves.recall_home, Skill::RecallHome, "", "Home"),
            (moves.dispel > 0, Skill::Dispel, "", "Dispel"),
            (moves.chrono_echo > 0, Skill::ChronoEcho, "", "Echo"),
            (moves.rift_gate, Skill::RiftGate, "", "Rift"),
            (moves.fireball > 0, Skill::Fireball, "", "Fireball"),
            (moves.missiles > 0, Skill::ArcaneMissiles, "", "Missiles"),
            (moves.frost_nova > 0, Skill::FrostNova, "", "Nova"),
            (moves.lance > 0, Skill::ArcaneLance, "", "Lance"),
            (moves.rejuvenate > 0, Skill::Rejuvenate, "", "Rejuv"),
            (moves.haste > 0, Skill::Haste, "", "Haste"),
            (moves.shadow_step > 0, Skill::ShadowStep, "", "Shadow"),
            (moves.arcane_clones > 0, Skill::ArcaneClone, "", "Arcane"),
            (moves.rapid_fire > 0, Skill::RapidFire, "", "Rapid"),
            (moves.stun_grenade > 0, Skill::StunGrenade, "", "Grenade"),
            (moves.whirlwind > 0, Skill::Whirlwind, "", "Whirl"),
            (moves.iron_skin > 0, Skill::IronSkin, "", "Iron"),
            (moves.mend > 0, Skill::Mend, "", "Mend"),
            (moves.vampiric > 0, Skill::VampiricAura, "", "Vampire"),
            (moves.stasis > 0, Skill::Stasis, "", "Stasis"),
            (moves.leap > 0, Skill::Leap, "", "Leap"),
        ] {
            if have && self.progress.equipped(skill) {
                let state = match self.casting {
                    Some((s, t, total)) if s == skill => SlotState::Active((1.0 - t / total).max(0.0)),
                    _ if skill == Skill::ChronoEcho && self.echo_charges > 0 => SlotState::Active(self.echo_left / ECHO_WINDOW),
                    _ if skill == Skill::Haste && self.haste_left > 0.0 => SlotState::Active(self.haste_left / haste_time(moves)),
                    _ if self.buff_left[skill.index()] > 0.0 => SlotState::Active(self.buff_left[skill.index()] / buff_time(skill, moves)),
                    _ => cooling(skill),
                };
                slots.push(Slot { key: key(skill), name: name.into(), state, charges: None });
            }
        }

        // The hotbar follows your slots, left to right (clone keys M and X
        // sit just after the clone's slot).
        let slot_labels: Vec<String> = self.slot_binds().iter().map(|b| b.label()).collect();
        let clone_at = slot_labels.iter().position(|l| *l == key(abilities::Skill::Clone)).unwrap_or(LOADOUT_END);
        let order = |s: &ui::Slot| match s.key.as_str() {
            "M" => clone_at as f32 + 0.3,
            "X" => clone_at as f32 + 0.6,
            k => slot_labels.iter().position(|l| l == k).unwrap_or(LOADOUT_END) as f32,
        };
        slots.sort_by(|a, b| order(a).total_cmp(&order(b)));

        let weapons = weapons::Weapon::ALL
            .into_iter()
            .map(|w| ui::WeaponSlot {
                weapon: w,
                unlocked: moves.has_weapon(w),
                selected: self.weapons.current == w,
                ready: self.weapons.ready_fraction(moves),
            })
            .collect();

        let mut meters = ui::Meters {
            speed: p.velocity().length(),
            momentum: (moves.momentum_cap > 1.0 && p.momentum() > 1.05).then(|| p.momentum()),
            ..Default::default()
        };
        if moves.hover_time > 0.0 && !moves.free_flight && p.hover_left() < moves.hover_time - 0.01 {
            let label = if moves.levitate { "Levitate" } else { "Hover" };
            meters.energy = Some((label, p.hover_left() / moves.hover_time));
        }
        if moves.free_flight {
            let cooldown = p.flight_cooldown();
            meters.energy = Some((
                if cooldown > 0.0 { "Flight cooldown" } else { "Flight" },
                if cooldown > 0.0 { 1.0 - cooldown / 30.0 } else { p.flight_left() / 60.0 },
            ));
        }
        if moves.glide && !p.grounded() {
            meters.rockets = Some((moves.boosts.saturating_sub(p.boosts_used()), moves.boosts));
        }
        if p.wall_running() {
            meters.status.push("Wall run");
        }
        if p.low_gravity() {
            meters.status.push("Low gravity");
        }
        if p.pinned() {
            meters.status.push("Pinned");
        }
        match self.time_effect.map(|(e, _)| e) {
            Some(TimeEffect::Bullet) => meters.status.push("Bullet time"),
            Some(TimeEffect::Stop) => meters.status.push("Time stopped"),
            None => {}
        }
        (slots, weapons, meters)
    }
    fn notify(&mut self, message: impl Into<String>) {
        self.toast = Some((message.into(), 3.0));
    }

    fn save_settings_if_changed(&mut self) {
        if self.settings != self.saved {
            self.settings.save();
            self.saved = self.settings.clone();
        }
    }

    fn open_menu(&mut self) {
        self.progress.save();
        self.menu_open = true;
        self.tutorial_note(tutorial::Did::OpenedMenu);
        self.input.release_all();
        self.set_mouse_captured(false);
    }

    fn close_menu(&mut self) {
        self.menu_open = false;
        self.save_settings_if_changed();
        self.set_mouse_captured(true);
    }

    /// Starts hosting or joining, and paints everyone in their colours.
    fn start_net(&mut self, action: &MenuAction) {
        // Leaving any game first (and no longer advertising one on Steam).
        self.net = None;
        if let Some(s) = &self.steam {
            s.advertise(false);
        }
        let result = match (action, &self.steam) {
            (MenuAction::Host(port), _) => net::Net::host(*port),
            (MenuAction::Join(address), _) => net::Net::join(address),
            (MenuAction::HostSteam, Some(s)) => Ok(net::Net::host_steam(s)),
            (MenuAction::JoinSteam(host), Some(s)) => Ok(net::Net::join_steam(s, *host)),
            (MenuAction::HostSteam | MenuAction::JoinSteam(_), None) => {
                self.notify("Steam isn't running: start Steam, then restart the game.");
                return;
            }
            _ => return,
        };
        match result {
            Ok(n) => {
                let mine = n.role.color_index();
                self.set_colors(mine);
                self.notify(n.status());
                self.net = Some(n);
            }
            Err(e) => self.notify(format!("Couldn't start multiplayer: {e}")),
        }
    }

    fn stop_net(&mut self) {
        self.net = None; // says goodbye on the way out
        self.structures.clear();
        if let Some(s) = &self.steam {
            s.advertise(false);
        }
        self.remote.state = None;
        self.remote.body.hide(&mut self.renderer);
        self.set_colors(0);
    }

    /// Your colour (and your clones'), and the other player's.
    fn set_colors(&mut self, mine: usize) {
        let other = 1 - mine;
        self.body.set_color(net::PLAYER_COLORS[mine]);
        self.clones.set_color(net::PLAYER_COLORS[mine]);
        self.remote.body.set_color(net::PLAYER_COLORS[other]);
    }

    /// Drone damage is decided here: playing alone, hosting, or a joiner
    /// that hasn't reached the host (yet).
    fn authority(&self) -> bool {
        self.net.as_ref().is_none_or(|n| n.is_host() || !n.connected())
    }

    /// Receives what the other player sent: their state, shots, and (from
    /// the host) the drones and your kills.
    fn receive(&mut self, dt: f32) {
        let Some(net) = &mut self.net else { return };
        let was_connected = net.connected();
        let messages = net.poll(dt);
        let (connected, is_host) = (net.connected(), net.is_host());
        if connected != was_connected {
            self.notify(if connected { "The other player is here." } else { "The other player left." });
            if !connected {
                self.remote.state = None;
            }
        }
        let authority = self.authority();
        let (colliders, terrain) = (&self.colliders, &self.scene.terrain);
        let mut credit = 0;
        let (mut spawn_at, mut reset, mut joiner_fell) = (None, false, false);
        let phase_before = self.arena.state.phase;
        for msg in messages {
            match msg {
                net::Msg::State(s) => {
                    if self.remote.state.is_none() {
                        (self.remote.pos, self.remote.yaw) = (s.pos, s.yaw);
                    }
                    self.remote.state = Some(s);
                }
                net::Msg::Fire(f) if f.weapon == ABILITY_BLAST => {
                    // Their Whirlwind, grenade or shockwave: does it catch you?
                    let stun = f.dir.x;
                    let effect = drones::Effect { stun, ..Default::default() };
                    let events = self.weapons.burst(f.origin, f.blast_radius, f.damage, &effect, true, authority, &mut self.drones);
                    if events.player_damage > 0.0 && stun > 0.0 {
                        self.player.stun(stun);
                    }
                    credit += events.remote_kills;
                    self.pending_damage += events.player_damage;
                }
                net::Msg::Fire(f) if f.weapon == 254 => {
                    // Their Chain Lightning struck you.
                    let me = self.player.pos + glam::Vec3::Y;
                    self.weapons.add_beam(f.origin, me);
                    self.audio.play(audio::Sfx::Thunder);
                    self.pending_damage += f.damage;
                    self.player.stun(0.6);
                }
                net::Msg::Fire(f) => {
                    let shooters = [(f.origin, f.dir)];
                    let sound = weapons::Weapon::from_index(f.weapon).map_or(audio::Sfx::Slam, weapon_sound);
                    self.audio.play_at(sound, f.origin);
                    // Their melee swings hit structures too (domes break from inside).
                    if let Some(w) = weapons::Weapon::from_index(f.weapon).filter(|w| w.is_melee()) {
                        let dir = f.dir.normalize_or(glam::Vec3::X);
                        let reach = if w == weapons::Weapon::Blade && f.mods & weapons::Mods::BLADE_CONE != 0 {
                            f.blast_radius
                        } else {
                            w.reach()
                        };
                        for id in self.structures.in_reach(f.origin, dir, reach) {
                            self.struct_damage.push((id, f.damage * w.swing_damage() * structures::SHOT_SCALE));
                        }
                    }
                    let events = match weapons::Weapon::from_index(f.weapon) {
                        Some(weapon) => {
                            let volley = weapons::Volley {
                                weapon,
                                damage: f.damage,
                                pellets: f.pellets as u32,
                                blast_radius: f.blast_radius,
                                mods: weapons::Mods(f.mods),
                                homing: f.homing,
                            };
                            self.weapons.volley(&volley, &shooters, true, authority, &mut self.drones, colliders, terrain)
                        }
                        // Not a weapon: their ground slam's shockwave.
                        None => self.weapons.shockwave(f.origin, f.damage, true, authority, &mut self.drones),
                    };
                    credit += events.remote_kills;
                    self.pending_damage += events.player_damage;
                }
                net::Msg::Round(state, spawn) if !is_host => {
                    if self.arena.apply(state) == arena::Event::NewRound {
                        spawn_at = Some(spawn);
                    }
                }
                net::Msg::Build(s) => {
                    self.structures.add(s);
                    self.audio.play_at(audio::Sfx::Build, s.pos + glam::Vec3::Y);
                    // Their Absolute Zero landed on you: frozen solid.
                    if s.kind == structures::Kind::Storm
                        && s.power >= 2.0
                        && s.pos.distance(self.player.pos) < structures::STORM_RADIUS * s.scale
                    {
                        self.player.pin(2.0);
                        self.player.stun(2.0);
                        self.toast = Some(("Frozen solid!".into(), 2.5));
                        self.audio.play(audio::Sfx::Freeze);
                    }
                }
                net::Msg::Crater { pos, radius, depth } => self.their_craters.push((pos, radius, depth)),
                net::Msg::Push { impulse, pin, stun } => {
                    // Their Pull, Repulse or Ground Pull caught you.
                    if !self.dead && self.vitals.invulnerable <= 0.0 {
                        self.player.push(impulse);
                        if pin > 0.0 {
                            self.player.pin(pin);
                            self.toast = Some(("Pinned to the ground!".into(), 3.0));
                        }
                        if stun > 0.0 {
                            self.player.stun(stun);
                        }
                        self.shake = self.shake.max(0.25 * self.settings.screen_shake);
                        self.audio.play(audio::Sfx::Hurt);
                    }
                }
                net::Msg::RiftGate { owner, pair } => {
                    let owner = (owner as usize).min(1);
                    self.rift_gates[owner] = Some(pair);
                    self.weapons.set_rift_gate(owner, Some(pair));
                }
                net::Msg::Unbuild(id) => {
                    if let Some(s) = self.structures.remove(id) {
                        let at = s.top() - glam::Vec3::Y * 0.8;
                        self.weapons.add_blast(at, 1.9);
                        self.audio.play_at(audio::Sfx::Shatter, at);
                    }
                }
                net::Msg::Died if is_host => joiner_fell = true,
                net::Msg::ResetAll => reset = true,
                net::Msg::Drones(states) if !is_host => {
                    for at in self.drones.apply_states(&states) {
                        self.weapons.add_blast(at, 1.6);
                    }
                }
                net::Msg::Kills(n) if !is_host => {
                    self.weapons.kills += n;
                    if let Some(unlock) = self.progress.record(abilities::Stat::Kills, n as f32) {
                        self.unlocked.push(unlock);
                    }
                    let gained = self.progress.add_xp(n as f32 * 6.0);
                    let scrap = n * items::DRONE_SCRAP * self.moves().scrap_multiplier;
                    self.progress.scrap += scrap;
                    self.toast = Some((format!("Drone destroyed! +{} XP, +{scrap} scrap", n * 6), 3.0));
                    self.audio.play(audio::Sfx::Kill);
                    self.feel.kill = 1.0;
                    if gained {
                        self.progress.scrap += items::XP_SCRAP;
                        self.progress.save();
                        self.banner = Some(ui::Banner::new("SKILL POINT", "Earned from experience. Press Esc to spend it", ui::BANNER_GOLD));
                        self.audio.play(audio::Sfx::SkillPoint);
                    }
                }
                _ => {}
            }
        }
        if credit > 0
            && let Some(net) = &self.net
        {
            net.send(&net::Msg::Kills(credit));
        }
        if !connected && !is_host {
            self.arena.disconnect();
        }
        if let Some(spawn) = spawn_at {
            self.respawn(spawn);
        }
        // The joiner hears about phase changes from the host.
        if !is_host {
            match (phase_before, self.arena.state.phase) {
                (arena::Phase::Explore, arena::Phase::Fight) => self.announce("FIGHT!", "First to fall loses the round", ui::BANNER_RED, audio::Sfx::Fight),
                (arena::Phase::Fight, arena::Phase::Over) => self.round_result(),
                _ => {}
            }
        }
        if joiner_fell {
            self.arena.report_death(1);
            self.round_result();
        }
        if reset {
            self.reset_everyone(false);
        }
    }

    /// Host: runs the match (new rounds, the explore timer) and keeps the
    /// joiner in step. Joiner: counts down between the host's updates.
    fn run_match(&mut self, dt: f32) {
        let Some(net) = &self.net else {
            self.arena.disconnect();
            return;
        };
        if !net.is_host() {
            self.arena.tick(dt);
            return;
        }
        let phase = self.arena.state.phase;
        if self.arena.host_update(dt, net.connected()) == arena::Event::NewRound {
            self.start_round();
        }
        if phase == arena::Phase::Explore && self.arena.fighting() {
            self.announce("FIGHT!", "First to fall loses the round", ui::BANNER_RED, audio::Sfx::Fight);
        }
        let Some(net) = &self.net else { return };
        if net.connected() && self.arena.sync_due(dt) {
            net.send(&net::Msg::Round(self.arena.state, self.joiner_spawn));
        }
    }

    /// Host: a new round, both players dropped at random spots far apart.
    fn start_round(&mut self) {
        let mine = self.random_spawn(None);
        self.joiner_spawn = self.random_spawn(Some(mine));
        self.respawn(mine);
    }

    /// A random open spot on the map: dry land, not too steep, not inside
    /// anything, and (if given) at least 60 m from `away_from`.
    fn random_spawn(&mut self, away_from: Option<glam::Vec3>) -> glam::Vec3 {
        let terrain = &self.scene.terrain;
        let reach = terrain.half_size - 30.0;
        let mut best = self.scene.spawn;
        for attempt in 0..200 {
            self.rng ^= self.rng << 13;
            self.rng ^= self.rng >> 17;
            self.rng ^= self.rng << 5;
            let a = (self.rng & 0xffff) as f32 / 65535.0;
            let b = (self.rng >> 16) as f32 / 65535.0;
            let (x, z) = ((a * 2.0 - 1.0) * reach, (b * 2.0 - 1.0) * reach);
            let y = terrain.height_at(x, z);
            let p = glam::Vec3::new(x, y, z);
            let dry = y > terrain.water_level + 0.5;
            let flat = terrain.normal_at(x, z).y > 0.85;
            let body = collision::Aabb::from_center(p + glam::Vec3::Y * 1.0, glam::Vec3::new(1.0, 1.8, 1.0));
            let clear = !self.scene.colliders.iter().any(|c| c.intersects(&body));
            let apart = away_from.is_none_or(|o| o.distance(p) > 60.0) || attempt > 150;
            if dry && flat && clear && apart {
                return p;
            }
            if dry && clear {
                best = p;
            }
        }
        best
    }

    /// A powerful blast of yours at `at` (radius `blast`): if it went off on
    /// or near the ground, it leaves a crater, for the other player too.
    fn blast_crater(&mut self, at: glam::Vec3, blast: f32) {
        let ground = self.scene.terrain.height_at(at.x, at.z);
        if at.y - ground > blast * 0.6 {
            return; // an airburst
        }
        let radius = (blast * 0.45).clamp(3.0, 9.0);
        let depth = (blast * 0.12).clamp(0.8, 2.4);
        let pos = glam::Vec3::new(at.x, ground, at.z);
        self.apply_crater(pos, radius, depth);
        if let Some(net) = self.net.as_ref().filter(|n| n.connected()) {
            net.send(&net::Msg::Crater { pos, radius, depth });
        }
    }

    /// Digs a crater into the ground (collision, drawing, water and grass
    /// all follow), leaving whatever is built on the ground standing.
    fn apply_crater(&mut self, pos: glam::Vec3, radius: f32, depth: f32) {
        let colliders = &self.scene.colliders;
        let on_it = |x: f32, z: f32, h: f32| {
            colliders.iter().any(|c| {
                x > c.min.x - 0.5 && x < c.max.x + 0.5 && z > c.min.z - 0.5 && z < c.max.z + 0.5 && c.min.y < h + 0.8 && c.max.y > h - 0.5
            })
        };
        let Some((first, last)) = self.scene.terrain.dent(pos, radius, depth, on_it) else { return };
        // Neighbouring rows' normals change too.
        let terrain = &self.scene.terrain;
        let (first, last) = (first.saturating_sub(1), (last + 1).min(terrain.res - 1));
        let vertices = terrain.row_vertices(first, last);
        self.renderer.update_terrain(first, &vertices, &terrain.heights, terrain.res as u32);
        self.add_shake(0.1 + depth * 0.08);
    }

    /// Where every clone stands once one more joins: a circle in front of
    /// you, with you on it, all facing its middle.
    fn clone_circle(&self, moves: &abilities::Moveset) -> Vec<(glam::Vec3, f32)> {
        let n = (self.clones.count_of(clones::Kind::Mirror) + 1).min(moves.max_clones);
        let terrain = &self.scene.terrain;
        let mut spots = clones::formation(self.player.pos, self.camera.yaw, n, |x, z| terrain.height_at(x, z));
        // A spot inside a wall or rock: pulled in towards you until it's clear.
        let blocked = |p: glam::Vec3| {
            let body = collision::Aabb::from_center(p + glam::Vec3::Y * 0.95, glam::Vec3::new(0.7, 1.8, 0.7));
            self.colliders.iter().any(|c| c.intersects(&body))
        };
        for (pos, _) in &mut spots {
            if !blocked(*pos) {
                continue;
            }
            let from = *pos;
            *pos = [0.75, 0.5, 0.25, 0.0]
                .into_iter()
                .map(|f| {
                    let p = self.player.pos + (from - self.player.pos) * f;
                    glam::Vec3::new(p.x, p.y.max(terrain.height_at(p.x, p.z)), p.z)
                })
                .find(|p| !blocked(*p))
                .unwrap_or(self.player.pos);
        }
        spots
    }

    /// One more clone joins the circle (the oldest goes past the limit).
    /// False if you can't have clones.
    fn spawn_clone(clones: &mut clones::Clones, renderer: &mut Renderer, spots: &[(glam::Vec3, f32)], yaw: f32, max: usize) -> bool {
        let Some(&(pos, facing)) = spots.last() else { return false };
        clones.spawn(clones::Kind::Mirror, pos, facing, yaw, max, renderer);
        clones.arrange(spots, yaw);
        true
    }

    /// Back in the round: at `spawn`, full health, air abilities refilled.
    fn respawn(&mut self, spawn: glam::Vec3) {
        self.player.teleport(spawn);
        self.player.refill(&self.moves());
        self.health = self.moves().max_health;
        self.vitals.trail = self.health;
        self.vitals.downed = None;
        self.dead = false;
        self.pending_damage = 0.0;
        self.hook = None;
        self.trail.clear();
        self.last_pos = spawn;
        self.body.reset_motion();
        let round = self.arena.state.round;
        self.announce(
            &format!("ROUND {round}"),
            format!("{:.0} seconds to explore, then fight!", arena::EXPLORE_TIME),
            ui::BANNER_BLUE,
            audio::Sfx::Unlock,
        );
        self.last_beep = -1;
    }

    /// Applies damage from the other player's shots (only during a fight).
    /// Applies this frame's damage: the other player's shots (only during a
    /// fight) and the drones' orbs. Armour softens it, the shield takes it
    /// first, and Second Wind may catch a fatal blow.
    fn take_damage(&mut self) {
        let pvp = std::mem::take(&mut self.pending_damage);
        let pvp = if self.arena.fighting() { pvp } else { 0.0 };
        let pve = std::mem::take(&mut self.pve_damage);
        let moves = self.moves();
        let mut damage = (pvp + pve) * moves.armor;
        if damage <= 0.0 || self.dead || self.vitals.invulnerable > 0.0 || self.screen != Screen::Playing {
            return;
        }
        // Guarding: a fresh guard (Parry) turns the hit away entirely.
        if self.blocking {
            if moves.parry && self.block_time < 0.3 {
                let here = self.player.pos + glam::Vec3::Y;
                for (i, _) in self.drones.within(here, 6.0) {
                    if let Some(p) = self.drones.position(i) {
                        self.drones.push(i, (p - here).normalize_or(glam::Vec3::Y) * 10.0, 1.5);
                    }
                }
                self.heal(10.0, false);
                self.weapons.add_flash(here, 1.5);
                self.audio.play(audio::Sfx::Crit);
                self.notify("PARRY!");
                return;
            }
            damage *= moves.block;
            self.audio.play(audio::Sfx::ShieldHit);
        }
        // A real hit spoils a spell being cast (unless your will is unshakable).
        if let Some((spell, _, _)) = self.casting
            && !moves.steady_cast
            && damage >= 10.0
        {
            self.casting = None;
            self.cooldowns[spell.index()] = 2.0;
            self.notify(format!("{} interrupted!", spell.name()));
            self.audio.play(audio::Sfx::Denied);
        }
        // Arcane Ward soaks it first.
        let warded = damage.min(self.vitals.ward);
        self.vitals.ward -= warded;
        damage -= warded;
        if damage <= 0.0 {
            self.audio.play(audio::Sfx::ShieldHit);
            return;
        }
        if let Some(u) = self.progress.record(abilities::Stat::DamageTaken, damage) {
            self.unlocked.push(u);
        }
        let v = &mut self.vitals;
        v.since_hurt = 0.0;
        let absorbed = damage.min(v.shield);
        v.shield -= absorbed;
        damage -= absorbed;
        self.health -= damage;
        self.add_shake(0.12 + (damage / 50.0).min(0.25));
        if damage >= 15.0 {
            self.feel.hitstop = self.feel.hitstop.max(HITSTOP_HURT * self.settings.screen_shake.min(1.0));
        }
        self.feel.damage = (0.4 + damage / 20.0).min(1.0);
        if self.health > 0.0 {
            self.audio.play(if damage > 0.0 { audio::Sfx::Hurt } else { audio::Sfx::Hooked });
            return;
        }
        // A blow that would down you: Second Wind, once a minute.
        if moves.second_wind && self.vitals.second_wind <= 0.0 {
            self.health = moves.max_health * 0.4;
            self.vitals.invulnerable = 2.0;
            self.vitals.second_wind = 60.0;
            self.announce("SECOND WIND", "Back from the brink: 2 s untouchable", ui::BANNER_GOLD, audio::Sfx::SkillPoint);
            return;
        }
        self.audio.play(audio::Sfx::Death);
        self.health = 0.0;
        self.dead = true;
        self.weapons.add_blast(self.player.pos + glam::Vec3::Y, 2.5);
        if pvp > 0.0 || self.arena.fighting() {
            // Out of the round.
            match &self.net {
                Some(net) if net.is_host() => {
                    self.arena.report_death(0);
                    self.round_result();
                }
                Some(net) => net.send(&net::Msg::Died),
                None => {}
            }
        } else {
            // Downed by drones: back in the village shortly.
            self.vitals.downed = Some(4.0);
            self.announce("DOWNED", "The drones got you. Back in the village in a moment", ui::BANNER_RED, audio::Sfx::Defeat);
        }
    }

    /// After being downed outside a match: up again in the village square.
    fn revive_in_village(&mut self) {
        let spawn = self.scene.spawn;
        self.player.teleport(spawn);
        self.player.refill(&self.moves());
        self.dead = false;
        self.vitals.downed = None;
        self.health = self.moves().max_health;
        self.vitals.trail = self.health;
        self.vitals.invulnerable = 3.0;
        self.trail.clear();
        self.last_pos = spawn;
        self.hook = None;
        self.body.reset_motion();
        self.camera.yaw = self.scene.spawn_yaw;
        self.fade = 1.0;
        self.announce("BACK ON YOUR FEET", "3 s of protection", ui::BANNER_GREEN, audio::Sfx::Unlock);
    }

    /// Heals from your attacks (lifesteal, Berserker kills).
    /// Heals you; `from_attacks` (lifesteal, Berserker) counts towards the
    /// Berserker trial, healing pads don't.
    fn heal(&mut self, amount: f32, from_attacks: bool) {
        let max = self.moves().max_health;
        let healed = amount.min(max - self.health).max(0.0);
        if healed <= 0.0 || self.dead {
            return;
        }
        self.health += healed;
        self.vitals.heal_flash = self.vitals.heal_flash.max(if from_attacks { 1.0 } else { 0.35 });
        if from_attacks
            && let Some(u) = self.progress.record(abilities::Stat::Healing, healed)
        {
            self.unlocked.push(u);
        }
    }

    /// The key or button of each of the ten slots: yours if you changed
    /// it, else its default.
    fn slot_binds(&self) -> [input::Bind; abilities::LOADOUT] {
        std::array::from_fn(|i| {
            self.settings
                .binds
                .get(&format!("slot_{}", i + 1))
                .and_then(|b| input::Bind::parse(b))
                .unwrap_or_else(|| input::Bind::parse(abilities::SLOT_KEYS[i]).expect("every default slot key is bindable"))
        })
    }

    /// Each skill's key: the key of the slot it's in (None: not equipped).
    fn binds(&self) -> [Option<input::Bind>; abilities::SKILLS] {
        let slots = self.slot_binds();
        std::array::from_fn(|i| self.progress.slot_of(abilities::Skill::ALL[i]).map(|slot| slots[slot]))
    }

    /// Binds slot `slot` to `bind`. A slot that had it takes this slot's
    /// old key, so no two slots ever share one.
    fn set_slot_bind(&mut self, slot: usize, bind: input::Bind) {
        if !bind.allowed() {
            self.notify(format!("{} is kept for movement or menus: pick another key", bind.label()));
            self.audio.play(audio::Sfx::Denied);
            return;
        }
        let binds = self.slot_binds();
        let old = binds[slot];
        if let Some(other) = (0..abilities::LOADOUT).find(|&i| i != slot && binds[i] == bind) {
            self.settings.binds.insert(format!("slot_{}", other + 1), old.id());
            self.notify(format!("Slot {} now on {}, slot {} moved to {}", slot + 1, bind.label(), other + 1, old.label()));
        } else {
            self.notify(format!("Slot {} now on {}", slot + 1, bind.label()));
        }
        self.settings.binds.insert(format!("slot_{}", slot + 1), bind.id());
        self.save_settings_if_changed();
        self.audio.play(audio::Sfx::Unlock);
    }

    /// The Skills tab is open and not busy (no slot waiting for a key, no
    /// text box being typed in): pressing a key assigns the chosen skill.
    fn choosing_keys(&self) -> bool {
        self.menu_open && self.menu.tab == ui::Tab::Abilities && self.menu.binding.is_none() && !self.egui_ctx.egui_wants_keyboard_input()
    }

    /// Puts `skill` on `bind`: into the slot that already has that key, or
    /// (if none does) its own slot or the first empty one, moved onto it.
    fn assign_to(&mut self, skill: abilities::Skill, bind: input::Bind) {
        if !bind.allowed() {
            return;
        }
        let owned = self.settings.unlock_all || self.progress.rank(skill) > 0;
        if !owned {
            self.notify(format!("Learn {} first, then press a key to put it there.", skill.name()));
            self.audio.play(audio::Sfx::Denied);
            return;
        }
        let binds = self.slot_binds();
        let slot = match binds.iter().position(|b| *b == bind) {
            Some(i) => i,
            None => {
                let Some(i) = self.progress.slot_of(skill).or_else(|| self.progress.slots().iter().position(Option::is_none)) else {
                    self.notify("All 10 slots are full: clear one, or press one of their keys.");
                    self.audio.play(audio::Sfx::Denied);
                    return;
                };
                self.set_slot_bind(i, bind);
                i
            }
        };
        if self.progress.slot_of(skill) == Some(slot) && self.slot_binds()[slot] == bind {
            self.notify(format!("{} is on {}", skill.name(), bind.label()));
            return;
        }
        if self.progress.place(skill, slot, self.settings.unlock_all).is_ok() {
            self.progress.save();
            self.notify(format!("{} is now on {}", skill.name(), bind.label()));
            self.audio.play(audio::Sfx::Unlock);
        }
    }

    /// Your player number (0 host or alone, 1 joiner).
    fn me(&self) -> usize {
        self.net.as_ref().map_or(0, |n| n.role.color_index())
    }

    /// Where a ceiling or dome goes: on the other player if you're looking
    /// at them, else where you aim (within 60 m).
    fn build_target(&self) -> Option<glam::Vec3> {
        self.aimed_player(60.0).or_else(|| {
            let (eye, aim) = (self.camera.position, self.camera.forward());
            gadgets::raycast(eye, aim, 60.0, &self.colliders, &self.scene.terrain)
        })
    }

    /// Spell targets can reach anywhere in the valley; a player target has no
    /// distance limit, while terrain rays stop at the edge of playable space.
    fn spell_target(&self) -> Option<glam::Vec3> {
        if let Some(at) = self.spell_at {
            return Some(at.target);
        }
        self.aimed_player(f32::INFINITY).or_else(|| {
            gadgets::raycast(self.camera.position, self.camera.forward(), SPELL_AIM_RANGE, &self.colliders, &self.scene.terrain)
        })
    }

    /// The other player's feet, if you're looking at them within `range`.
    fn aimed_player(&self, range: f32) -> Option<glam::Vec3> {
        let (eye, aim) = (self.camera.position, self.camera.forward());
        let connected = self.net.as_ref().is_some_and(|n| n.connected());
        if connected && self.remote.state.is_some_and(|s| s.health > 0) {
            let to = self.remote.pos + glam::Vec3::Y * 0.9 - eye;
            if to.length() < range && to.normalize().dot(aim) > 0.95 {
                return Some(self.remote.pos);
            }
        }
        None
    }

    /// Drones in a narrow cone along your view, within `range`.
    fn aimed_drones(&self, range: f32) -> Vec<(usize, glam::Vec3)> {
        let (eye, aim) = (self.camera.position, self.camera.forward());
        self.drones
            .within(eye, range)
            .into_iter()
            .filter_map(|(i, _)| self.drones.position(i).map(|p| (i, p)))
            .filter(|&(_, p)| (p - eye).normalize_or_zero().dot(aim) > 0.93)
            .collect()
    }

    /// Sends the other player flying (they apply it on their side).
    fn push_other(&self, impulse: glam::Vec3, pin: f32, stun: f32) {
        if let Some(net) = self.net.as_ref().filter(|n| n.connected()) {
            net.send(&net::Msg::Push { impulse, pin, stun });
        }
    }

    /// What a spell planted at `origin` would be aimed at if its cast ended
    /// now: shots fly at whatever is under the crosshair (the other player,
    /// a drone, the world, or far off into the sky); other spells land on
    /// the ground (or the other player) there, and self-centred ones go off
    /// on their circle.
    fn spell_aim(&self, spell: abilities::Skill, origin: glam::Vec3) -> SpellPlace {
        let (eye, aim) = (self.camera.position, self.camera.forward());
        let flat = glam::Vec3::new(aim.x, 0.0, aim.z).normalize_or(glam::Vec3::X);
        if centred(spell) {
            return SpellPlace { origin, target: origin, dir: flat };
        }
        let target = if shot_spell(spell) {
            let world = gadgets::raycast(eye, aim, SPELL_AIM_RANGE, &self.colliders, &self.scene.terrain).map_or(SPELL_AIM_RANGE, |p| p.distance(eye));
            let reach = self.drones.ray_hit(eye, aim, world).map_or(world, |(_, t)| t);
            self.aimed_player(f32::INFINITY).map_or(eye + aim * reach, |feet| feet + glam::Vec3::Y * 0.9)
        } else {
            self.spell_target().unwrap_or_else(|| {
                // Aimed at the sky: the spot on the ground beneath it.
                let far = eye + aim * 250.0;
                glam::Vec3::new(far.x, self.scene.terrain.height_at(far.x, far.z), far.z)
            })
        };
        let hand = origin + glam::Vec3::Y * 1.3;
        SpellPlace { origin, target, dir: (target - hand).normalize_or(aim) }
    }

    /// Where a cast shot leaves your hand, and its direction: at whatever
    /// is under the crosshair (like a gun).
    fn spell_shot(&self) -> (glam::Vec3, glam::Vec3) {
        // A planted spell fires from its circle, at what you aimed it at.
        if let Some(at) = self.spell_at {
            return (at.origin + glam::Vec3::Y * 1.3, at.dir);
        }
        let (eye, aim) = (self.camera.position, self.camera.forward());
        let world = gadgets::raycast(eye, aim, 250.0, &self.colliders, &self.scene.terrain).map_or(250.0, |p| p.distance(eye));
        let reach = self.drones.ray_hit(eye, aim, world).map_or(world, |(_, t)| t);
        let side = glam::Vec3::new(-self.camera.yaw.sin(), 0.0, self.camera.yaw.cos());
        let hand = self.spell_from.unwrap_or(self.player.pos) + glam::Vec3::Y * 1.35 + side * 0.3;
        (hand, (eye + aim * reach - hand).normalize_or(aim))
    }

    /// Fires a spell's shots (also sent to the other player).
    fn spell_volley(&mut self, volley: &weapons::Volley, shooters: &[(glam::Vec3, glam::Vec3)]) {
        let authority = self.authority();
        let events = self.weapons.volley(volley, shooters, false, authority, &mut self.drones, &self.colliders, &self.scene.terrain);
        self.extra_kills += events.kills;
        self.send_volley(volley, shooters);
    }

    /// Frost Nova: freeze and throw back everything around you, and shatter
    /// incoming shots (Deep Freeze: wider, and it hurts).
    fn frost_nova(&mut self, moves: &abilities::Moveset, power: f32) {
        let deep = moves.frost_nova >= 2;
        let center = self.spell_at.map_or(self.spell_from.unwrap_or(self.player.pos), |at| at.target) + glam::Vec3::Y;
        let r = if deep { 26.0 } else { 18.0 } * self.spell_boost.sqrt();
        if self.net.as_ref().is_some_and(|n| n.connected()) && self.remote.state.is_some_and(|s| s.health > 0) {
            let to = self.remote.pos + glam::Vec3::Y - center;
            if to.length() < r {
                let away = glam::Vec3::new(to.x, 0.0, to.z).normalize_or(glam::Vec3::X);
                self.push_other(away * 10.0 + glam::Vec3::Y * 3.0, 0.0, 1.2);
            }
        }
        for (i, _) in self.drones.within(center, r) {
            if let Some(p) = self.drones.position(i) {
                self.drones.push(i, (p - center).normalize_or(glam::Vec3::Y) * 12.0, 2.5);
            }
        }
        if deep && self.authority() {
            let damage = 10.0 * moves.weapon_damage * power;
            let chill = drones::Effect { stun: 0.8, ..Default::default() };
            let hit = self.weapons.burst(center, r, damage, &chill, false, true, &mut self.drones);
            self.send_blast(center, r, damage, 0.0);
            self.extra_kills += hit.kills;
        }
        self.drones.clear_orbs_near(center, r);
        self.weapons.add_flash(center, r * 0.7);
        self.audio.play_at(audio::Sfx::Freeze, center);
        self.add_shake(0.2);
    }

    /// Shadow Step: reappear just behind the drone or player you aim at,
    /// facing them (Assassin: and stun them). False if there's no target.
    fn shadow_step(&mut self, moves: &abilities::Moveset) -> bool {
        let (eye, aim) = (self.camera.position, self.camera.forward());
        let target = match self.aimed_player(80.0) {
            Some(feet) => Some((feet + glam::Vec3::Y, None)),
            None => self.drones.ray_hit(eye, aim, 80.0).and_then(|(i, _)| self.drones.position(i).map(|p| (p, Some(i)))),
        };
        let Some((at, drone)) = target else { return false };
        let flat = glam::Vec3::new(at.x - eye.x, 0.0, at.z - eye.z).normalize_or(glam::Vec3::X);
        let mut dest = at + flat * 2.5;
        dest.y = (at.y - 1.0).max(self.scene.terrain.height_at(dest.x, dest.z));
        let from = self.player.pos;
        let to = dest - from;
        self.player.blink_free(to.normalize_or(flat), to.length(), &self.colliders, &self.scene.terrain);
        self.body.reset_motion();
        // Turn round to face what you stepped behind.
        self.camera.yaw = (-flat.z).atan2(-flat.x);
        if moves.shadow_step >= 2 {
            match drone {
                Some(i) => self.drones.push(i, glam::Vec3::ZERO, 1.5),
                None => self.push_other(glam::Vec3::ZERO, 0.0, 1.5),
            }
        }
        self.weapons.add_flash(from + glam::Vec3::Y, 1.2);
        self.weapons.add_flash(self.player.pos + glam::Vec3::Y, 1.2);
        self.audio.play(audio::Sfx::Dash);
        self.feel.fov_kick += 10.0;
        true
    }

    /// A finished cast: the spell lies planted on its circle as a rune,
    /// aimed where you were aiming as the cast ended, until you press its
    /// key again. Its overcharge starts fading now. It costs the spell's
    /// cooldown now.
    fn set_rune(&mut self, spell: abilities::Skill, boost: f32, origin: glam::Vec3) {
        let at = self.spell_aim(spell, origin);
        self.runes.retain(|r| r.spell != spell);
        self.runes.push(Rune { spell, boost, at, fuse: RUNE_LIFE });
        self.cooldowns[spell.index()] = skill_cooldown(spell);
        self.weapons.add_flash(origin + glam::Vec3::Y * 0.5, 1.0 + boost * 0.5);
        self.audio.play_at(audio::Sfx::Marker, origin);
        let key = self.binds()[spell.index()].map_or_else(|| "its key".to_owned(), |b| b.label());
        let charge = if boost > 1.05 { format!(" at {:.0}% (fading)", boost * 100.0) } else { String::new() };
        self.notify(format!("{} ready{charge}: press {key} to cast it", spell.name()));
    }

    /// A rune set off by its own key: it goes off, your arcane clones cast
    /// it too (from where they stand, at the same target), and it counts
    /// as an ability for Chrono Echo and your clone's actions.
    fn release_rune(&mut self, rune: Rune) {
        let spell = rune.spell;
        let (at, boost) = (rune.at, rune.boost);
        self.detonate(vec![rune]);
        // Only spells that strike from where they're cast are worth a
        // copy: a clone's Ward, Haste or Rejuvenate would just buff or
        // heal you again.
        let copies = if clone_copies(spell) { self.clones.arcane() } else { Vec::new() };
        for pos in copies {
            self.weapons.add_flash(pos + glam::Vec3::Y * 1.4, 1.0);
            let mut copy = self.spell_aim(spell, pos);
            if !centred(spell) {
                copy.target = at.target;
                copy.dir = (at.target - (pos + glam::Vec3::Y * 1.3)).normalize_or(at.dir);
            }
            let cooldown = self.cooldowns[spell.index()];
            self.spell_at = Some(copy);
            self.spell_from = Some(pos);
            self.spell_boost = boost;
            self.cast_spell(spell);
            self.cooldowns[spell.index()] = cooldown;
        }
        self.spell_at = None;
        self.spell_from = None;
        self.spell_boost = 1.0;
        arm_echo(&mut self.echo_charges, &mut self.echo_repeat, spell, glam::Vec3::ZERO);
        if self.clones.spend_action(&mut self.renderer) > 0 {
            self.notify("Your clone faded after its five actions.");
        }
    }

    /// Runes going off together. Each is stronger for every other one in
    /// the group (resonance), and different elements close together react.
    fn detonate(&mut self, runes: Vec<Rune>) {
        let resonance = 1.0 + RESONANCE * (runes.len() as f32 - 1.0);
        for r in &runes {
            let cooldown = self.cooldowns[r.spell.index()];
            self.spell_boost = r.boost * resonance;
            self.spell_at = Some(r.at);
            self.cast_spell(r.spell);
            self.cooldowns[r.spell.index()] = cooldown;
        }
        self.spell_boost = 1.0;
        self.spell_at = None;
        if runes.len() > 1 {
            self.announce("RESONANCE", format!("{} runes at once: each {:.0}% stronger", runes.len(), (resonance - 1.0) * 100.0), ui::BANNER_BLUE, audio::Sfx::Unlock);
        }
        let power = resonance * runes.iter().map(|r| r.boost).fold(1.0, f32::max);
        // React with each other, and with runes set off a moment ago (with
        // another key).
        let now: Vec<(Element, glam::Vec3)> = runes.iter().map(|r| (element(r.spell), r.at.target)).collect();
        let mut placed: Vec<(Element, glam::Vec3)> = self.recent_runes.iter().map(|r| (r.0, r.1)).collect();
        placed.extend(now.iter().copied());
        let found = reactions(&placed);
        for &(kind, at) in &found {
            let power = if matches!(kind, Reaction::Cataclysm) { power * 1.5 } else { power };
            self.reaction(kind, at, power);
        }
        if found.is_empty() {
            self.recent_runes.extend(now.into_iter().map(|(e, p)| (e, p, 0.0)));
        } else {
            // Spent: they've reacted.
            self.recent_runes.clear();
        }
    }

    /// An elemental reaction going off at `at`.
    fn reaction(&mut self, kind: Reaction, at: glam::Vec3, power: f32) {
        let moves = self.moves();
        let dmg = moves.weapon_damage * moves.spell_power * power;
        let authority = self.authority();
        let ground = glam::Vec3::new(at.x, self.scene.terrain.height_at(at.x, at.z) + 1.0, at.z);
        let blast = |game: &mut Self, pos: glam::Vec3, radius: f32, damage: f32, effect: drones::Effect, stun: f32| {
            let hit = game.weapons.burst(pos, radius, damage, &effect, false, authority, &mut game.drones);
            game.extra_kills += hit.kills;
            game.send_blast(pos, radius, damage, stun);
            game.weapons.add_blast(pos, radius);
            game.audio.play_at(audio::Sfx::Explosion(radius), pos);
        };
        let (title, line) = match kind {
            Reaction::Steam => {
                let scald = drones::Effect { burn: (4.0 * dmg, 3.0), ..Default::default() };
                blast(self, ground, 14.0, 22.0 * dmg, scald, 0.0);
                ("STEAM BURST", "Fire met frost: a scalding blast")
            }
            Reaction::Superconduct => {
                let shock = drones::Effect { stun: 3.5, shock_every: 1, shock_bonus: 4.0 * dmg, chain: true, ..Default::default() };
                blast(self, ground, 22.0, 8.0 * dmg, shock, 2.0);
                ("SUPERCONDUCT", "Storm met frost: everything around is locked in place")
            }
            Reaction::Overload => {
                let flame = drones::Effect { burn: (2.0 * dmg, 3.0), ..Default::default() };
                for k in 0..5 {
                    let a = k as f32 / 5.0 * std::f32::consts::TAU;
                    let p = ground + glam::Vec3::new(a.cos(), 0.0, a.sin()) * 8.0;
                    blast(self, p, 7.0, 12.0 * dmg, flame, 0.0);
                }
                ("OVERLOAD", "Fire met storm: a ring of explosions")
            }
            Reaction::Cataclysm => {
                let ruin = drones::Effect { stun: 2.0, burn: (5.0 * dmg, 4.0), ..Default::default() };
                blast(self, ground, 28.0, 45.0 * dmg, ruin, 1.5);
                self.weapons.craters.push((ground, 24.0));
                ("CATACLYSM", "Fire, frost and storm as one")
            }
        };
        self.add_shake(0.5);
        self.feel.hitstop = self.feel.hitstop.max(0.09 * self.settings.screen_shake.min(1.0));
        self.announce(title, line, ui::BANNER_GOLD, audio::Sfx::Victory);
    }

    /// A spell's cast bar filled: it goes off.
    fn cast_spell(&mut self, spell: abilities::Skill) {
        use abilities::Skill;
        let moves = self.moves();
        // Overcharge and resonance: more power, and (as the square root) a
        // wider area.
        let power = moves.spell_power * 1.5 * self.spell_boost;
        let area = self.spell_boost.sqrt();
        if self.spell_from.is_none()
            && let Some(u) = self.progress.record(abilities::Stat::SpellsCast, 1.0)
        {
            self.unlocked.push(u);
        }
        match spell {
            Skill::ArcaneClone => {
                // A pair, one either side of you (the oldest pair makes way
                // past the limit).
                let max = moves.arcane_clones;
                let yaw = self.camera.yaw;
                for _ in 0..2 {
                    let k = self.clones.count_of(clones::Kind::Arcane).min(max.saturating_sub(1));
                    let pos = clones::Clones::beside(self.player.pos, yaw, k);
                    self.clones.spawn(clones::Kind::Arcane, pos, yaw, yaw, max, &mut self.renderer);
                    self.weapons.add_flash(pos + glam::Vec3::Y, 1.4);
                }
                self.audio.play(audio::Sfx::Clone);
                self.notify(format!("Arcane clones ready: they mirror your next {} attacks and spells.", clones::CLONE_ACTIONS));
            }
            Skill::Meteor => {
                let Some(target) = self.spell_target() else {
                    self.notify("The meteor found nothing to fall on: aim at the ground or the other player.");
                    self.audio.play(audio::Sfx::Denied);
                    self.cooldowns[spell.index()] = 3.0;
                    return;
                };
                // Meteors: huge burning rockets dropped from high above.
                let volley = weapons::Volley {
                    weapon: weapons::Weapon::Rockets,
                    damage: 3.5 * moves.weapon_damage * power,
                    pellets: 0,
                    blast_radius: 20.0 * area,
                    mods: weapons::Mods(weapons::Mods::NAPALM),
                    homing: false,
                };
                let spots: &[(f32, f32)] = if moves.meteors >= 3 { &[(0.0, 0.0), (13.0, 5.0), (-11.0, -8.0), (-6.0, 13.0), (9.0, -12.0)] } else { &[(0.0, 0.0)] };
                let shooters: Vec<(glam::Vec3, glam::Vec3)> =
                    spots.iter().map(|&(x, z)| (target + glam::Vec3::new(x, 55.0, z), glam::Vec3::NEG_Y)).collect();
                let authority = self.authority();
                let events = self.weapons.volley(&volley, &shooters, false, authority, &mut self.drones, &self.colliders, &self.scene.terrain);
                self.extra_kills += events.kills;
                self.send_volley(&volley, &shooters);
                self.weapons.add_flash(target + glam::Vec3::Y * 0.3, 3.0);
            }
            Skill::ChainLightning => self.chain_lightning(&moves),
            Skill::Blizzard => {
                let Some(target) = self.spell_target() else {
                    self.notify("Nowhere to call the blizzard: aim at the ground or the other player.");
                    self.audio.play(audio::Sfx::Denied);
                    self.cooldowns[spell.index()] = 3.0;
                    return;
                };
                let s = structures::Structure {
                    id: self.structures.new_id(self.me()),
                    kind: structures::Kind::Storm,
                    pos: target,
                    yaw: 0.0,
                    health: 1.0,
                    max_health: 1.0,
                    life: 8.0,
                    warden: false,
                    // A storm 44 m across (12 m radius at size 1).
                    scale: moves.build_scale.sqrt() * 1.85 * area,
                    power: moves.blizzard as f32 * power,
                };
                self.structures.add(s);
                if let Some(net) = self.net.as_ref().filter(|n| n.connected()) {
                    net.send(&net::Msg::Build(s));
                }
                // Absolute Zero: drones inside freeze solid.
                if moves.blizzard >= 2 {
                    for (i, _) in self.drones.within(target, structures::STORM_RADIUS * s.scale) {
                        self.drones.push(i, glam::Vec3::ZERO, 2.0);
                    }
                }
                self.audio.play_at(audio::Sfx::Freeze, target);
            }
            Skill::ArcaneWard => {
                self.vitals.ward = moves.ward * power;
                self.vitals.ward_time = 8.0;
                if moves.ward >= 200.0 {
                    self.heal(40.0, false);
                }
                self.weapons.add_flash(self.player.pos + glam::Vec3::Y, 1.6);
            }
            Skill::Dispel => self.dispel(&moves),
            Skill::ChronoEcho => {
                // Afterimage: the next two abilities echo.
                self.echo_charges = if moves.chrono_echo >= 2 { 2 } else { 1 };
                self.echo_left = ECHO_WINDOW;
                self.weapons.add_flash(self.player.pos + glam::Vec3::Y, 1.4);
                self.notify(if self.echo_charges > 1 { "Echo: your next two abilities happen twice." } else { "Echo: your next ability happens twice." });
            }
            Skill::RiftGate => self.place_rift_gate(),
            Skill::Fireball => {
                let (hand, dir) = self.spell_shot();
                let volley = weapons::Volley {
                    weapon: weapons::Weapon::Rockets,
                    damage: 0.9 * moves.weapon_damage * power,
                    pellets: 0,
                    blast_radius: 14.0 * area,
                    mods: weapons::Mods(weapons::Mods::NAPALM),
                    homing: false,
                };
                let fan: &[f32] = if moves.fireball >= 2 { &[-0.22, 0.0, 0.22] } else { &[0.0] };
                let shooters: Vec<_> = fan.iter().map(|&a| (hand, glam::Quat::from_rotation_y(a) * dir)).collect();
                self.spell_volley(&volley, &shooters);
                self.audio.play(audio::Sfx::RocketFire);
            }
            Skill::ArcaneMissiles => {
                let (hand, dir) = self.spell_shot();
                let barrage = moves.missiles >= 2;
                let volley = weapons::Volley {
                    weapon: weapons::Weapon::Scatter,
                    damage: if barrage { 0.7 } else { 0.6 } * moves.weapon_damage * power,
                    pellets: ((if barrage { 28.0 } else { 16.0 }) * area) as u32,
                    blast_radius: 0.0,
                    mods: weapons::Mods(weapons::Mods::ION),
                    homing: true,
                };
                self.spell_volley(&volley, &[(hand, dir)]);
                self.audio.play(audio::Sfx::Scatter);
            }
            Skill::ArcaneLance => {
                let (hand, dir) = self.spell_shot();
                let volley = weapons::Volley {
                    weapon: weapons::Weapon::Railgun,
                    damage: 0.45 * moves.weapon_damage * power,
                    pellets: 0,
                    blast_radius: 0.0,
                    mods: weapons::Mods(weapons::Mods::ION),
                    homing: false,
                };
                // A fan of beams sweeping the area ahead.
                let (beams, half) = if moves.lance >= 2 { (13, 0.5) } else { (7, 0.35) };
                let fan: Vec<f32> = (0..beams).map(|k| (k as f32 / (beams - 1) as f32 * 2.0 - 1.0) * half).collect();
                let shooters: Vec<_> = fan.iter().map(|&a| (hand, glam::Quat::from_rotation_y(a) * dir)).collect();
                self.spell_volley(&volley, &shooters);
                self.audio.play(audio::Sfx::Railgun);
                self.add_shake(0.12);
            }
            Skill::FrostNova => self.frost_nova(&moves, power),
            Skill::Rejuvenate => {
                let renewal = moves.rejuvenate >= 2;
                self.heal(if renewal { 50.0 } else { 30.0 }, false);
                if renewal {
                    self.vitals.ward = self.vitals.ward.max(60.0 * moves.spell_power);
                    self.vitals.ward_time = 8.0;
                }
                let pad = structures::Structure {
                    id: self.structures.new_id(self.me()),
                    kind: structures::Kind::HealPad,
                    pos: self.spell_at.map_or(self.player.pos, |at| at.target),
                    yaw: 0.0,
                    health: 1.0,
                    max_health: 1.0,
                    life: 6.0,
                    warden: false,
                    // A circle 15 m across (a heal pad is 5 m at size 1).
                    scale: moves.build_scale * 3.0 * area,
                    power: if renewal { 16.0 } else { 10.0 } * moves.spell_power,
                };
                self.structures.add(pad);
                if let Some(net) = self.net.as_ref().filter(|n| n.connected()) {
                    net.send(&net::Msg::Build(pad));
                }
                self.weapons.add_flash(self.player.pos + glam::Vec3::Y, 1.5);
            }
            Skill::Haste => {
                self.haste_left = haste_time(&moves);
                self.audio.play(audio::Sfx::Boost);
                self.feel.fov_kick += 6.0;
                self.notify(format!("Haste: everything 35% faster for {:.0} s.", self.haste_left));
            }
            Skill::ShadowStep => {
                if !self.shadow_step(&moves) {
                    self.notify("Shadow Step needs a target: aim at a drone or the other player.");
                    self.audio.play(audio::Sfx::Denied);
                    self.cooldowns[spell.index()] = 2.0;
                    return;
                }
            }
            Skill::TimeWarp => {
                if self.time_effect.is_some() || self.time_cooldown > 0.0 {
                    return;
                }
                if moves.time_stop {
                    self.time_effect = Some((TimeEffect::Stop, moves.time_stop_length));
                    self.notify("Time stopped.");
                    self.audio.play(audio::Sfx::TimeStop);
                } else if moves.bullet_time {
                    self.time_effect = Some((TimeEffect::Bullet, 5.0));
                    self.notify("Bullet time.");
                    self.audio.play(audio::Sfx::TimeSlow);
                }
                self.cooldowns[spell.index()] = 0.0;
                return;
            }
            _ => {
                // Recall Home.
                let spawn = self.scene.spawn;
                self.player.teleport(spawn);
                self.health = moves.max_health;
                self.vitals.trail = self.health;
                self.trail.clear();
                self.last_pos = spawn;
                self.hook = None;
                self.body.reset_motion();
                self.fade = 0.7;
            }
        }
        self.cooldowns[spell.index()] = skill_cooldown(spell);
        self.audio.play(audio::Sfx::SpellRelease);
        self.notify(format!("{}!", spell.rank_info(self.progress.rank(spell).max(1)).name));
    }

    /// Teleport through either endpoint when the player's movement crosses
    /// its plane inside the opening.
    fn try_rift_player(
        player: &mut Player,
        camera: &mut Camera,
        gates: [Option<net::GatePair>; 2],
        before: glam::Vec3,
        weapons: &mut weapons::Weapons,
    ) {
        let start = before + glam::Vec3::Y * 0.9;
        let end = player.pos + glam::Vec3::Y * 0.9;
        for pair in gates.into_iter().flatten() {
            for (source, destination) in [(pair.a, pair.b), (pair.b, pair.a)] {
                let normal = glam::Vec3::new(source.yaw.cos(), 0.0, source.yaw.sin());
                let right = glam::Vec3::new(-normal.z, 0.0, normal.x);
                let d0 = (start - source.pos).dot(normal);
                let d1 = (end - source.pos).dot(normal);
                if (d0 - d1).abs() < 1e-4 { continue; }
                let crossed_plane = d0 * d1 <= 0.0;
                // Collision with a wall-mounted gate can stop the player just
                // short of its plane. Treat a close, forward approach as entry.
                let blocked_at_gate = !crossed_plane && d1.abs() <= 0.65 && d0.abs() > d1.abs();
                if !crossed_plane && !blocked_at_gate { continue; }
                let t = if crossed_plane { (d0 / (d0 - d1)).clamp(0.0, 1.0) } else { 1.0 };
                let crossing = start.lerp(end, t);
                let offset = crossing - source.pos;
                let lateral = offset.dot(right);
                let vertical = offset.y;
                if lateral.abs() > 1.6 || vertical.abs() > 1.55 { continue; }
                let side = if d0 >= 0.0 { 1.0 } else { -1.0 };
                let exit_normal = glam::Vec3::new(destination.yaw.cos(), 0.0, destination.yaw.sin());
                let exit_right = glam::Vec3::new(-exit_normal.z, 0.0, exit_normal.x);
                let exit = destination.pos + exit_normal * side * 1.35 + exit_right * lateral + glam::Vec3::Y * vertical;
                let yaw_delta = destination.yaw - source.yaw + std::f32::consts::PI;
                player.portal_teleport(exit - glam::Vec3::Y * 0.9, yaw_delta);
                camera.yaw += yaw_delta;
                weapons.add_flash(source.pos, 1.8);
                weapons.add_flash(destination.pos, 1.8);
                return;
            }
        }
    }

    /// Place the two endpoints of a linked Rift Gate pair.
    fn place_rift_gate(&mut self) {
        let Some(target) = self.spell_target() else {
            self.notify("Aim at the ground or a surface to place a Rift Gate.");
            self.audio.play(audio::Sfx::Denied);
            return;
        };
        let point = net::GatePoint { pos: target + glam::Vec3::Y * 1.25, yaw: self.camera.yaw };
        if let Some(first) = self.rift_first.take() {
            if first.pos.distance(point.pos) < 6.0 {
                self.rift_first = Some(first);
                self.notify("Place the second gate farther away.");
                self.audio.play(audio::Sfx::Denied);
                return;
            }
            let pair = net::GatePair { a: first, b: point };
            let owner = self.me().min(1);
            self.rift_gates[owner] = Some(pair);
            self.weapons.set_rift_gate(owner, Some(pair));
            if let Some(net) = self.net.as_ref().filter(|n| n.connected()) {
                net.send(&net::Msg::RiftGate { owner: owner as u8, pair });
            }
            self.weapons.add_flash(first.pos, 2.5);
            self.weapons.add_flash(point.pos, 2.5);
            self.notify("Rift Gate linked.");
        } else {
            self.rift_first = Some(point);
            self.rift_wait = PENDING_TIME;
            self.weapons.add_flash(point.pos, 2.0);
            self.audio.play(audio::Sfx::Click);
        }
    }

    /// Dispel shatters the structure under the crosshair, including your own.
    /// Its second rank also clears nearby structures around the impact point.
    fn dispel(&mut self, moves: &abilities::Moveset) {
        let origin = self.camera.position;
        let dir = self.camera.forward();
        let Some((distance, id)) = self.structures.blockers().hit(origin, dir, SPELL_AIM_RANGE) else {
            self.notify("Dispel found no structure. Aim at a barrier or trap.");
            self.audio.play(audio::Sfx::Denied);
            return;
        };
        let impact = origin + dir * distance;
        let mut ids = vec![id];
        {
            let radius = 12.0 * moves.spell_power.sqrt();
            for nearby in self.structures.near(impact, radius) {
                if !ids.contains(&nearby) {
                    ids.push(nearby);
                }
            }
        }
        for target in ids {
            self.unbuild(target, true);
        }
        self.weapons.add_flash(impact, 2.2 * moves.spell_power);
        self.audio.play_at(audio::Sfx::Shatter, impact);
    }

    /// Chain Lightning: from your target to the next and the next.
    fn chain_lightning(&mut self, moves: &abilities::Moveset) {
        let power = moves.spell_power * 1.5 * self.spell_boost;
        let strong = if moves.lightning_jumps >= 16 { 1.4 } else { 1.0 };
        let damage = 15.0 * moves.weapon_damage * power * strong;
        // A planted spell: a bolt from its circle to where you aimed it,
        // then on from there.
        let rune = self.spell_at.map(|at| {
            let (start, end) = (at.origin + glam::Vec3::Y * 1.4, at.target + glam::Vec3::Y * 1.4);
            if start.distance(end) > 1.0 {
                self.weapons.add_beam(start, end);
            }
            end
        });
        let mut from = rune.unwrap_or(self.spell_from.unwrap_or(self.player.pos) + glam::Vec3::Y * 1.4);
        let mut struck: Vec<usize> = Vec::new();
        let mut any = false;
        // The other player first, if you're looking at them.
        if let Some(them) = self.aimed_player(f32::INFINITY).filter(|_| rune.is_none()) {
            let at = them + glam::Vec3::Y;
            self.weapons.add_beam(from, at);
            if let Some(net) = self.net.as_ref().filter(|n| n.connected()) {
                // Weapon 254: lightning straight at them (damage already for players).
                net.send(&net::Msg::Fire(net::FireMsg {
                    weapon: 254,
                    origin: from,
                    dir: glam::Vec3::Y,
                    damage: 2.5 * damage,
                    pellets: 0,
                    blast_radius: 0.0,
                    mods: 0,
                    homing: false,
                }));
            }
            from = at;
            any = true;
        }
        let authority = self.authority();
        let first = if rune.is_some() { None } else { self.aimed_drones(SPELL_AIM_RANGE).first().map(|&(i, _)| i) };
        for jump in 0..moves.lightning_jumps {
            let next = if jump == 0 && !any && first.is_some() {
                first
            } else {
                let reach = if any { 22.0 } else { 40.0 } * self.spell_boost.sqrt();
                self.drones
                    .within(from, reach)
                    .into_iter()
                    .filter(|(i, _)| !struck.contains(i))
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(i, _)| i)
            };
            let Some(i) = next else { break };
            let Some(p) = self.drones.position(i) else { break };
            self.weapons.add_beam(from, p);
            struck.push(i);
            any = true;
            if authority {
                let zap = drones::Effect { shock_every: 1, shock_bonus: 2.0 * power, stun: 1.2, ..Default::default() };
                let r = self.drones.hit(i, damage, &zap, false);
                for at in [r.killed, r.chain_kill].into_iter().flatten() {
                    self.weapons.add_blast(at, 1.6);
                    self.weapons.kills += 1;
                    self.extra_kills += 1;
                }
            }
            from = p;
        }
        if any {
            self.audio.play(audio::Sfx::Thunder);
            self.add_shake(0.15);
        } else {
            self.notify("The lightning found nothing to strike.");
        }
    }

    /// Uses a skill bound to its own key (if equipped and ready).
    fn activate(&mut self, skill: abilities::Skill) {
        let before = self.cooldowns[skill.index()];
        self.activate_once(skill);
        // It went off if it just went on cooldown (spells only start their
        // cast here; they echo when the cast lands).
        if before <= 0.0 && self.cooldowns[skill.index()] > 0.0 {
            arm_echo(&mut self.echo_charges, &mut self.echo_repeat, skill, glam::Vec3::ZERO);
        }
    }

    /// Chrono Echo's repeat of `skill`, free: no cooldown, charge or cast.
    fn run_echo(&mut self, skill: abilities::Skill, dir: glam::Vec3) {
        use abilities::Skill;
        let moves = self.moves();
        match skill {
            Skill::Dash => {
                let (colliders, terrain) = (&self.colliders, &self.scene.terrain);
                let moved = if moves.dash_any_direction {
                    self.player.blink_free(dir, moves.dash_distance, colliders, terrain)
                } else {
                    self.player.blink(dir, moves.dash_distance, colliders, terrain)
                };
                if moved {
                    self.body.reset_motion();
                    self.audio.play(audio::Sfx::Dash);
                    self.feel.fov_kick += 8.0;
                }
            }
            Skill::Pad => {
                self.gadgets.place_pad(self.player.pos, moves.pads);
                self.audio.play(audio::Sfx::PadPlace);
            }
            Skill::Clone => {
                let spots = self.clone_circle(&moves);
                Self::spawn_clone(&mut self.clones, &mut self.renderer, &spots, self.camera.yaw, moves.max_clones);
                self.audio.play(audio::Sfx::Clone);
            }
            Skill::Meteor
            | Skill::ChainLightning
            | Skill::Blizzard
            | Skill::ArcaneWard
            | Skill::Dispel
            | Skill::Fireball
            | Skill::ArcaneMissiles
            | Skill::FrostNova
            | Skill::ArcaneLance
            | Skill::Rejuvenate => self.cast_spell(skill),
            _ => {
                let saved = self.cooldowns[skill.index()];
                self.cooldowns[skill.index()] = 0.0;
                self.activate_once(skill);
                self.cooldowns[skill.index()] = self.cooldowns[skill.index()].max(saved);
            }
        }
        self.weapons.add_flash(self.player.pos + glam::Vec3::Y, 0.9);
        self.audio.play(audio::Sfx::Swap);
    }

    /// Uses a skill once (if equipped and ready); see [`App::activate`].
    fn activate_once(&mut self, skill: abilities::Skill) {
        use abilities::Skill;
        let moves = self.moves();
        match skill {
            Skill::Barrier | Skill::Ceiling | Skill::Dome | Skill::GiantBox | Skill::HealPad | Skill::GravityWell => {
                let have = match skill {
                    Skill::Barrier => moves.barriers > 0,
                    Skill::Ceiling => moves.ceiling,
                    Skill::Dome => moves.dome,
                    Skill::GiantBox => moves.giant_box > 0,
                    Skill::HealPad => moves.heal_pad > 0.0,
                    _ => moves.gravity_well > 0.0,
                };
                if have {
                    self.build(skill);
                }
            }
            Skill::SplitShot if moves.split > 0 && self.ready(skill) => {
                self.split_left = moves.split_time;
                self.cooldowns[skill.index()] = skill_cooldown(skill) + moves.split_time;
                self.audio.play(audio::Sfx::Clone);
                self.notify(format!("Split Shot: {} shots per trigger pull", moves.split));
            }
            Skill::Pull if moves.pull > 0.0 && self.ready(skill) => self.pull(&moves),
            Skill::Repulse if moves.repulse > 0.0 && self.ready(skill) => self.repulse(&moves),
            Skill::GroundPull if moves.ground_pull > 0.0 && self.ready(skill) => self.ground_pull(&moves),
            Skill::RapidFire if moves.rapid_fire > 0 && self.ready(skill) => self.start_buff(skill, &moves),
            Skill::IronSkin if moves.iron_skin > 0 && self.ready(skill) => self.start_buff(skill, &moves),
            Skill::VampiricAura if moves.vampiric > 0 && self.ready(skill) => self.start_buff(skill, &moves),
            Skill::StunGrenade if moves.stun_grenade > 0 && self.ready(skill) => self.stun_grenade(&moves),
            Skill::Whirlwind if moves.whirlwind > 0 && self.ready(skill) => self.whirlwind(&moves),
            Skill::Stasis if moves.stasis > 0 && self.ready(skill) => self.stasis(&moves),
            Skill::Mend if moves.mend > 0 && !self.dead && self.ready(skill) => {
                self.heal(if moves.mend >= 2 { 55.0 } else { 35.0 }, false);
                self.weapons.add_flash(self.player.pos + glam::Vec3::Y, 1.0);
                self.cooldowns[skill.index()] = skill_cooldown(skill);
            }
            Skill::Leap if moves.leap > 0 && !self.dead && self.ready(skill) => {
                let forward = glam::Vec3::new(self.camera.yaw.cos(), 0.0, self.camera.yaw.sin());
                let (out, up) = if moves.leap >= 2 { (30.0, 16.0) } else { (22.0, 12.0) };
                self.player.push(forward * out + glam::Vec3::Y * up);
                self.audio.play(audio::Sfx::Jump);
                self.feel.fov_kick += 8.0;
                self.cooldowns[skill.index()] = skill_cooldown(skill);
            }
            spell if spell.cast_time().is_some() => {
                let have = match spell {
                    Skill::Meteor => moves.meteors > 0,
                    Skill::ChainLightning => moves.lightning_jumps > 0,
                    Skill::Blizzard => moves.blizzard > 0,
                    Skill::ArcaneWard => moves.ward > 0.0,
                    Skill::TimeWarp => moves.bullet_time && self.time_effect.is_none() && self.time_cooldown <= 0.0,
                    Skill::Dispel => moves.dispel > 0,
                    Skill::ChronoEcho => moves.chrono_echo > 0,
                    Skill::RiftGate => moves.rift_gate,
                    Skill::Fireball => moves.fireball > 0,
                    Skill::ArcaneMissiles => moves.missiles > 0,
                    Skill::FrostNova => moves.frost_nova > 0,
                    Skill::ArcaneLance => moves.lance > 0,
                    Skill::Rejuvenate => moves.rejuvenate > 0,
                    Skill::Haste => moves.haste > 0,
                    Skill::ShadowStep => moves.shadow_step > 0,
                    Skill::ArcaneClone => moves.arcane_clones > 0,
                    _ => moves.recall_home,
                };
                if !have {
                    return;
                }
                // Its rune is lying out there: this press casts it.
                if self.casting.is_none()
                    && let Some(i) = self.runes.iter().position(|r| r.spell == spell)
                {
                    let rune = self.runes.remove(i);
                    self.release_rune(rune);
                    return;
                }
                // Pressing the key again while casting calls it off.
                if let Some((casting, _, _)) = self.casting {
                    if casting == spell {
                        self.casting = None;
                        self.notify(format!("{} cancelled", spell.name()));
                    }
                    return;
                }
                if !self.ready(spell) {
                    return;
                }
                // A rift gate's first end goes down at once; only linking
                // the pair takes the cast.
                if spell == Skill::RiftGate && self.rift_first.is_none() {
                    self.place_rift_gate();
                    return;
                }
                // Planted spells need ground under you to plant them on,
                // unless you can cast in the air.
                if rune_able(spell) && !self.player.grounded() && !moves.air_cast {
                    self.notify("You need solid ground to plant a spell (Air Casting lets you cast mid-air).");
                    self.audio.play(audio::Sfx::Denied);
                    return;
                }
                self.cast_origin = self.player.pos;
                let total = spell.cast_time().unwrap_or(1.0) / moves.cast_speed;
                // Your arcane clones raise their hands with you.
                for pos in self.clones.arcane() {
                    self.weapons.add_flash(pos + glam::Vec3::Y * 1.4, 0.7);
                }
                self.casting = Some((spell, 0.0, total));
                self.audio.play(audio::Sfx::Cast);
            }
            _ => {}
        }
    }

    /// Tether: yank the other player or drones in your sights towards you.
    fn pull(&mut self, moves: &abilities::Moveset) {
        let me = self.player.pos + glam::Vec3::Y;
        let mut pulled = false;
        if let Some(them) = self.aimed_player(50.0) {
            let dir = (me - them).normalize_or_zero();
            self.push_other(dir * moves.pull + glam::Vec3::Y * (4.0 + moves.pull * 0.15), 0.0, moves.pull_stun);
            self.weapons.add_flash(them + glam::Vec3::Y, 1.2);
            pulled = true;
        }
        for (i, p) in self.aimed_drones(50.0) {
            self.drones.push(i, (me - p).normalize_or_zero() * moves.pull * 0.8, moves.pull_stun.max(0.3));
            pulled = true;
        }
        if pulled {
            self.cooldowns[abilities::Skill::Pull.index()] = skill_cooldown(abilities::Skill::Pull);
            self.audio.play(audio::Sfx::Grapple);
        } else {
            self.notify("Nothing to pull: aim at the other player or a drone.");
            self.audio.play(audio::Sfx::Denied);
        }
    }

    /// Repulse: throw everything near away, and wipe out drone shots.
    fn repulse(&mut self, moves: &abilities::Moveset) {
        let center = self.player.pos + glam::Vec3::Y;
        let r = moves.repulse_radius;
        if self.net.as_ref().is_some_and(|n| n.connected()) && self.remote.state.is_some_and(|s| s.health > 0) {
            let to = self.remote.pos + glam::Vec3::Y - center;
            if to.length() < r {
                let away = glam::Vec3::new(to.x, 0.0, to.z).normalize_or(glam::Vec3::X);
                self.push_other(away * moves.repulse + glam::Vec3::Y * moves.repulse * 0.45, 0.0, 0.0);
            }
        }
        for (i, _) in self.drones.within(center, r) {
            if let Some(p) = self.drones.position(i) {
                self.drones.push(i, (p - center).normalize_or(glam::Vec3::Y) * moves.repulse, 0.6);
            }
        }
        if moves.repulse_damage > 0.0 && self.authority() {
            // Shockwave: it hurts as well.
            let hit = self.weapons.shockwave(center, moves.repulse_damage * moves.weapon_damage * 0.25, false, true, &mut self.drones);
            self.send_blast(center, 8.0, moves.repulse_damage * moves.weapon_damage, 0.0);
            self.extra_kills += hit.kills;
        }
        self.drones.clear_orbs_near(center, r);
        self.weapons.add_flash(center, r * 0.6);
        self.cooldowns[abilities::Skill::Repulse.index()] = skill_cooldown(abilities::Skill::Repulse);
        self.audio.play(audio::Sfx::Slam);
        self.add_shake(0.25);
    }

    /// Starts a timed buff (Rapid Fire, Iron Skin, Vampiric Aura).
    fn start_buff(&mut self, skill: abilities::Skill, moves: &abilities::Moveset) {
        use abilities::Skill;
        self.buff_left[skill.index()] = buff_time(skill, moves);
        self.cooldowns[skill.index()] = skill_cooldown(skill);
        let (sound, line) = match skill {
            Skill::RapidFire => (audio::Sfx::Boost, "Rapid Fire!"),
            Skill::IronSkin => (audio::Sfx::ShieldHit, "Iron Skin: you take less damage."),
            _ => (audio::Sfx::Crit, "Vampiric Aura: your damage heals you."),
        };
        self.audio.play(sound);
        self.weapons.add_flash(self.player.pos + glam::Vec3::Y, 1.1);
        self.notify(line);
    }

    /// Stun Grenade: a burst where you aim that hurts and stuns.
    fn stun_grenade(&mut self, moves: &abilities::Moveset) {
        let (eye, aim) = (self.camera.position, self.camera.forward());
        let at = self.aimed_player(35.0).or_else(|| gadgets::raycast(eye, aim, 35.0, &self.colliders, &self.scene.terrain)).unwrap_or(eye + aim * 35.0);
        let cluster = moves.stun_grenade >= 2;
        let r = if cluster { 4.5 } else { 3.0 };
        let stun = drones::Effect { stun: 2.5, ..Default::default() };
        let damage = if cluster { 10.0 } else { 6.0 } * moves.weapon_damage;
        let authority = self.authority();
        let hit = self.weapons.burst(at, r, damage, &stun, false, authority, &mut self.drones);
        self.send_blast(at, r, damage, 1.5);
        self.extra_kills += hit.kills;
        if self.net.as_ref().is_some_and(|n| n.connected()) && self.remote.pos.distance(at) < r {
            self.push_other(glam::Vec3::Y * 4.0, 0.0, 0.0);
        }
        self.weapons.add_blast(at, r * 0.6);
        self.audio.play_at(audio::Sfx::Explosion(r), at);
        self.cooldowns[abilities::Skill::StunGrenade.index()] = skill_cooldown(abilities::Skill::StunGrenade);
    }

    /// Whirlwind: strike everything within 5 m and knock it away (Cyclone:
    /// harder, and it stuns).
    fn whirlwind(&mut self, moves: &abilities::Moveset) {
        let center = self.player.pos + glam::Vec3::Y;
        let cyclone = moves.whirlwind >= 2;
        let effect = drones::Effect { stun: if cyclone { 1.2 } else { 0.0 }, ..Default::default() };
        let damage = if cyclone { 12.0 } else { 7.0 } * moves.weapon_damage * moves.melee_damage;
        let reach = WHIRLWIND_REACH;
        let authority = self.authority();
        let hit = self.weapons.burst(center, reach, damage, &effect, false, authority, &mut self.drones);
        self.send_blast(center, reach, damage, if cyclone { 1.0 } else { 0.0 });
        self.extra_kills += hit.kills;
        for (i, _) in self.drones.within(center, reach) {
            if let Some(p) = self.drones.position(i) {
                self.drones.push(i, (p - center).normalize_or(glam::Vec3::Y) * 14.0, 0.0);
            }
        }
        if self.net.as_ref().is_some_and(|n| n.connected()) && self.remote.state.is_some_and(|s| s.health > 0) {
            let to = self.remote.pos + glam::Vec3::Y - center;
            if to.length() < reach {
                let away = glam::Vec3::new(to.x, 0.0, to.z).normalize_or(glam::Vec3::X);
                self.push_other(away * 14.0 + glam::Vec3::Y * 4.0, 0.0, 0.0);
            }
        }
        self.weapons.add_flash(center, 3.0);
        self.audio.play(audio::Sfx::HeavySwing);
        self.add_shake(0.15);
        self.cooldowns[abilities::Skill::Whirlwind.index()] = skill_cooldown(abilities::Skill::Whirlwind);
    }

    /// Stasis Field: the drone or player you aim at stops dead (Time
    /// Prison: everything round them too).
    fn stasis(&mut self, moves: &abilities::Moveset) {
        let (eye, aim) = (self.camera.position, self.camera.forward());
        let player = self.aimed_player(60.0);
        let drone = self.drones.ray_hit(eye, aim, 60.0).and_then(|(i, _)| self.drones.position(i).map(|p| (i, p)));
        let at = match (player, drone) {
            (Some(feet), _) => feet + glam::Vec3::Y,
            (None, Some((_, p))) => p,
            (None, None) => {
                self.notify("Stasis needs a target: aim at a drone or the other player.");
                self.audio.play(audio::Sfx::Denied);
                return;
            }
        };
        let around = if moves.stasis >= 2 { 4.0 } else { 0.0 };
        if let Some((i, _)) = drone.filter(|_| player.is_none()) {
            self.drones.push(i, glam::Vec3::ZERO, 5.0);
        }
        if around > 0.0 {
            for (i, _) in self.drones.within(at, around) {
                self.drones.push(i, glam::Vec3::ZERO, 5.0);
            }
        }
        let them = self.net.as_ref().is_some_and(|n| n.connected()) && self.remote.pos.distance(at) < around + 1.5;
        if player.is_some() || them {
            self.push_other(glam::Vec3::ZERO, 0.0, 2.5);
        }
        self.drones.clear_orbs_near(at, around.max(1.5));
        self.weapons.add_flash(at, 1.5 + around);
        self.audio.play_at(audio::Sfx::TimeStop, at);
        self.cooldowns[abilities::Skill::Stasis.index()] = skill_cooldown(abilities::Skill::Stasis);
    }

    /// Ground Pull: drag the target down and pin it (Earthbind: everything
    /// around where you aim).
    fn ground_pull(&mut self, moves: &abilities::Moveset) {
        let pin = moves.ground_pull;
        let mut hit = None;
        if moves.ground_pull_area > 0.0 {
            if let Some(at) = self.build_target() {
                let r = moves.ground_pull_area;
                if self.net.as_ref().is_some_and(|n| n.connected()) && self.remote.pos.distance(at) < r {
                    self.push_other(glam::Vec3::NEG_Y * 30.0, pin, 0.0);
                }
                for (i, _) in self.drones.within(at, r) {
                    self.drones.push(i, glam::Vec3::NEG_Y * 25.0, pin);
                }
                hit = Some(at);
            }
        } else if let Some(them) = self.aimed_player(60.0) {
            self.push_other(glam::Vec3::NEG_Y * 30.0, pin, 0.0);
            hit = Some(them);
        } else if let Some(&(i, p)) = self.aimed_drones(60.0).first() {
            self.drones.push(i, glam::Vec3::NEG_Y * 25.0, pin);
            hit = Some(p);
        }
        match hit {
            Some(at) => {
                self.weapons.add_flash(at + glam::Vec3::Y * 0.5, 1.5 + moves.ground_pull_area * 0.4);
                self.audio.play_at(audio::Sfx::HammerHit, at);
                self.cooldowns[abilities::Skill::GroundPull.index()] = skill_cooldown(abilities::Skill::GroundPull);
            }
            None => {
                self.notify("Nothing to pull down: aim at the other player or a drone.");
                self.audio.play(audio::Sfx::Denied);
            }
        }
    }

    /// The point under the crosshair (any surface, or mid-air), up to `range` away.
    fn barrier_corner(&self, range: f32) -> glam::Vec3 {
        self.aim_point(range).0
    }

    /// Wherever you aim: on any surface (rooftops, crates, cliffs), or in
    /// mid-air at full range; never below the ground. True if it's on a
    /// surface.
    fn aim_point(&self, range: f32) -> (glam::Vec3, bool) {
        let eye = self.camera.position;
        let dir = self.camera.forward();
        let hit = gadgets::raycast(eye, dir, range, &self.colliders, &self.scene.terrain);
        let p = hit.unwrap_or(eye + dir * range);
        (glam::Vec3::new(p.x, p.y.max(self.scene.terrain.height_at(p.x, p.z)), p.z), hit.is_some())
    }

    /// A ceiling corner: aimed at a surface, the slab goes just above head
    /// height over it (low enough to stop jumps, or with Warden to pin);
    /// aimed into the air, exactly there.
    fn ceiling_corner(&self, moves: &abilities::Moveset) -> glam::Vec3 {
        let (p, on_surface) = self.aim_point(CEILING_REACH * moves.build_scale);
        if on_surface { p + glam::Vec3::Y * if moves.warden { 2.0 } else { 2.6 } } else { p }
    }

    /// Where the far corner of a ceiling goes from `first`: as aimed, but no
    /// wider or deeper than a standard ceiling.
    fn ceiling_far_corner(first: glam::Vec3, aimed: glam::Vec3, moves: &abilities::Moveset) -> glam::Vec3 {
        let max = structures::CEILING_SIZE * moves.build_scale;
        let dx = (aimed.x - first.x).clamp(-max, max);
        let dz = (aimed.z - first.z).clamp(-max, max);
        glam::Vec3::new(first.x + dx, aimed.y, first.z + dz)
    }

    /// Two presses (or a press and a click) set opposite corners of a
    /// custom ceiling.
    fn place_ceiling(&mut self, moves: &abilities::Moveset) {
        let aimed = self.ceiling_corner(moves);
        let Some(first) = self.ceiling_first.take() else {
            self.ceiling_first = Some(aimed);
            self.ceiling_wait = PENDING_TIME;
            self.audio.play(audio::Sfx::Click);
            return;
        };
        let second = Self::ceiling_far_corner(first, aimed, moves);
        let (w, d) = ((second.x - first.x).abs(), (second.z - first.z).abs());
        if w < CEILING_MIN || d < CEILING_MIN {
            self.ceiling_first = Some(first);
            self.notify(format!("Make the ceiling at least {CEILING_MIN:.0} m each way."));
            self.audio.play(audio::Sfx::Denied);
            return;
        }
        let pos = glam::Vec3::new((first.x + second.x) * 0.5, first.y.max(second.y), (first.z + second.z) * 0.5);
        let health = 200.0 * moves.build_health;
        let me = self.me();
        let ceiling = structures::Structure {
            id: self.structures.new_id(me),
            kind: structures::Kind::Ceiling,
            pos,
            // A custom ceiling carries its depth in yaw and width in power
            // (see Structure::ceiling_size), so peers build the same slab.
            yaw: d,
            health,
            max_health: health,
            life: 10.0 * moves.build_life,
            warden: false,
            scale: moves.build_scale,
            power: w,
        };
        self.structures.add(ceiling);
        if let Some(net) = self.net.as_ref().filter(|n| n.connected()) {
            net.send(&net::Msg::Build(ceiling));
        }
        self.cooldowns[abilities::Skill::Ceiling.index()] = skill_cooldown(abilities::Skill::Ceiling);
        self.notify(format!("Ceiling built · {w:.0} × {d:.0} m"));
        self.audio.play_at(audio::Sfx::Build, pos);
    }

    /// The two-part ability waiting on its second piece, if any (what a
    /// left click confirms).
    fn pending_skill(&self) -> Option<abilities::Skill> {
        use abilities::Skill;
        if self.barrier_first.is_some() {
            Some(Skill::Barrier)
        } else if self.ceiling_first.is_some() {
            Some(Skill::Ceiling)
        } else if self.rift_first.is_some() && self.casting.is_none() {
            Some(Skill::RiftGate)
        } else {
            None
        }
    }

    /// Two presses set the endpoints of one custom barrier.
    fn place_barrier(&mut self, moves: &abilities::Moveset) {
        let range = 20.0 * moves.build_scale;
        let corner = self.barrier_corner(range);
        let Some(first) = self.barrier_first.take() else {
            self.barrier_first = Some(corner);
            self.barrier_wait = PENDING_TIME;
            self.audio.play(audio::Sfx::Click);
            return;
        };

        let mut span = glam::Vec3::new(corner.x - first.x, 0.0, corner.z - first.z);
        let mut width = span.length();
        if width < 1.0 {
            self.barrier_first = Some(first);
            self.notify("Place the second corner at least 1 m away.");
            self.audio.play(audio::Sfx::Denied);
            return;
        }
        if width > range {
            span = span.normalize() * range;
            width = range;
        }

        let me = self.me();
        let mine = self.structures.mine(structures::Kind::Barrier, me);
        if mine.len() >= moves.barriers as usize {
            self.unbuild(mine[0], false);
        }
        let end = first + span;
        let mut pos = (first + end) * 0.5;
        // The wall stands from the lower of its two corners.
        pos.y = first.y.min(corner.y);
        let yaw = (-span.x).atan2(span.z);
        let health = 220.0 * moves.build_health;
        let life = 20.0 * moves.build_life;
        let barrier = structures::Structure {
            id: self.structures.new_id(me),
            kind: structures::Kind::Barrier,
            pos,
            yaw,
            health,
            max_health: health,
            life,
            warden: false,
            scale: moves.build_scale,
            // Barrier power is otherwise unused; here it carries the custom
            // width so peers render and collide with the same wall length.
            power: width,
        };
        self.structures.add(barrier);
        if let Some(net) = self.net.as_ref().filter(|n| n.connected()) {
            net.send(&net::Msg::Build(barrier));
        }
        // Charges: the cooldown only starts once the last one is used.
        let up = self.structures.mine(structures::Kind::Barrier, me).len();
        if up >= moves.barriers as usize {
            self.cooldowns[abilities::Skill::Barrier.index()] = skill_cooldown(abilities::Skill::Barrier);
        }
        self.notify(format!("Barrier built · {width:.0} m · {up}/{}", moves.barriers));
        self.audio.play_at(audio::Sfx::Build, pos + glam::Vec3::Y);
    }

    /// Counts down a half-placed barrier or rift gate (cancelling it when
    /// time runs out) and draws the preview of what the next press makes.
    fn tick_pending(&mut self, dt: f32, moves: &abilities::Moveset) {
        if self.barrier_first.is_some() {
            self.barrier_wait -= dt;
            if self.barrier_wait <= 0.0 || self.dead {
                self.barrier_first = None;
                self.notify("Barrier cancelled.");
                self.audio.play(audio::Sfx::Denied);
            }
        }
        if self.ceiling_first.is_some() {
            self.ceiling_wait -= dt;
            if self.ceiling_wait <= 0.0 || self.dead {
                self.ceiling_first = None;
                self.notify("Ceiling cancelled.");
                self.audio.play(audio::Sfx::Denied);
            }
        }
        if self.rift_first.is_some() {
            self.rift_wait -= dt;
            if self.rift_wait <= 0.0 || self.dead {
                self.rift_first = None;
                self.notify("Rift Gate cancelled.");
                self.audio.play(audio::Sfx::Denied);
            }
        }
        let barrier = self.barrier_first.map(|first| {
            let range = 20.0 * moves.build_scale;
            let mut second = self.barrier_corner(range);
            let span = glam::Vec3::new(second.x - first.x, 0.0, second.z - first.z);
            if span.length() > range {
                second = first + span.normalize() * range;
            }
            placement::BarrierPlan {
                first,
                second,
                height: structures::BARRIER_HEIGHT * moves.build_scale,
                invalid: span.length() < 1.0,
            }
        });
        let gate = self.rift_first.map(|first| {
            let second = self.spell_target().map(|t| (t + glam::Vec3::Y * 1.25, self.camera.yaw));
            placement::GatePlan {
                first: first.pos,
                first_yaw: first.yaw,
                second,
                invalid: second.is_some_and(|(p, _)| p.distance(first.pos) < 6.0),
            }
        });
        let ceiling = self.ceiling_first.map(|first| {
            let second = Self::ceiling_far_corner(first, self.ceiling_corner(moves), moves);
            let small = (second.x - first.x).abs() < CEILING_MIN || (second.z - first.z).abs() < CEILING_MIN;
            placement::CeilingPlan { first, second, invalid: small }
        });
        self.preview.update(&mut self.renderer, dt, barrier, ceiling, gate);
    }

    /// The HUD prompt for a half-placed ability.
    fn pending_prompt(&self) -> Option<ui::Pending> {
        use abilities::Skill;
        let key = |s: Skill| self.binds()[s.index()].map_or_else(|| "its key".to_owned(), |b| b.label());
        if self.barrier_first.is_some() {
            return Some(ui::Pending {
                title: "Barrier".into(),
                hint: format!("Aim at the far end, then click or press {}", key(Skill::Barrier)),
                step: (1, 2),
                left: self.barrier_wait / PENDING_TIME,
            });
        }
        if self.ceiling_first.is_some() {
            return Some(ui::Pending {
                title: "Ceiling".into(),
                hint: format!("Aim at the opposite corner, then click or press {}", key(Skill::Ceiling)),
                step: (1, 2),
                left: self.ceiling_wait / PENDING_TIME,
            });
        }
        self.rift_first.map(|_| ui::Pending {
            title: "Rift Gate".into(),
            hint: format!("Aim 6 m+ away, then click or cast {} to link", key(Skill::RiftGate)),
            step: (1, 2),
            left: self.rift_wait / PENDING_TIME,
        })
    }

    /// Builds a barrier (in front of you), ceiling, dome or gravity well
    /// (on your target) or a healing pad (at your feet), for both players
    /// to see. Everything grows with your Ascension.
    fn build(&mut self, skill: abilities::Skill) {
        use abilities::Skill;
        use structures::Kind;
        let moves = self.moves();
        let kind = match skill {
            Skill::Barrier => Kind::Barrier,
            Skill::Ceiling => Kind::Ceiling,
            Skill::Dome => Kind::Dome,
            Skill::HealPad => Kind::HealPad,
            Skill::GravityWell => Kind::Well,
            Skill::GiantBox => Kind::Box,
            _ => return,
        };
        if self.dead || !self.ready(skill) {
            return;
        }
        if kind == Kind::Barrier {
            self.place_barrier(&moves);
            return;
        }
        if kind == Kind::Ceiling {
            self.place_ceiling(&moves);
            return;
        }
        let me = self.me();
        let warden = moves.warden;
        let scale = moves.build_scale;
        let tough = moves.build_health;
        let lasting = moves.build_life;
        let (mut pos, yaw, mut power, mut upgraded) = (self.player.pos, 0.0, 0.0, false);
        let mut box_scale = 1.0;
        let (health, life, _) = match kind {
            Kind::Barrier => unreachable!("barriers use two-point placement"),
            Kind::HealPad => {
                power = moves.heal_pad;
                upgraded = moves.heal_pad_shield;
                (1.0, moves.heal_pad_life, 20.0)
            }
            Kind::Ceiling | Kind::Dome | Kind::Well | Kind::Storm | Kind::Box => {
                // The Giant Box drops round you if you aim at nothing.
                let target = self.build_target().or((kind == Kind::Box).then_some(self.player.pos));
                let Some(target) = target else {
                    self.notify("Nothing in reach: aim at the other player or the ground.");
                    self.audio.play(audio::Sfx::Denied);
                    return;
                };
                pos = target;
                match kind {
                    Kind::Ceiling => {
                        // Low enough to stop jumps (or, with Warden, pin).
                        pos += glam::Vec3::Y * if warden { 2.0 } else { 2.6 };
                        (200.0 * tough, 10.0 * lasting, 12.0)
                    }
                    Kind::Dome => {
                        upgraded = warden;
                        (250.0 * tough, 12.0 * lasting, 18.0)
                    }
                    Kind::Box => {
                        // Citadel: a third bigger, twice as tough, standing longer.
                        let citadel = moves.giant_box >= 2;
                        if citadel {
                            box_scale = 1.3;
                        }
                        (1500.0 * tough * if citadel { 2.0 } else { 1.0 }, if citadel { 30.0 } else { 20.0 } * lasting, 45.0)
                    }
                    _ => {
                        power = moves.gravity_well;
                        (1.0, moves.well_life, 18.0)
                    }
                }
            }
        };
        let s = structures::Structure {
            id: self.structures.new_id(me),
            kind,
            pos,
            yaw,
            health,
            max_health: health,
            life,
            warden: upgraded,
            scale: scale * box_scale,
            power,
        };
        self.structures.add(s);
        if let Some(net) = self.net.as_ref().filter(|n| n.connected()) {
            net.send(&net::Msg::Build(s));
        }
        self.cooldowns[skill.index()] = skill_cooldown(skill);
        self.audio.play_at(audio::Sfx::Build, pos + glam::Vec3::Y);
    }

    /// A skill is off cooldown (else a "not yet" buzz).
    fn ready(&mut self, skill: abilities::Skill) -> bool {
        if self.cooldowns[skill.index()] > 0.0 {
            self.audio.play(audio::Sfx::Denied);
            return false;
        }
        true
    }

    /// Takes a structure down (`shatter`: it broke, with a burst).
    fn unbuild(&mut self, id: u32, shatter: bool) {
        if let Some(s) = self.structures.remove(id) {
            if shatter {
                let at = s.top() - glam::Vec3::Y * 0.8;
                self.weapons.add_blast(at, 1.9);
                self.audio.play_at(audio::Sfx::Shatter, at);
            }
            if let Some(net) = self.net.as_ref().filter(|n| n.connected()) {
                net.send(&net::Msg::Unbuild(id));
            }
        }
    }

    /// Damage to structures this frame: shots, blasts, drone fire and
    /// melee. Your own structures soaking damage counts for Warden's trial.
    fn damage_structures(&mut self, hits: Vec<(u32, f32)>) {
        let tag = (self.me() as u32 + 1) << 24;
        let mut pinged = false;
        for (id, amount) in hits {
            if id & 0xff00_0000 == tag
                && let Some(u) = self.progress.record(abilities::Stat::Absorbed, amount)
            {
                self.unlocked.push(u);
            }
            if self.structures.damage(id, amount) {
                self.unbuild(id, true);
            } else if !pinged {
                pinged = true;
                self.audio.play(audio::Sfx::ShieldHit);
            }
        }
    }

    /// Health, shield and timers over time.
    fn update_vitals(&mut self, dt: f32) {
        let moves = self.moves();
        let v = &mut self.vitals;
        v.since_hurt += dt;
        v.invulnerable = (v.invulnerable - dt).max(0.0);
        v.second_wind = (v.second_wind - dt).max(0.0);
        v.heal_flash = (v.heal_flash - dt * 2.0).max(0.0);
        v.ward_time = (v.ward_time - dt).max(0.0);
        if v.ward_time <= 0.0 {
            v.ward = 0.0;
        }
        if !self.dead {
            if v.since_hurt > moves.regen_delay && self.health < moves.max_health {
                self.health = (self.health + moves.regen_rate * dt).min(moves.max_health);
            }
            if v.since_hurt > 3.0 {
                v.shield = (v.shield + 15.0 * dt).min(moves.shield);
            }
            v.shield = v.shield.min(moves.shield);
            self.health = self.health.min(moves.max_health);
        }
        // The pale trail lingers, then drains down to the health.
        if v.trail < self.health || v.since_hurt > 0.5 {
            v.trail = (v.trail - 45.0 * dt).max(self.health);
        }
        if let Some(t) = &mut v.downed {
            *t -= dt;
            if *t <= 0.0 {
                self.revive_in_village();
            }
        }
    }

    /// A capstone trial finished.
    fn trial_done(&mut self, unlock: abilities::Unlock) {
        self.progress.save();
        let what = match unlock {
            abilities::Unlock::Skill(_) => "Esc > Skills",
            abilities::Unlock::Item(_) => "Esc > Items",
        };
        self.announce(
            "TRIAL COMPLETE",
            format!("{} can now be unlocked: {what}", unlock.name()),
            ui::BANNER_GOLD,
            audio::Sfx::Unlock,
        );
    }

    /// Host: announce who took the round.
    fn round_result(&mut self) {
        if let Some(w) = self.arena.state.winner {
            let me = self.net.as_ref().map_or(0, |n| n.role.color_index());
            let [a, b] = self.arena.state.scores;
            let score = format!("Gold {a} - {b} Teal");
            if w as usize == me {
                self.announce("ROUND WON", score, ui::BANNER_GREEN, audio::Sfx::Victory);
            } else {
                self.announce("ROUND LOST", score, ui::BANNER_RED, audio::Sfx::Defeat);
            }
        }
    }

    /// Every player's progress back to the start, and the score to 0-0.
    /// `tell_other`: this player pressed the button (pass it on).
    fn reset_everyone(&mut self, tell_other: bool) {
        // Keep a copy, just in case.
        let backup = settings::Settings::file_path().with_file_name("progress.backup.cfg");
        let _ = std::fs::write(&backup, self.progress.serialize());
        self.progress = abilities::Progress::default();
        self.progress.save();
        self.weapons.kills = 0;
        self.rift_gates = [None, None];
        self.rift_first = None;
        self.weapons.set_rift_gate(0, None);
        self.weapons.set_rift_gate(1, None);
        self.echo_charges = 0;
        self.echo_repeat = None;
        self.haste_left = 0.0;
        self.runes.clear();
        (self.threat_time, self.threat_kills, self.threat) = (0.0, 0, 0);
        self.buff_left = [0.0; abilities::SKILLS];
        self.clones.dismiss_all(&mut self.renderer);
        self.gadgets.clear_pads();
        self.marker = None;
        if let Some(net) = &self.net {
            if tell_other && net.connected() {
                net.send(&net::Msg::ResetAll);
            }
            if net.is_host() && self.arena.reset() == arena::Event::NewRound {
                self.start_round();
            }
        }
        self.notify("All players were reset: fresh progress, score 0-0.");
    }

    /// Sends this player's state (and, hosting, the drones) when due.
    fn transmit(&mut self, dt: f32) {
        let max_health = self.moves().max_health;
        let Some(net) = &mut self.net else { return };
        if net.state_due(dt) {
            net.send(&net::Msg::State(net::PlayerState {
                pos: self.player.pos,
                yaw: self.camera.yaw,
                grounded: self.player.grounded(),
                flight: self.player.flight_direction(),
                // As a percentage, so bigger health pools still fit.
                health: if self.dead { 0 } else { (self.health / max_health * 100.0).ceil().clamp(1.0, 100.0) as u8 },
            }));
        }
        if net.drones_due(dt) {
            net.send(&net::Msg::Drones(self.drones.states()));
        }
    }

    /// Sends shots you fired so the other player sees them (and, if they
    /// host, applies their damage).
    /// An ability's blast (Whirlwind, grenades, shockwaves) for the other
    /// player's game to replay against them: where, how big, its damage
    /// (in weapon units), and how long it stuns a player.
    fn send_blast(&self, pos: glam::Vec3, radius: f32, damage: f32, stun: f32) {
        if let Some(net) = self.net.as_ref().filter(|n| n.connected()) {
            net.send(&net::Msg::Fire(net::FireMsg {
                weapon: ABILITY_BLAST,
                origin: pos,
                dir: glam::Vec3::new(stun, 0.0, 0.0),
                damage,
                pellets: 0,
                blast_radius: radius,
                mods: 0,
                homing: false,
            }));
        }
    }

    fn send_volley(&self, volley: &weapons::Volley, shooters: &[(glam::Vec3, glam::Vec3)]) {
        let Some(net) = self.net.as_ref().filter(|n| n.connected()) else { return };
        for &(origin, dir) in shooters {
            net.send(&net::Msg::Fire(net::FireMsg {
                weapon: volley.weapon.index(),
                origin,
                dir,
                damage: volley.damage,
                pellets: volley.pellets.min(255) as u8,
                blast_radius: volley.blast_radius,
                mods: volley.mods.0,
                homing: volley.homing,
            }));
        }
    }

    fn quit(&mut self, event_loop: &ActiveEventLoop) {
        self.save_settings_if_changed();
        self.progress.save();
        event_loop.exit();
    }

    fn set_mouse_captured(&mut self, captured: bool) {
        if captured {
            // Locked is ideal but unsupported on Windows; Confined works everywhere.
            let grabbed = self
                .window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| self.window.set_cursor_grab(CursorGrabMode::Confined));
            if let Err(e) = grabbed {
                log::warn!("could not capture the mouse: {e}");
                return;
            }
        } else {
            let _ = self.window.set_cursor_grab(CursorGrabMode::None);
        }
        self.window.set_cursor_visible(!captured);
        self.mouse_captured = captured;
    }

    fn update_and_render(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        // Cap dt so a long frame (e.g. dragging the window) can't tunnel through walls.
        let dt = (now - self.last_frame).as_secs_f32().min(1.0 / 30.0);
        self.last_frame = now;
        // The click that captures the mouse shouldn't also fire.
        let captured_before = self.mouse_captured;
        // Multiplayer: what the other player sent (also while paused, so the
        // connection stays up).
        // In a fight, the other player's shots can hit you.
        self.weapons.victim = (self.arena.fighting() && !self.dead).then_some(self.player.pos);
        // Steam: its events, and "Join Game" clicked in the friends list.
        let steam_join = self.steam.as_ref().and_then(|s| {
            s.run_callbacks();
            s.take_join_request()
        });
        if let Some(host) = steam_join {
            self.start_net(&MenuAction::JoinSteam(host));
        }
        // Friends to invite, while hosting through Steam with the menu open.
        let inviting = self.menu_open && self.net.as_ref().is_some_and(|n| n.over_steam() && n.is_host() && !n.connected());
        self.friends_refresh -= dt;
        if inviting && self.friends_refresh <= 0.0 {
            self.friends_refresh = 3.0;
            self.steam_friends = self.steam.as_ref().map(|s| s.friends()).unwrap_or_default();
        } else if !inviting {
            self.friends_refresh = 0.0;
        }
        self.receive(dt);
        self.run_match(dt);
        // Structures: their boxes join the world's colliders when they
        // change, and they stop shots and drone fire.
        if self.structures.take_dirty() {
            self.colliders = self.scene.colliders.clone();
            self.colliders.extend(self.structures.boxes());
            // Built walls are sheer: no stepping, mantling or wall running on them.
            self.player.sheer = self.structures.boxes();
        }
        let blockers = self.structures.blockers();
        self.drones.blockers = blockers.clone();
        self.weapons.blockers = blockers;
        for c in &mut self.cooldowns {
            *c = (*c - dt).max(0.0);
        }

        self.clock += dt;
        let on_title = self.screen == Screen::Title;
        // Title screen: Enter plays; Esc only closes a menu opened from it.
        if on_title {
            self.title_time += dt;
            if self.input.pressed(KeyCode::Escape) && self.menu_open {
                self.menu_open = false;
                self.save_settings_if_changed();
                self.audio.play(audio::Sfx::Click);
            }
            if !self.menu_open && !self.starting && (self.input.pressed(KeyCode::Enter) || self.input.pressed(KeyCode::NumpadEnter)) {
                self.starting = true;
                self.audio.play(audio::Sfx::Click);
            }
        }
        // The tutorial: Enter skips it; it closes itself once you're done.
        if let Some(t) = &mut self.tutorial {
            let skip = !self.menu_open && self.input.pressed(KeyCode::Enter);
            if t.tick(dt) || skip {
                self.tutorial = None;
                self.settings.seen_intro = true;
                self.save_settings_if_changed();
                if skip {
                    self.audio.play(audio::Sfx::Click);
                }
            }
        }

        // Escape toggles the pause/settings menu.
        if on_title {
            // Handled above.
        } else if self.input.pressed(KeyCode::Escape) {
            if self.menu_open {
                self.close_menu();
            } else {
                self.open_menu();
            }
            self.audio.play(audio::Sfx::Click);
        } else if self.input.pressed(KeyCode::Tab) {
            // Tab: straight to the upgrades (skills), and Tab again to leave.
            if self.menu_open {
                self.close_menu();
            } else {
                self.open_menu();
                self.menu.tab = ui::Tab::Abilities;
            }
            self.audio.play(audio::Sfx::Click);
        } else if !self.menu_open {
            if self.input.pressed(KeyCode::KeyV) {
                self.third_person = !self.third_person;
            }
            if self.input.left_clicked() && !self.mouse_captured {
                self.set_mouse_captured(true);
            }
        }

        // The world keeps going while the menu is open (you just stand
        // still: no input reaches you). Bullet time and time stop slow the
        // world down more than you.
        let mut world_dt = dt;
        let mut player_dt = if on_title { 0.0 } else { world_dt };
        if self.feel.hitstop > 0.0 && world_dt > 0.0 {
            self.feel.hitstop -= dt;
            world_dt *= 0.08;
            player_dt *= 0.25;
        }
        // Hits and kills this frame (for markers and sounds).
        let mut hits = 0;
        let mut kills = std::mem::take(&mut self.extra_kills);
        if !on_title {
            use abilities::Skill;
            let moves = self.moves();
            // While casting, only the spell's own key does anything (to
            // overcharge it, or press again to call it off).
            let cast = self.casting.map(|(s, _, _)| s);
            let clone_spawn = if moves.max_clones > 0
                && cast.is_none()
                && self.cooldowns[Skill::Clone.index()] <= 0.0
                && self.binds()[Skill::Clone.index()].is_some_and(|b| self.input.bind_pressed(b))
            {
                Some(self.clone_circle(&moves))
            } else {
                None
            };
            // Half-placed barrier / rift gate: count down, cancel when
            // time runs out, and preview what the next press makes.
            self.tick_pending(dt, &moves);
            let input = &self.input;
            // Each skill's key or button (yours, or its default).
            let binds = self.binds();
            let free = |s: Skill| cast.is_none_or(|c| c == s);
            let pressed = |s: Skill| free(s) && binds[s.index()].is_some_and(|b| input.bind_pressed(b));
            let held = |s: Skill| free(s) && binds[s.index()].is_some_and(|b| input.bind_held(b));
            let mut message: Option<String> = None;
            // A capstone trial completed this frame.
            let mut trial: Option<abilities::Unlock> = None;
            if self.mouse_captured {
                let mut delta = input.mouse_delta();
                if self.settings.invert_y {
                    delta.y = -delta.y;
                }
                self.camera
                    .rotate(delta, MOUSE_SENSITIVITY * self.settings.mouse_sensitivity);
                if let Some(t) = &mut self.tutorial
                    && t.note(tutorial::Did::Looked(delta.length()))
                {
                    self.audio.play(audio::Sfx::Toast);
                }
            }
            let shift = [KeyCode::ShiftLeft, KeyCode::ShiftRight];
            let controls = Controls {
                forward: input.axis(&[KeyCode::KeyS, KeyCode::ArrowDown], &[KeyCode::KeyW, KeyCode::ArrowUp]),
                right: input.axis(&[KeyCode::KeyA, KeyCode::ArrowLeft], &[KeyCode::KeyD, KeyCode::ArrowRight]),
                sprint: input.any_held(&shift),
                jump: input.pressed(KeyCode::Space),
                pitch: self.camera.pitch,
                flight_toggle: pressed(Skill::Hover),
                slam: input.pressed(KeyCode::ControlLeft) || input.pressed(KeyCode::ControlRight),
                boost: input.pressed(KeyCode::ShiftLeft) || input.pressed(KeyCode::ShiftRight),
                levitate: held(Skill::Hover),
                gravity_toggle: pressed(Skill::LowGravity) && self.cooldowns[Skill::LowGravity.index()] <= 0.0,
            };
            if controls.gravity_toggle && moves.low_gravity {
                self.cooldowns[Skill::LowGravity.index()] = skill_cooldown(Skill::LowGravity);
            }
            // Out of the round: you can look around, but not move.
            let controls = if self.dead { Controls { pitch: self.camera.pitch, ..Default::default() } } else { controls };
            // Block: hold Q to guard; slow, and no attacking meanwhile.
            let was_blocking = self.blocking;
            let guard_ready = was_blocking || self.cooldowns[Skill::Block.index()] <= 0.0;
            self.blocking = moves.block > 0.0 && !self.dead && held(Skill::Block) && guard_ready;
            if was_blocking && !self.blocking {
                self.cooldowns[Skill::Block.index()] = skill_cooldown(Skill::Block);
            }
            self.block_time = if self.blocking { self.block_time + dt } else { 0.0 };
            if self.blocking && !was_blocking {
                self.audio.play(audio::Sfx::ShieldHit);
            }
            // Casting slows you (unless your will is unshakable); so does a
            // blizzard someone else called down on you.
            let tag = (self.me() as u32 + 1) << 24;
            let chilled = self.structures.storms().iter().any(|&(c, r, _, id)| id & 0xff00_0000 != tag && c.distance(self.player.pos) < r);
            if chilled && self.arena.fighting() {
                self.pending_damage += 4.0 * player_dt;
            }
            let controls = if cast.is_some() {
                let crawl = if moves.steady_cast { CAST_WALK_STEADY } else { CAST_WALK };
                Controls {
                    forward: controls.forward * crawl,
                    right: controls.right * crawl,
                    pitch: controls.pitch,
                    ..Default::default()
                }
            } else if self.blocking {
                Controls { forward: controls.forward * 0.45, right: controls.right * 0.45, sprint: false, ..controls }
            } else if chilled {
                Controls { forward: controls.forward * 0.5, right: controls.right * 0.5, sprint: false, ..controls }
            } else {
                controls
            };
            let (colliders, terrain) = (&self.colliders, &self.scene.terrain);
            let frame_start = self.player.pos;
            let actions = [
                (pressed(Skill::Barrier), Skill::Barrier),
                (pressed(Skill::Ceiling), Skill::Ceiling),
                (pressed(Skill::Dome), Skill::Dome),
                (pressed(Skill::GiantBox), Skill::GiantBox),
                (pressed(Skill::HealPad), Skill::HealPad),
                (pressed(Skill::GravityWell), Skill::GravityWell),
                (pressed(Skill::Pull), Skill::Pull),
                (pressed(Skill::Repulse), Skill::Repulse),
                (pressed(Skill::GroundPull), Skill::GroundPull),
                (pressed(Skill::SplitShot), Skill::SplitShot),
                (pressed(Skill::Meteor), Skill::Meteor),
                (pressed(Skill::ChainLightning), Skill::ChainLightning),
                (pressed(Skill::Blizzard), Skill::Blizzard),
                (pressed(Skill::ArcaneWard), Skill::ArcaneWard),
                (pressed(Skill::RecallHome), Skill::RecallHome),
                (pressed(Skill::Dispel), Skill::Dispel),
                (pressed(Skill::ChronoEcho), Skill::ChronoEcho),
                (pressed(Skill::RiftGate), Skill::RiftGate),
                (pressed(Skill::Fireball), Skill::Fireball),
                (pressed(Skill::ArcaneMissiles), Skill::ArcaneMissiles),
                (pressed(Skill::FrostNova), Skill::FrostNova),
                (pressed(Skill::ArcaneLance), Skill::ArcaneLance),
                (pressed(Skill::Rejuvenate), Skill::Rejuvenate),
                (pressed(Skill::Haste), Skill::Haste),
                (pressed(Skill::ShadowStep), Skill::ShadowStep),
                (pressed(Skill::ArcaneClone), Skill::ArcaneClone),
                (pressed(Skill::RapidFire), Skill::RapidFire),
                (pressed(Skill::StunGrenade), Skill::StunGrenade),
                (pressed(Skill::Whirlwind), Skill::Whirlwind),
                (pressed(Skill::IronSkin), Skill::IronSkin),
                (pressed(Skill::Mend), Skill::Mend),
                (pressed(Skill::VampiricAura), Skill::VampiricAura),
                (pressed(Skill::Stasis), Skill::Stasis),
                (pressed(Skill::Leap), Skill::Leap),
                (pressed(Skill::TimeWarp), Skill::TimeWarp),
            ];
            // A left click finishes a half-placed barrier, ceiling or rift gate.
            let click_confirm = self.mouse_captured && captured_before && input.left_clicked() && !self.dead && cast.is_none();
            let click_confirm = if click_confirm { self.pending_skill() } else { None };

            // Time: T starts bullet time (or, at the top level, time stop).
            if let Some((_, left)) = &mut self.time_effect {
                *left -= dt;
                if *left <= 0.0 {
                    self.time_effect = None;
                    self.time_cooldown = moves.time_cooldown;
                }
            } else {
                self.time_cooldown = (self.time_cooldown - dt).max(0.0);
            }
            if self.time_effect.is_none() && self.time_cooldown == moves.time_cooldown && moves.time_cooldown > 0.0 {
                // It just ran out this frame.
                self.audio.play(audio::Sfx::TimeResume);
            }
            match self.time_effect.map(|(e, _)| e) {
                Some(TimeEffect::Bullet) => (world_dt, player_dt) = (dt * 0.3, dt * 0.7),
                Some(TimeEffect::Stop) => world_dt = 0.0,
                None => {}
            }

            // Teleport dash: charges recharge one at a time.
            self.dash_ready = self.dash_ready.min(moves.dash_charges);
            if self.dash_ready < moves.dash_charges {
                self.dash_timer += dt;
                if self.dash_timer >= moves.dash_recharge {
                    self.dash_ready += 1;
                    self.dash_timer = 0.0;
                }
            }
            let dash_forward = glam::Vec3::new(self.camera.yaw.cos(), 0.0, self.camera.yaw.sin());
            let dash_right = glam::Vec3::new(-self.camera.yaw.sin(), 0.0, self.camera.yaw.cos());
            let move_direction = dash_forward * controls.forward + dash_right * controls.right;
            let dash_direction = if move_direction.length_squared() > 0.0 {
                move_direction.normalize()
            } else {
                self.camera.forward()
            };
            if pressed(Skill::Dash)
                && self.dash_ready > 0
                && !self.dead
                && if moves.dash_any_direction {
                    self.player.blink_free(dash_direction, moves.dash_distance, colliders, terrain)
                } else {
                    self.player.blink(dash_direction, moves.dash_distance, colliders, terrain)
                }
            {
                self.dash_ready -= 1;
                arm_echo(&mut self.echo_charges, &mut self.echo_repeat, Skill::Dash, dash_direction);
                trial = trial.or(self.progress.record(abilities::Stat::Dashes, 1.0));
                self.body.reset_motion();
                self.audio.play(audio::Sfx::Dash);
                self.feel.fov_kick += 8.0;
            } else if pressed(Skill::Dash) && moves.dash_charges > 0 {
                self.audio.play(audio::Sfx::Denied);
            }

            // Recall marker and rewind.
            self.play_time += player_dt;
            self.trail.push_back((self.play_time, self.player.pos));
            while self.trail.front().is_some_and(|(t, _)| self.play_time - t > 6.5) {
                self.trail.pop_front();
            }
            self.rewind_cooldown = (self.rewind_cooldown - dt).max(0.0);
            let mut jumped_back = None;
            if pressed(Skill::Recall) && moves.recall && self.marker.is_some() && self.cooldowns[Skill::Recall.index()] > 0.0 {
                self.audio.play(audio::Sfx::Denied);
            } else if pressed(Skill::Recall) && moves.recall {
                match self.marker.take() {
                    None => {
                        self.marker = Some(self.player.pos);
                        message = Some("Marker dropped. B to return to it.".into());
                        self.audio.play(audio::Sfx::Marker);
                    }
                    Some(p) => {
                        jumped_back = Some(p);
                        self.cooldowns[Skill::Recall.index()] = skill_cooldown(Skill::Recall);
                    }
                }
            }
            if pressed(Skill::Rewind) && moves.rewind > 0.0 && self.rewind_cooldown <= 0.0 {
                jumped_back = self
                    .trail
                    .iter()
                    .find(|(t, _)| self.play_time - t <= moves.rewind)
                    .map(|&(_, p)| p);
                self.rewind_cooldown = skill_cooldown(Skill::Rewind);
                if jumped_back.is_some() {
                    trial = trial.or(self.progress.record(abilities::Stat::Rewinds, 1.0));
                }
            }
            if let Some(p) = jumped_back {
                self.audio.play(audio::Sfx::Rewind);
                self.player.teleport(p);
                if moves.time_refill {
                    self.player.refill(&moves);
                }
                self.trail.clear();
                self.body.reset_motion();
            }

            // Clones.
            let (switch, dismiss) = (
                cast.is_none() && input.pressed(KeyCode::KeyX),
                cast.is_none() && input.pressed(KeyCode::KeyR),
            );
            if let Some(spots) = clone_spawn
                && Self::spawn_clone(&mut self.clones, &mut self.renderer, &spots, self.camera.yaw, moves.max_clones)
            {
                arm_echo(&mut self.echo_charges, &mut self.echo_repeat, Skill::Clone, glam::Vec3::ZERO);
                if self.clones.count_of(clones::Kind::Mirror) >= moves.max_clones {
                    self.cooldowns[Skill::Clone.index()] = skill_cooldown(Skill::Clone);
                }
                self.audio.play(audio::Sfx::Clone);
            }
            if dismiss && self.clones.count() > 0 {
                self.clones.dismiss_all(&mut self.renderer);
                self.audio.play(audio::Sfx::Click);
            }
            if switch && moves.switch_places && self.clones.switch_with_newest(&mut self.player) {
                self.body.reset_motion();
                trial = trial.or(self.progress.record(abilities::Stat::Swaps, 1.0));
                self.audio.play(audio::Sfx::Swap);
            }

            // Gadgets: pads (P) and the grapple (hold E).
            if pressed(Skill::Pad) && moves.pads > 0 && self.cooldowns[Skill::Pad.index()] <= 0.0 {
                self.gadgets.place_pad(self.player.pos, moves.pads);
                if self.gadgets.pad_count() >= moves.pads {
                    self.cooldowns[Skill::Pad.index()] = skill_cooldown(Skill::Pad);
                }
                arm_echo(&mut self.echo_charges, &mut self.echo_repeat, Skill::Pad, glam::Vec3::ZERO);
                self.audio.play(audio::Sfx::PadPlace);
            }
            if moves.pads == 0 {
                self.gadgets.clear_pads();
            }
            let e_held = held(Skill::Grapple);
            if pressed(Skill::Grapple) && moves.grapple_range > 0.0 && self.cooldowns[Skill::Grapple.index()] <= 0.0 {
                let aim = gadgets::raycast(self.camera.position, self.camera.forward(), moves.grapple_range, colliders, terrain);
                match aim {
                    Some(point) => {
                        self.player.set_hook(point);
                        self.hook = Some(point);
                        self.cooldowns[Skill::Grapple.index()] = skill_cooldown(Skill::Grapple);
                        self.audio.play(audio::Sfx::Grapple);
                        self.audio.play_at(audio::Sfx::Hooked, point);
                    }
                    None => {
                        message = Some("Nothing in reach to hook.".into());
                        self.audio.play(audio::Sfx::Denied);
                    }
                }
            } else if self.player.hooked() && (!e_held || moves.grapple_range <= 0.0) {
                self.player.release_hook(moves.slingshot, &moves);
            }
            if !self.player.hooked() {
                self.hook = None;
            }
            // Weapons: 1-7 choose, left mouse fires at the crosshair (or swings).
            let mut remote_kills = 0;
            // Damage dealt (melee and all) and status effects, for lifesteal and trials.
            let (mut dealt, mut melee_dealt, mut melee_hits, mut effects) = (0.0, 0.0, 0u32, 0u32);
            self.weapons.ensure_owned(&moves);
            let mut fired = None;
            let authority = self.net.as_ref().is_none_or(|n| n.is_host() || !n.connected());
            let picks = [
                KeyCode::Digit1,
                KeyCode::Digit2,
                KeyCode::Digit3,
                KeyCode::Digit4,
                KeyCode::Digit5,
                KeyCode::Digit6,
                KeyCode::Digit7,
            ];
            for (key, weapon) in picks.into_iter().zip(weapons::Weapon::ALL) {
                if cast.is_none() && input.pressed(key) && self.weapons.select(weapon, &moves) {
                    message = Some(weapon.name().into());
                    self.audio.play(audio::Sfx::WeaponSwitch);
                }
            }
            self.weapons.reload(dt);
            let charging_blade = self.weapons.current == weapons::Weapon::Blade && moves.blade_cone;
            if charging_blade && input.left_held() && captured_before && cast.is_none() {
                self.blade_charge = (self.blade_charge + dt).min(1.5);
            } else if !charging_blade {
                self.blade_charge = 0.0;
            }
            let blade_charged = charging_blade && input.left_released() && self.blade_charge >= 0.08;
            let weapon_input = if charging_blade { input.left_released() } else { input.left_held() };
            let charge_ratio = (self.blade_charge / 1.5).clamp(0.0, 1.0);
            // While placing (or until the click that placed is let go), the
            // left button builds instead of attacking.
            if !input.left_held() {
                self.click_used = false;
            }
            let placing = self.click_used || self.pending_skill().is_some();
            if captured_before && weapon_input && !placing && moves.armed() && !self.dead && !self.blocking && self.casting.is_none() {
                let (eye, aim) = (self.camera.position, self.camera.forward());
                let world = gadgets::raycast(eye, aim, 250.0, colliders, terrain).map_or(250.0, |p| p.distance(eye));
                let reach = self.drones.ray_hit(eye, aim, world).map_or(world, |(_, t)| t);
                let target = eye + aim * reach;
                let side = glam::Vec3::new(-self.camera.yaw.sin(), 0.0, self.camera.yaw.cos());
                let hand = self.player.pos + glam::Vec3::Y * 1.35 + side * 0.3;
                // Melee swings from the chest along your view; guns from the
                // hand towards what's under the crosshair.
                let mut shooters = if self.weapons.current.is_melee() {
                    vec![(self.player.pos + glam::Vec3::Y * 1.3, aim)]
                } else {
                    vec![(hand, target - hand)]
                };
                if moves.clones_fire {
                    let pitch = self.camera.pitch;
                    for (pos, yaw) in self.clones.shooters() {
                        let dir = glam::Vec3::new(yaw.cos() * pitch.cos(), pitch.sin(), yaw.sin() * pitch.cos());
                        shooters.push((pos + glam::Vec3::Y * 1.35, dir));
                    }
                }
                // Arcane clones copy the attack from where they stand, at
                // what you're aiming at.
                for pos in self.clones.arcane() {
                    if self.weapons.current.is_melee() {
                        shooters.push((pos + glam::Vec3::Y * 1.3, aim));
                    } else {
                        let from = pos + glam::Vec3::Y * 1.35;
                        shooters.push((from, target - from));
                    }
                }
                let authority = self.net.as_ref().is_none_or(|n| n.is_host() || !n.connected());
                // Split Shot: every gun shot fans out into several, each weaker.
                let mut fire_moves = moves;
                // A short tap remains a close blade swing. Releasing a charge
                // sends one carried slash whose range follows charge time.
                fire_moves.blade_cone = blade_charged;
                if blade_charged {
                    fire_moves.blast_radius = 4.0 + charge_ratio * 36.0;
                }
                if self.weapons.current == weapons::Weapon::Hammer {
                    let velocity = self.player.velocity();
                    let forward_speed = velocity.dot(aim).max(0.0);
                    let downward_speed = (-velocity.y).max(0.0);
                    fire_moves.weapon_damage *= 1.0 + (forward_speed + downward_speed) / 45.0;
                }
                if self.split_left > 0.0 && !self.weapons.current.is_melee() && moves.split > 1 {
                    fire_moves.weapon_damage *= moves.split_damage;
                    let base = shooters.clone();
                    shooters.clear();
                    for (o, d) in base {
                        let half = (moves.split as i32 - 1) / 2;
                        for k in -half..=half {
                            let turn = glam::Quat::from_rotation_y(k as f32 * 0.09);
                            shooters.push((o, turn * d));
                        }
                    }
                }
                let (events, volley) = self.weapons.fire(&shooters, &fire_moves, authority, &mut self.drones, colliders, terrain);
                if volley.is_some_and(|v| v.weapon == weapons::Weapon::Blade && moves.blade_cone) {
                    self.blade_charge = 0.0;
                }
                if volley.is_some_and(|v| v.weapon == weapons::Weapon::Scythe) {
                    self.blocking = true;
                    self.block_time = 0.0;
                }
                kills += events.kills;
                hits += events.hits;
                remote_kills += events.remote_kills;
                (dealt, melee_dealt) = (dealt + events.dealt, melee_dealt + events.melee_dealt);
                (melee_hits, effects) = (melee_hits + events.melee_hits, effects + events.effects);
                if let Some(v) = &volley {
                    if let Some(t) = &mut self.tutorial
                        && t.note(tutorial::Did::Swung)
                    {
                        self.audio.play(audio::Sfx::Toast);
                    }
                    // One action per pull of the trigger (or charged release),
                    // however many shots a held gun fires.
                    if (input.left_clicked() || input.left_released()) && self.clones.spend_action(&mut self.renderer) > 0 {
                        message = Some("Your clone faded after its five actions.".into());
                    }
                    self.audio.play(weapon_sound(v.weapon));
                    let kick = match v.weapon {
                        weapons::Weapon::Blade => 0.04,
                        weapons::Weapon::Scythe => 0.07,
                        weapons::Weapon::Hammer => 0.12,
                        weapons::Weapon::Blaster => 0.03,
                        weapons::Weapon::Scatter => 0.1,
                        weapons::Weapon::Rockets => 0.08,
                        weapons::Weapon::Railgun => 0.14,
                    };
                    self.shake = self.shake.max(kick * self.settings.screen_shake);
                }
                // Your swing also batters structures in reach.
                if let Some(v) = volley.as_ref().filter(|v| v.weapon.is_melee()) {
                    for &(origin, dir) in &shooters {
                        let reach = if v.weapon == weapons::Weapon::Blade && v.mods.has(weapons::Mods::BLADE_CONE) {
                            v.blast_radius
                        } else {
                            v.weapon.reach()
                        };
                        for id in self.structures.in_reach(origin, dir.normalize_or(glam::Vec3::X), reach) {
                            self.struct_damage.push((id, v.damage * v.weapon.swing_damage() * structures::SHOT_SCALE));
                        }
                    }
                }
                fired = volley.map(|v| (v, shooters));
            }

            if self.gadgets.bounce(&mut self.player, moves.pad_launch) {
                self.audio.play(audio::Sfx::Bounce);
            }
            for clone in self.clones.players_mut() {
                self.gadgets.bounce(clone, moves.pad_launch);
            }

            let slammed = self
                .player
                .update(player_dt, &controls, self.camera.yaw, colliders, terrain, &moves);
            Self::try_rift_player(&mut self.player, &mut self.camera, self.rift_gates, frame_start, &mut self.weapons);
            // Domes: nobody gets out (or in) until they break, however they move.
            self.structures.constrain(&mut self.player, frame_start);
            // The tutorial: walking, sprinting and jumping.
            if let Some(t) = &mut self.tutorial {
                let step = self.player.pos - frame_start;
                let walked = glam::Vec2::new(step.x, step.z).length();
                let mut stepped = false;
                if self.player.grounded() && walked < 2.0 {
                    stepped |= t.note(tutorial::Did::Walked(walked));
                    if controls.sprint && walked > 0.0 {
                        stepped |= t.note(tutorial::Did::Sprinted(player_dt));
                    }
                }
                if self.player.events.contains(&player::Event::Jump) {
                    stepped |= t.note(tutorial::Did::Jumped);
                }
                if stepped {
                    self.audio.play(audio::Sfx::Toast);
                }
            }
            // Movement sounds and the landing dip.
            for event in self.player.events.clone() {
                use player::Event;
                let sound = match event {
                    Event::Jump => Some(audio::Sfx::Jump),
                    Event::AirJump | Event::WallJump => Some(audio::Sfx::AirJump),
                    Event::WingsOpen | Event::WingsFold | Event::Slide => Some(audio::Sfx::Wings),
                    Event::Boost => {
                        self.feel.fov_kick += 6.0;
                        Some(audio::Sfx::Boost)
                    }
                    Event::SlamStart => Some(audio::Sfx::Dash),
                    Event::Land(speed) => {
                        if self.settings.head_bob {
                            self.feel.dip_velocity -= (speed * 0.12).min(2.6);
                        }
                        (speed > 3.0).then(|| audio::Sfx::Land(((speed - 3.0) / 22.0).min(1.0)))
                    }
                };
                if let Some(s) = sound {
                    self.audio.play(s);
                }
            }
            if slammed {
                self.shake = self.shake.max(0.35 * self.settings.screen_shake);
                self.audio.play(audio::Sfx::Slam);
                let wave = self.weapons.shockwave(self.player.pos, moves.weapon_damage * self.player.slam_power(), false, authority, &mut self.drones);
                if self.player.slam_power() >= 1.5 {
                    self.feel.hitstop = self.feel.hitstop.max(HITSTOP_SLAM * self.settings.screen_shake.min(1.0));
                }
                // Slamming down hard from high up leaves a crater.
                if self.player.slam_power() >= 1.5 {
                    self.weapons.craters.push((self.player.pos, 6.0 + 4.0 * (self.player.slam_power() - 1.5)));
                }
                kills += wave.kills;
                hits += wave.hits;
                dealt += wave.dealt;
                effects += wave.effects;
                if let Some(net) = self.net.as_ref().filter(|n| n.connected()) {
                    // Weapon number 255: a slam's shockwave.
                    net.send(&net::Msg::Fire(net::FireMsg {
                        weapon: 255,
                        origin: self.player.pos,
                        dir: glam::Vec3::Y,
                        damage: moves.weapon_damage,
                        pellets: 0,
                        blast_radius: 6.0,
                        mods: 0,
                        homing: false,
                    }));
                }
            }
            let shots = self.weapons.update(
                world_dt,
                authority,
                &mut self.drones,
                colliders,
                terrain,
                self.player.pos + glam::Vec3::Y * 0.9,
            );
            self.player.push(shots.push);
            self.pending_damage += shots.player_damage;
            if shots.push.length() > 6.0 {
                self.shake = self.shake.max(0.2 * self.settings.screen_shake);
            }
            kills += shots.kills;
            hits += shots.hits;
            remote_kills += shots.remote_kills;
            dealt += shots.dealt;
            effects += shots.effects;
            // Drones: they bleed and burn, and shoot back at you.
            self.threat_time += world_dt;
            let level = self.threat_level();
            if level > self.threat {
                self.threat = level;
                let (title, line) = threat_news(level);
                self.banner = Some(ui::Banner::new(&title, line, ui::BANNER_GOLD));
                self.audio.play(audio::Sfx::Fight);
            }
            self.drones.threat = self.threat;
            self.drones.aggression = [0.0, 1.0, 1.7][(self.settings.difficulty as usize).min(2)]
                * (1.0 + moves.ascension as f32 * 0.04)
                * threat_strength(self.threat);
            let target = (!self.dead).then(|| self.player.pos + glam::Vec3::Y * 0.9);
            self.drones.decoys = self.clones.middles();
            let tick = self.drones.update(&mut self.renderer, world_dt, self.player.pos, authority, target, terrain);
            if self.clones.damage(&tick.decoy_hits, &mut self.renderer) > 0 {
                message = Some("A clone was destroyed.".into());
                self.audio.play(audio::Sfx::DroneDown);
            }
            self.pve_damage += tick.player_damage + tick.bird_damage;
            dealt += tick.dealt;
            for &(at, by_other) in &tick.kills {
                self.weapons.add_blast(at, 1.6);
                if by_other {
                    remote_kills += 1;
                } else {
                    kills += 1;
                    self.weapons.kills += 1;
                }
            }
            for &from in &tick.fired {
                self.audio.play_at(audio::Sfx::DroneShot, from);
            }
            if let Some(at) = tick.flock {
                self.audio.play_at(audio::Sfx::Wings, at);
                message = Some("A great flock is coming for you!".into());
            }
            // A big attack winding up, and mortar shells bursting.
            for &from in &tick.warnings {
                self.audio.play_at(audio::Sfx::Beep, from);
            }
            for &(at, radius) in &tick.impacts {
                self.weapons.add_blast(at, radius);
                self.audio.play_at(audio::Sfx::Explosion(radius), at);
            }
            // Lifesteal, and Berserker kills.
            if kills > 0 {
                self.threat_kills += kills;
                self.feel.hitstop = self.feel.hitstop.max(HITSTOP_KILL * self.settings.screen_shake.min(1.0));
                if let Some(t) = &mut self.tutorial
                    && t.note(tutorial::Did::Destroyed)
                {
                    self.audio.play(audio::Sfx::Toast);
                }
            }
            let berserk = if moves.berserker { kills as f32 * 20.0 } else { 0.0 };
            let healing = melee_dealt * moves.melee_lifesteal + dealt * moves.lifesteal + berserk;
            if healing > 0.0 {
                self.heal_queue.push((healing, true));
            }
            if let Some((volley, shooters)) = &fired {
                self.send_volley(volley, shooters);
            }
            if remote_kills > 0
                && let Some(net) = &self.net
            {
                net.send(&net::Msg::Kills(remote_kills));
            }
            self.clones.update(
                &mut self.renderer,
                world_dt,
                &controls,
                self.camera.yaw,
                &self.colliders,
                &self.scene.terrain,
                &moves,
                Some((self.player.pos, self.player.grounded())),
            );

            // Experience: distance travelled and time in the air.
            let moved = self.player.pos.distance(self.last_pos);
            let mut xp = if moved < 3.0 { moved * 0.1 } else { 0.0 }; // not teleports
            if !self.player.grounded() {
                xp += player_dt * 0.5;
            }
            self.last_pos = self.player.pos;

            // Capstone trials.
            let travelled = if moved < 3.0 { moved } else { 0.0 };
            let p = &self.player;
            let mut record = |stat, amount: f32| {
                if amount > 0.0 {
                    trial = trial.or(self.progress.record(stat, amount));
                }
            };
            record(abilities::Stat::GlideDistance, if p.gliding() { travelled } else { 0.0 });
            record(abilities::Stat::WallRunDistance, if p.wall_running() { travelled } else { 0.0 });
            record(abilities::Stat::GrappleDistance, if p.hooked() { travelled } else { 0.0 });
            record(abilities::Stat::HoverTime, if p.hovering() { player_dt } else { 0.0 });
            record(abilities::Stat::Kills, kills as f32);
            record(abilities::Stat::MeleeHits, melee_hits as f32);
            record(abilities::Stat::Effects, effects as f32);
            if p.grounded() {
                let v = p.velocity();
                let foot = glam::Vec2::new(v.x, v.z).length();
                trial = trial.or(self.progress.record_best(abilities::Stat::FootSpeed, foot));
            }
            if let Some(unlock) = trial {
                self.unlocked.push(unlock);
            }
            if kills > 0 {
                xp += kills as f32 * 6.0;
                let scrap = kills * items::DRONE_SCRAP * moves.scrap_multiplier;
                self.progress.scrap += scrap;
                message = Some(format!("Drone destroyed! +{} XP, +{scrap} scrap", kills * 6));
            }
            if self.progress.add_xp(xp) {
                self.progress.scrap += items::XP_SCRAP;
                self.progress.save();
                self.announce("SKILL POINT", "Earned from experience. Press Esc to spend it", ui::BANNER_GOLD, audio::Sfx::SkillPoint);
            }

            // Skill crystals.
            let chest = self.player.pos + glam::Vec3::Y * 1.0;
            if let Some(crystal) = self.orbs.update(&mut self.renderer, world_dt, chest)
                && self.progress.collect(crystal)
            {
                self.progress.save();
                let (found, total) = (self.progress.found.len(), self.orbs.total());
                self.progress.scrap += items::CRYSTAL_SCRAP;
                let title = if found == total { "ALL CRYSTALS FOUND" } else { "SKILL CRYSTAL" };
                self.announce(
                    title,
                    format!("{found} of {total} found  ·  +1 skill point  ·  +{} scrap", items::CRYSTAL_SCRAP),
                    ui::BANNER_BLUE,
                    audio::Sfx::Crystal,
                );
            }

            // Footsteps and head bob follow the distance walked.
            let v = self.player.velocity();
            let ground_speed = glam::Vec2::new(v.x, v.z).length();
            let walking = self.player.grounded() && ground_speed > 0.8 && !self.dead;
            if walking {
                let stride = (1.25 + ground_speed * 0.06).min(2.4);
                self.feel.stride += ground_speed * player_dt;
                if self.feel.stride > stride {
                    self.feel.stride -= stride;
                    self.audio.play(audio::Sfx::Footstep);
                }
                self.feel.bob_phase += ground_speed * player_dt / stride * std::f32::consts::PI;
            } else {
                self.feel.stride = 0.9; // the first step comes quickly
            }
            let bob_target = if walking && self.settings.head_bob { (ground_speed / 7.0).min(1.3) } else { 0.0 };
            self.feel.bob_amount += (bob_target - self.feel.bob_amount) * (dt * 8.0).min(1.0);

            self.camera.position = self.player.eye();
            if self.shake > 0.0 {
                let t = self.clock;
                let wobble = glam::Vec3::new((t * 47.0).sin(), (t * 39.0).cos(), (t * 43.0).sin());
                self.camera.position += wobble * self.shake * 0.10;
                self.shake = (self.shake - dt * 2.2).max(0.0);
            }
            if let Some(m) = message {
                self.notify(m);
            }
            // Building: H barrier, J ceiling, K dome.
            // Gravity wells drag you in (unless they're yours), and drones.
            let (colliders, terrain) = (&self.colliders, &self.scene.terrain);
            let tag = (self.me() as u32 + 1) << 24;
            for (c, r, power, id) in self.structures.wells() {
                let to = c - self.player.pos;
                let flat = glam::Vec3::new(to.x, 0.0, to.z);
                let d = flat.length();
                if id & 0xff00_0000 != tag && d < r && d > 0.6 && !self.dead {
                    let speed = power * 7.0 * (0.3 + 0.7 * (1.0 - d / r));
                    self.player.drag(flat / d * speed * player_dt, colliders, terrain);
                }
                for (i, _) in self.drones.within(c, r) {
                    if let Some(p) = self.drones.position(i) {
                        let stun = if power >= 2.0 { 0.3 } else { 0.0 };
                        self.drones.push(i, (c + glam::Vec3::Y * 2.0 - p).normalize_or_zero() * power * 24.0 * world_dt, stun);
                    }
                }
            }
            // Blizzards keep drones frozen; yours batter them twice a second.
            self.storm_tick -= world_dt;
            let batter = self.storm_tick <= 0.0;
            if batter {
                self.storm_tick = 0.5;
            }
            let authority = self.authority();
            for (c, r, power, id) in self.structures.storms() {
                // Someone else's storm batters you while you stand in it.
                if batter && id & 0xff00_0000 != tag && c.distance(self.player.pos) < r {
                    self.pending_damage += 2.0 * power;
                }
                for (i, _) in self.drones.within(c, r) {
                    self.drones.push(i, glam::Vec3::ZERO, 0.3);
                    if batter && authority && id & 0xff00_0000 == tag {
                        let chill = drones::Effect { stun: 0.6, ..Default::default() };
                        let r = self.drones.hit(i, 1.5 * moves.spell_power * moves.weapon_damage, &chill, false);
                        if r.killed.is_some() {
                            self.weapons.add_blast(c, 1.6);
                            self.weapons.kills += 1;
                            self.extra_kills += 1;
                        }
                    }
                }
            }
            // Healing pads (anyone's) heal whoever stands on them.
            let (rate, recharge) = self.structures.heal_at(self.player.pos);
            if rate > 0.0 && !self.dead {
                self.heal_queue.push((rate * player_dt, false));
                if recharge {
                    self.vitals.shield = (self.vitals.shield + 20.0 * player_dt).min(moves.shield);
                }
            }
            self.split_left = (self.split_left - dt).max(0.0);
            // Casting: the spell goes off when the bar fills.
            let mut cast_done = None;
            let holding = self.casting.is_some_and(|(s, _, _)| self.binds()[s.index()].is_some_and(|b| self.input.bind_held(b)));
            if let Some((spell, t, total)) = &mut self.casting {
                *t += player_dt;
                // Still holding the key once the bar is full: it overcharges,
                // until you let go (or it has been full a while).
                let charging = holding && *t < *total + OVERCHARGE_TIME + OVERCHARGE_HOLD;
                if *t >= *total && !charging {
                    cast_done = Some((*spell, 1.0 + ((*t - *total) / OVERCHARGE_TIME).clamp(0.0, 1.0)));
                }
            }
            if self.dead {
                self.casting = None;
            }
            // Circles: each planted spell's (and a small one on what it's
            // aimed at), and while casting, the circle being drawn and
            // where the spell is aimed right now.
            let mut marks: Vec<(glam::Vec3, glam::Vec4, f32, f32)> = Vec::new();
            for r in &self.runes {
                let (color, left) = (element(r.spell).color(), (r.fuse / 8.0).min(1.0));
                marks.push((r.at.origin, color, left, 2.4));
                if !centred(r.spell) {
                    marks.push((r.at.target, color * 0.6, left, 1.2));
                }
            }
            if let Some((spell, t, total)) = self.casting.filter(|(s, _, _)| rune_able(*s)) {
                let color = element(spell).color() * (0.4 + 0.6 * (t / total).min(1.0));
                marks.push((self.cast_origin, color, 1.0, 2.4 * (0.4 + 0.6 * (t / total).min(1.0))));
                if !centred(spell) {
                    let aim = self.spell_aim(spell, self.cast_origin);
                    marks.push((aim.target, color * 0.6, 1.0, 1.2));
                }
            }
            self.rune_marks.update(&mut self.renderer, dt, &marks);
            for (at, blast) in std::mem::take(&mut self.weapons.craters) {
                self.blast_crater(at, blast);
            }
            for (pos, radius, depth) in std::mem::take(&mut self.their_craters) {
                self.apply_crater(pos, radius, depth);
            }
            // Chrono Echo: the repeat goes off a moment after the original;
            // an unused echo fades.
            if let Some((left, _, _)) = &mut self.echo_repeat {
                *left -= dt;
            }
            if let Some((_, skill, dir)) = self.echo_repeat.take_if(|(left, _, _)| *left <= 0.0) {
                self.run_echo(skill, dir);
            }
            self.haste_left = (self.haste_left - dt).max(0.0);
            for b in &mut self.buff_left {
                *b = (*b - dt).max(0.0);
            }
            if self.echo_charges > 0 {
                self.echo_left -= dt;
                if self.echo_left <= 0.0 {
                    self.echo_charges = 0;
                    self.notify("The echo faded.");
                }
            }
            // Skills on their own keys.
            for (key, skill) in actions {
                if key {
                    self.activate(skill);
                }
            }
            if let Some(skill) = click_confirm {
                self.click_used = true;
                self.activate(skill);
            }
            // Runes: each spell's key sets off its own; tap Alt (not
            // mid-cast) to set them all off at once. Left unused, a rune
            // fizzles out after a while.
            for r in &mut self.runes {
                r.fuse -= world_dt;
                r.boost = (r.boost - OVERCHARGE_FADE * world_dt).max(1.0);
            }
            for r in &mut self.recent_runes {
                r.2 += world_dt;
            }
            self.recent_runes.retain(|r| r.2 < COMBO_WINDOW);
            let faded = self.runes.len();
            self.runes.retain(|r| r.fuse > 0.0);
            if self.runes.len() < faded {
                self.notify("A rune fizzled out unused.");
            }
            let tap = self.input.pressed(KeyCode::AltLeft) && self.casting.is_none() && cast_done.is_none() && !self.menu_open;
            if tap && !self.runes.is_empty() {
                let all = std::mem::take(&mut self.runes);
                self.detonate(all);
            }
            // A finished elemental spell lies planted on its circle as a
            // rune, aimed where you were aiming, until you press its key
            // again; its overcharge starts fading.
            if let Some((spell, boost)) = cast_done.filter(|(s, _)| rune_able(*s)) {
                self.casting = None;
                cast_done = None;
                let origin = self.cast_origin;
                self.set_rune(spell, boost, origin);
            }
            if let Some((spell, boost)) = cast_done {
                self.casting = None;
                let before = self.cooldowns[spell.index()];
                self.spell_boost = boost;
                // Spells that act on you (Recall Home, Shadow Step, Rift
                // Gate, Arcane Clone...) aren't copied by arcane clones:
                // repeating them would teleport you again, unlink a fresh
                // rift gate or spawn clones from clones. Clones copy the
                // planted spells when you set them off (see release_rune).
                self.cast_spell(spell);
                self.spell_boost = 1.0;
                if boost > 1.05 {
                    self.notify(format!("Overcharged {} ({:.0}%)", spell.name(), boost * 100.0));
                }
                // It worked if it went on its full cooldown.
                let after = self.cooldowns[spell.index()];
                if after > before && after >= skill_cooldown(spell) - 1e-3 {
                    arm_echo(&mut self.echo_charges, &mut self.echo_repeat, spell, glam::Vec3::ZERO);
                    // Summoning arcane clones isn't one of their five actions.
                    if spell != Skill::ArcaneClone && self.clones.spend_action(&mut self.renderer) > 0 {
                        self.notify("Your clone faded after its five actions.");
                    }
                }
            }
        }

        for (amount, from_attacks) in std::mem::take(&mut self.heal_queue) {
            self.heal(amount, from_attacks);
        }
        for unlock in std::mem::take(&mut self.unlocked) {
            self.trial_done(unlock);
        }

        // Camera feel (in play): bob, landing dip and a wider view at speed.
        if self.screen == Screen::Playing {
            let f = &mut self.feel;
            // Damped spring for the landing dip.
            let accel = -f.dip * 140.0 - f.dip_velocity * 16.0;
            f.dip_velocity += accel * dt;
            f.dip += f.dip_velocity * dt;
            f.dip = f.dip.clamp(-0.6, 0.2);
            if !self.menu_open && !self.third_person {
                let right = glam::Vec3::new(-self.camera.yaw.sin(), 0.0, self.camera.yaw.cos());
                let bob = glam::Vec3::Y * (f.bob_phase * 2.0).sin().abs() * -0.025 * f.bob_amount + right * f.bob_phase.sin() * 0.012 * f.bob_amount;
                self.camera.position += bob + glam::Vec3::Y * f.dip;
            }
            let speed = self.player.velocity().length();
            let target = if self.settings.speed_fov { ((speed - 9.0) / 30.0).clamp(0.0, 1.0) * 6.0 } else { 0.0 };
            f.fov_kick += (target - f.fov_kick) * (dt * 7.0).min(1.0);
            f.fov_kick = f.fov_kick.min(10.0);
            self.camera.fov_y = (self.settings.fov_degrees + f.fov_kick).min(125.0).to_radians();
            f.hit = (f.hit - dt * 5.0).max(0.0);
            f.kill = (f.kill - dt * 2.5).max(0.0);
            f.damage = (f.damage - dt * 2.0).max(0.0);
            if hits > 0 {
                f.hit = 1.0;
                self.audio.play(audio::Sfx::Hit);
            }
            if kills > 0 {
                f.kill = 1.0;
                self.audio.play(audio::Sfx::Kill);
            }
        } else {
            self.title_camera();
        }

        // Structures: shots, drone fire, blasts and swings wear them down;
        // they fade when their time is up.
        let mut struct_hits = std::mem::take(&mut self.struct_damage);
        struct_hits.extend(self.weapons.struct_hits.drain(..).map(|(id, d, _)| (id, d)));
        struct_hits.extend(self.drones.struct_hits.drain(..).map(|(id, d, _)| (id, d)));
        for &(pos, radius) in &self.weapons.booms {
            if radius >= 2.0 {
                for id in self.structures.near(pos, radius) {
                    struct_hits.push((id, 6.0 * structures::SHOT_SCALE));
                }
            }
        }
        if !struct_hits.is_empty() {
            self.damage_structures(struct_hits);
        }
        self.structures.update(world_dt);
        self.structures.draw(&mut self.renderer);

        // Explosions: a boom where each one happened, and a jolt if close.
        for (pos, radius) in std::mem::take(&mut self.weapons.booms) {
            let sound = if radius < 2.0 { audio::Sfx::DroneDown } else { audio::Sfx::Explosion(radius) };
            self.audio.play_at(sound, pos);
            let near = 1.0 - (pos.distance(self.player.pos) / (radius * 5.0 + 10.0)).min(1.0);
            if self.screen == Screen::Playing && near > 0.0 {
                self.add_shake(0.3 * near * (radius / 4.0).min(1.5));
            }
        }

        // Melee blows that landed: a thud and a jolt.
        for (pos, weapon) in std::mem::take(&mut self.weapons.impacts) {
            let heavy = weapon == weapons::Weapon::Hammer;
            self.audio.play_at(if heavy { audio::Sfx::HammerHit } else { audio::Sfx::MeleeHit }, pos);
            self.add_shake(if heavy { 0.22 } else { 0.08 });
        }
        // Damage numbers, coloured by what caused them.
        let (mut zap, mut crit) = (false, false);
        for p in std::mem::take(&mut self.drones.popups) {
            use drones::Status;
            let (text, color, big) = match p.status {
                Status::Hit => (format!("{:.0}", p.amount.max(1.0)), egui::Color32::WHITE, false),
                Status::Crit => (format!("{:.0} CRIT", p.amount), egui::Color32::from_rgb(255, 90, 220), true),
                Status::Bleed => (format!("{:.0}", p.amount.max(1.0)), egui::Color32::from_rgb(235, 60, 60), false),
                Status::Burn => (format!("{:.0}", p.amount.max(1.0)), egui::Color32::from_rgb(255, 150, 40), false),
                Status::Shock => (format!("{:.0} SHOCK", p.amount), egui::Color32::from_rgb(110, 220, 255), true),
                Status::Stun => ("STUNNED".to_owned(), egui::Color32::from_rgb(255, 225, 90), false),
            };
            zap |= p.status == Status::Shock;
            crit |= p.status == Status::Crit;
            // A little scatter so numbers don't stack exactly.
            let jitter = glam::Vec3::new(((self.popups.len() * 37) % 7) as f32 * 0.12 - 0.36, 0.9, 0.0);
            if self.popups.len() < 40 {
                self.popups.push(FloatText { pos: p.pos + jitter, text, color, age: 0.0, big });
            }
        }
        if zap {
            self.audio.play(audio::Sfx::Zap);
        }
        if crit {
            self.audio.play(audio::Sfx::Crit);
        }
        for p in &mut self.popups {
            p.age += dt;
            p.pos.y += dt * 1.4;
        }
        self.popups.retain(|p| p.age < 1.1);
        if self.screen == Screen::Playing {
            self.update_vitals(dt);
        }

        // Match moments both players hear: the countdown to a fight.
        let phase = self.arena.state.phase;
        if phase == arena::Phase::Explore && self.net.as_ref().is_some_and(|n| n.connected()) {
            let second = self.arena.state.time_left.ceil() as i32;
            if second <= 3 && second >= 1 && second != self.last_beep {
                self.last_beep = second;
                self.audio.play(audio::Sfx::Beep);
            }
        }
        self.last_phase = phase;

        // Ambience follows where you are: windier up high, birds near the
        // ground, the rush of air at speed; the world muffles when paused.
        {
            let terrain = &self.scene.terrain;
            let (listener, forward) = (self.camera.position, self.camera.forward());
            self.audio.set_listener(listener, forward);
            let height = (listener.y - terrain.water_level).max(0.0);
            let altitude = (height / 70.0).clamp(0.0, 1.0);
            let speed = if self.screen == Screen::Playing { self.player.velocity().length() } else { 0.0 };
            let rush = ((speed - 9.0) / 40.0).clamp(0.0, 1.0);
            let wind = (0.25 + self.settings.wind * 0.5 + altitude * 0.5).min(1.0);
            let fighting = self.arena.fighting();
            let birds = if fighting { 0.0 } else { 1.0 - altitude };
            self.audio.set_ambience(wind, rush, birds);
            self.audio.set_muffled(self.menu_open && self.screen == Screen::Playing);
            self.audio.set_tension(if fighting { 1.0 } else { 0.0 });
            let s = &self.settings;
            self.audio.set_volumes(s.master_volume, s.sfx_volume, s.ambient_volume, s.music_volume);
        }

        // Fades between screens, and the switch to playing once dark.
        if self.starting {
            self.fade = (self.fade + dt * 3.0).min(1.0);
            if self.fade >= 1.0 {
                self.start_playing();
            }
        } else {
            self.fade = (self.fade - dt * 1.1).max(0.0);
        }
        if let Some(b) = &mut self.banner {
            b.age += dt;
            if b.age > b.life {
                self.banner = None;
            }
        }
        let hand = self.player.pos + glam::Vec3::Y * 1.2;
        self.gadgets.draw(&mut self.renderer, dt, hand, self.hook, self.marker);
        self.weapons.draw(&mut self.renderer);
        self.world_time += world_dt;
        if let Some((text, left)) = &mut self.toast {
            // A new message: restart its animation, with a soft tick.
            if *text != self.toast_shown {
                self.toast_shown.clone_from(text);
                self.toast_age = 0.0;
                if !self.menu_open {
                    self.audio.play(audio::Sfx::Toast);
                }
            }
            self.toast_age += dt;
            *left -= dt;
            if *left <= 0.0 {
                self.toast = None;
                self.toast_shown.clear();
            }
        }
        if self.third_person && self.screen == Screen::Playing {
            // Behind and a little above the player, looking where they look.
            self.camera.position = self.player.eye() - self.camera.forward() * 3.5 + glam::Vec3::Y * 0.4;
        }
        self.input.end_frame();

        // UI: the menu (which may change settings) or the HUD.
        let raw_input = self.egui_state.take_egui_input(&self.window);
        let moves = self.moves();
        let (slots, weapon_slots, meters) = self.hud_parts(&moves);
        let in_match = self.arena.in_match() && self.net.as_ref().is_some_and(|n| n.connected());
        let me = self.net.as_ref().map_or(0, |n| n.role.color_index());
        // The other player's head on screen, for their health bar.
        let enemy = self.remote.state.filter(|s| in_match && s.health > 0).and_then(|s| {
            let clip = self.camera.view_projection() * (self.remote.pos + glam::Vec3::Y * 2.2).extend(1.0);
            (clip.w > 0.1).then(|| (glam::Vec2::new(clip.x, clip.y) / clip.w, s.health as f32 / arena::MAX_HEALTH))
        });
        // World point -> screen (-1..1), if in front of the camera.
        let view_projection = self.camera.view_projection();
        let project = |p: glam::Vec3| {
            let clip = view_projection * p.extend(1.0);
            (clip.w > 0.1).then(|| glam::Vec2::new(clip.x, clip.y) / clip.w).filter(|n| n.x.abs() < 1.2 && n.y.abs() < 1.2)
        };
        let hud = ui::Hud {
            match_view: in_match.then(|| ui::MatchView {
                round: self.arena.state.round,
                phase: self.arena.state.phase,
                time_left: self.arena.state.time_left,
                scores: self.arena.state.scores,
                winner: self.arena.state.winner,
                me,
            }),
            enemy,
            dead: in_match && self.dead,
            update: self.updater.as_ref().map(|u| {
                let s = u.status();
                (s.describe(), matches!(s, updater::Status::Ready(_)))
            }),
            found: self.progress.found.len(),
            total: self.orbs.total(),
            kills: self.weapons.kills,
            net: ui::NetView {
                active: self.net.is_some(),
                is_host: self.net.as_ref().is_some_and(|n| n.is_host()),
                connected: self.net.as_ref().is_some_and(|n| n.connected()),
                status: self.net.as_ref().map(|n| n.status()).unwrap_or_default(),
                port_forwarding: self.net.as_ref().map(|n| n.port_forwarding_status()).unwrap_or_default(),
                join_code: self.net.as_ref().filter(|n| n.is_host()).and_then(|n| n.join_code()),
                public_address: self.net.as_ref().and_then(|n| n.public_address()),
                my_color: net::PLAYER_COLORS[self.net.as_ref().map_or(0, |n| n.role.color_index())],
                other_color: net::PLAYER_COLORS[1 - self.net.as_ref().map_or(0, |n| n.role.color_index())],
                lan_ip: self.lan_ip.clone(),
                steam_name: self.steam.as_ref().map(|s| s.my_name()),
                over_steam: self.net.as_ref().is_some_and(|n| n.over_steam()),
                steam_friends: self.steam_friends.clone(),
                invited: self.invited.clone(),
                steam_overlay: self.steam.as_ref().is_some_and(|s| s.overlay()),
                peer_name: match (&self.steam, self.net.as_ref().and_then(|n| n.peer_steam_id())) {
                    (Some(s), Some(id)) => Some(s.name_of(id)),
                    _ => None,
                },
            },
            slots,
            weapons: weapon_slots,
            meters,
            toast: self.toast.as_ref().map(|(t, left)| (t.as_str(), self.toast_age, *left)),
            fx: ui::Fx {
                hit: self.feel.hit,
                kill: self.feel.kill,
                damage: self.feel.damage,
                low_health: in_match && !self.dead && self.health < arena::MAX_HEALTH * 0.3,
                banner: self.banner.clone(),
                goals: if in_match || self.dead { Vec::new() } else { self.goals(&moves) },
                tutorial: if on_title { None } else { self.tutorial.as_ref().map(|t| t.view()) },
                time: self.clock,
                speed: ((self.player.velocity().length() - 18.0) / 35.0).clamp(0.0, 1.0),
                cast: self.casting.map(|(s, t, total)| ui::CastView {
                    name: s.name().to_owned(),
                    progress: (t / total).min(1.0),
                    overcharge: ((t - total) / OVERCHARGE_TIME).clamp(0.0, 1.0),
                    rune: rune_able(s),
                }),
                runes: self.runes.iter().map(|r| (r.spell.name().to_owned(), (r.fuse / RUNE_LIFE).clamp(0.0, 1.0), r.boost)).collect(),
                pending: if on_title { None } else { self.pending_prompt() },
            },
            on_title,
            slot_keys: self.slot_binds().iter().map(|b| b.label()).collect(),
            vitals: (!on_title).then(|| ui::VitalsView {
                health: self.health.max(0.0),
                max: moves.max_health,
                shield: self.vitals.shield,
                max_shield: moves.shield,
                ward: self.vitals.ward,
                trail: self.vitals.trail,
                regen: !self.dead && self.vitals.since_hurt > moves.regen_delay && self.health < moves.max_health,
                heal_flash: self.vitals.heal_flash,
                invulnerable: self.vitals.invulnerable > 0.0,
                adrenaline: moves.adrenaline && (moves.berserker || self.health < moves.max_health * 0.35),
                second_wind: moves.second_wind.then_some(self.vitals.second_wind <= 0.0),
            }),
            popups: self
                .popups
                .iter()
                .filter_map(|p| {
                    project(p.pos).map(|ndc| ui::ScreenText { ndc, text: p.text.clone(), color: p.color, age: p.age, big: p.big })
                })
                .collect(),
            drone_bars: if on_title {
                Vec::new()
            } else {
                let structure_bars = self.structures.bars().into_iter().filter_map(|(pos, health)| {
                    project(pos).map(|ndc| ui::BarView { ndc, health, scale: 1.3, bleeding: false, burning: false, stunned: false })
                });
                let clone_bars: Vec<ui::BarView> = self
                    .clones
                    .bars()
                    .into_iter()
                    .filter_map(|(pos, health)| project(pos).map(|ndc| ui::BarView { ndc, health, scale: 0.9, bleeding: false, burning: false, stunned: false }))
                    .collect();
                self.drones
                    .bars()
                    .into_iter()
                    .filter(|b| b.pos.distance(self.camera.position) < 60.0)
                    .filter_map(|b| {
                        let scale = (1.4 - b.pos.distance(self.camera.position) / 60.0).clamp(0.5, 1.2);
                        project(b.pos + glam::Vec3::Y * 1.2).map(|ndc| ui::BarView {
                            ndc,
                            health: b.health,
                            scale,
                            bleeding: b.bleeding,
                            burning: b.burning,
                            stunned: b.stunned,
                        })
                    })
                    .chain(structure_bars)
                    .chain(clone_bars)
                    .collect()
            },
        };

        let ranks_before = self.progress.ranks();
        let items_before = self.progress.items().len();
        let title_info = on_title.then(|| ui::TitleInfo {
            time: self.title_time,
            has_progress: self.progress.nodes() > 0 || !self.progress.found.is_empty() || self.progress.xp > 0.0,
            crystals: (self.progress.found.len(), self.orbs.total()),
            levels: self.progress.nodes(),
            version: env!("CARGO_PKG_VERSION"),
            update: hud.update.as_ref().filter(|(_, ready)| *ready).map(|(s, _)| s.clone()),
        });
        let (settings, progress, menu, menu_open, fps) =
            (&mut self.settings, &mut self.progress, &mut self.menu, self.menu_open, self.fps);
        let mut action = MenuAction::None;
        let mut title_action = ui::TitleAction::None;
        let fade = self.fade;
        let output = self.egui_ctx.run_pass(raw_input, |ctx| {
            match &title_info {
                Some(info) if !menu_open => title_action = ui::draw_title(ctx, info),
                Some(_) => action = ui::draw(ctx, settings, progress, menu, &hud, &moves, true, fps),
                None => action = ui::draw(ctx, settings, progress, menu, &hud, &moves, menu_open, fps),
            }
            ui::draw_fade(ctx, fade);
        });
        if (self.menu_open || on_title) && self.egui_ctx.input(|i| i.pointer.any_click()) {
            self.audio.play(audio::Sfx::Click);
        }
        match title_action {
            ui::TitleAction::Play if !self.starting => self.starting = true,
            ui::TitleAction::Multiplayer => {
                self.menu.tab = ui::Tab::Multiplayer;
                self.menu_open = true;
            }
            ui::TitleAction::Settings => {
                self.menu.tab = ui::Tab::Settings;
                self.menu_open = true;
            }
            ui::TitleAction::Quit => {
                self.quit(event_loop);
                return;
            }
            _ => {}
        }
        // A key pressed while binding a skill.
        if let Some(captured) = self.captured_bind.take()
            && let Some(slot) = self.menu.binding.take()
            && let Some(bind) = captured
        {
            self.set_slot_bind(slot, bind);
        }
        let right = std::mem::take(&mut self.menu.assign_right).then_some(input::Bind::Mouse(winit::event::MouseButton::Right));
        if let Some(bind) = self.assign_bind.take().or(right)
            && self.menu.binding.is_none()
        {
            self.assign_to(self.menu.selected, bind);
        }
        if !self.menu_open {
            self.menu.binding = None;
        }
        // An upgrade in the Abilities panel: save it and announce it.
        if let Some(skill) = abilities::Skill::ALL
            .into_iter()
            .find(|s| self.progress.rank(*s) != ranks_before[s.index()])
        {
            self.progress.save();
            let level = skill.rank_info(self.progress.rank(skill));
            self.notify(format!("Unlocked: {} - {}", level.name, level.description));
            self.audio.play(audio::Sfx::Unlock);
        }
        // An item bought on the Items tab.
        if self.progress.items().len() > items_before
            && let Some(&i) = self.progress.items().last()
        {
            self.progress.save();
            let it = &items::ITEMS[i];
            self.notify(format!("Unlocked {}: {}", it.name, it.description));
            self.audio.play(audio::Sfx::Unlock);
        }
        // Only let egui change the cursor while the menu is up; during play
        // the cursor stays hidden and captured.
        if self.menu_open || on_title {
            self.egui_state
                .handle_platform_output(&self.window, output.platform_output);
        }
        let mut ui_frame = UiFrame {
            primitives: self.egui_ctx.tessellate(output.shapes, output.pixels_per_point),
            textures_delta: output.textures_delta,
            pixels_per_point: output.pixels_per_point,
        };
        match action {
            MenuAction::Host(_) | MenuAction::Join(_) | MenuAction::HostSteam | MenuAction::JoinSteam(_) => {
                self.start_net(&action);
                // Hosting or joining from the title screen goes straight in.
                if on_title && self.net.is_some() {
                    self.menu_open = false;
                    self.starting = true;
                }
            }
            MenuAction::MainMenu => self.to_title(),
            MenuAction::InviteSteam => {
                if let Some(s) = &self.steam {
                    s.invite_friends();
                }
            }
            MenuAction::Tutorial => {
                self.tutorial = Some(tutorial::Tutorial::new());
                self.menu_open = false;
                self.set_mouse_captured(true);
            }
            MenuAction::InviteFriend(id) => {
                if let Some(s) = &self.steam {
                    s.invite(id);
                    let name = s.name_of(id);
                    if !self.invited.contains(&id) {
                        self.invited.push(id);
                    }
                    self.notify(format!("Invite sent to {name} on Steam."));
                    self.audio.play(audio::Sfx::Click);
                }
            }
            MenuAction::Disconnect => self.stop_net(),
            MenuAction::ResetAll => self.reset_everyone(true),
            MenuAction::Restart => {
                self.progress.save();
                self.save_settings_if_changed();
                updater::restart();
            }
            MenuAction::Resume if on_title => {
                self.menu_open = false;
                self.save_settings_if_changed();
            }
            MenuAction::Resume => self.close_menu(),
            MenuAction::Quit => {
                ui_frame.textures_delta.clear(); // not drawing this frame
                self.quit(event_loop);
                return;
            }
            MenuAction::None => {}
        }
        self.apply_settings();

        // Give the window a few frames to settle before a screenshot.
        self.frame_count += 1;
        if self.frame_count == 30
            && let Some(path) = self.screenshot.clone()
        {
            self.renderer.request_screenshot(path);
        }

        // The body animates even while paused (idle), so the menu doesn't freeze it.
        // On the title screen the robot idles in the square, facing out.
        let (body_dt, body_yaw, show_body) = if on_title {
            (dt, self.scene.spawn_yaw + std::f32::consts::PI * 0.15, true)
        } else {
            (player_dt, self.camera.yaw, self.third_person)
        };
        self.body.update(
            &mut self.renderer,
            body_dt,
            self.player.pos,
            body_yaw,
            self.player.grounded(),
            self.player.flight_direction(),
            show_body,
        );
        self.take_damage();
        self.transmit(dt);
        if !self.update_announced
            && let Some(updater::Status::Ready(v)) = self.updater.as_ref().map(|u| u.status())
        {
            self.update_announced = true;
            self.toast = Some((format!("Version {v} downloaded: Esc > Multiplayer > Restart now"), 8.0));
        }
        self.remote.draw(&mut self.renderer, dt);
        match self
            .renderer
            .render(&self.camera, &self.scene.lighting, self.world_time, ui_frame, self.rift_gates)
        {
            Ok(true) => event_loop.exit(), // screenshot saved
            Ok(false) => {}
            Err(e) => {
                log::error!("render failed: {e:#}");
                event_loop.exit();
                return;
            }
        }

        // FPS for the HUD, measured over half a second.
        self.fps_frames += 1;
        self.fps_timer += (Instant::now() - now).as_secs_f32().max(dt);
        if self.fps_timer >= 0.5 {
            self.fps = self.fps_frames as f32 / self.fps_timer;
            self.fps_frames = 0;
            self.fps_timer = 0.0;
        }
    }
}

#[derive(Default)]
struct App {
    game: Option<Game>,
    error: Option<anyhow::Error>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.game.is_some() {
            return;
        }
        match Game::new(event_loop) {
            Ok(game) => self.game = Some(game),
            Err(e) => {
                self.error = Some(e);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(game) = self.game.as_mut() else {
            return;
        };
        // egui sees every event (it needs resizes, mouse position, etc.);
        // the game only gets input while the menu is closed, except Escape.
        let _ = game.egui_state.on_window_event(&game.window, &event);
        match event {
            WindowEvent::CloseRequested => game.quit(event_loop),
            WindowEvent::Resized(size) => {
                game.renderer.resize(size.width, size.height);
                if size.height > 0 {
                    game.camera.aspect = size.width as f32 / size.height as f32;
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let released = event.state == winit::event::ElementState::Released;
                if let PhysicalKey::Code(code) = event.physical_key {
                    // Waiting for a key to bind a skill to: this is it (Esc cancels).
                    if game.menu.binding.is_some() && !released && !event.repeat {
                        game.captured_bind = Some(if code == KeyCode::Escape { None } else { Some(input::Bind::Key(code)) });
                        return;
                    }
                    // On the Skills tab, a skill key puts the chosen skill on it
                    // (unless you're typing in a text box).
                    if game.choosing_keys() && !released && !event.repeat && input::Bind::Key(code).allowed() {
                        game.assign_bind = Some(input::Bind::Key(code));
                        return;
                    }
                    // Releases always go through so no key stays stuck "held";
                    // Esc and Tab close the menu.
                    if !game.menu_open || code == KeyCode::Escape || code == KeyCode::Tab || released {
                        game.input.on_key(code, event.state, event.repeat);
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let pressed = state == winit::event::ElementState::Pressed;
                if game.menu.binding.is_some() && pressed && button != winit::event::MouseButton::Left {
                    game.captured_bind = Some(Some(input::Bind::Mouse(button)));
                    return;
                }
                // Middle and side buttons on the Skills tab (right click is
                // seen by the menu, so it can still change a slot's key).
                use winit::event::MouseButton as B;
                if game.choosing_keys() && pressed && matches!(button, B::Middle | B::Back | B::Forward) && input::Bind::Mouse(button).allowed() {
                    game.assign_bind = Some(input::Bind::Mouse(button));
                    return;
                }
                if !game.menu_open || !pressed {
                    game.input.on_mouse_button(button, state);
                }
            }
            WindowEvent::Focused(false) => {
                game.input.release_all();
                // Pause if the player was in the middle of playing.
                if game.mouse_captured {
                    game.open_menu();
                }
            }
            WindowEvent::RedrawRequested => game.update_and_render(event_loop),
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        if let (Some(game), DeviceEvent::MouseMotion { delta }) = (self.game.as_mut(), event)
            && game.mouse_captured
        {
            game.input.on_mouse_motion(delta.0, delta.1);
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(game) = &self.game {
            game.window.request_redraw();
        }
    }
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App::default();
    event_loop.run_app(&mut app)?;
    match app.error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}
