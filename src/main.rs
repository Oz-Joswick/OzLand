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
mod player;
mod renderer;
mod scene;
mod settings;
mod terrain;
mod textures;
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
const TITLE: &str = "3D Walkabout";
#[derive(Clone, Copy, PartialEq, Eq)]
enum TimeEffect {
    Bullet,
    Stop,
}

/// The notice when a capstone trial is completed.
fn trial_message(tree: abilities::Tree) -> String {
    let capstone = tree.level_info(abilities::MAX_LEVEL);
    format!("Trial complete! {} can now be unlocked ({} points).", capstone.name, abilities::CAPSTONE_COST)
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
        let drones = drones::Drones::new(&mut renderer, &scene.terrain, scene.spawn);
        let orbs = orbs::Orbs::new(&mut renderer, scene.crystals.clone(), &progress.found);
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
        egui_ctx.set_visuals(egui::Visuals::dark());
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
            remote: Remote { body: remote_body, state: None, pos: glam::Vec3::ZERO, yaw: 0.0 },
            lan_ip: net::lan_address(),
            // Dev aid: WALKABOUT_MENU=<tree name> opens the menu on that tree.
            menu: ui::MenuState {
                selected: std::env::var("WALKABOUT_MENU")
                    .ok()
                    .and_then(|v| abilities::Tree::ALL.into_iter().find(|t| t.name().eq_ignore_ascii_case(&v)))
                    .unwrap_or(abilities::Tree::Movement),
                tab: match std::env::var("WALKABOUT_MENU").as_deref() {
                    Ok("settings") => ui::Tab::Settings,
                    Ok("items") => ui::Tab::Items,
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
        };
        game.apply_settings();
        // Multiplayer from the command line: `--host [port]` or `--join address`
        // (or WALKABOUT_HOST / WALKABOUT_JOIN).
        if let Some(action) = startup_net_action() {
            game.start_net(&action);
        }
        let capture = fixed_view.is_none() && game.screenshot.is_none() && !game.menu_open;
        game.set_mouse_captured(capture);
        Ok(game)
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
        if self.settings.unlock_all {
            let mut levels = [abilities::MAX_LEVEL; abilities::TREES];
            levels[abilities::Tree::Ascension.index()] = self.settings.sandbox_ascension as u32;
            let mut m = abilities::Moveset::from_levels(levels);
            items::apply(&mut m, &items::all());
            m
        } else {
            let mut m = abilities::Moveset::from_levels(self.progress.levels());
            items::apply(&mut m, self.progress.items());
            m
        }
    }

    /// The HUD's model: an ability tile for everything you can trigger, the
    /// weapon bar, and movement meters.
    fn hud_parts(&self, moves: &abilities::Moveset) -> (Vec<ui::Slot>, Vec<ui::WeaponSlot>, ui::Meters) {
        use ui::{Slot, SlotState};
        let p = &self.player;
        let mut slots = Vec::new();
        let mut slot = |key: &'static str, name: &str, state: SlotState, charges: Option<(u32, u32)>| {
            slots.push(Slot { key, name: name.to_owned(), state, charges });
        };
        if moves.dash_charges > 0 {
            let ready = self.dash_ready.min(moves.dash_charges);
            let state = if ready > 0 {
                SlotState::Ready
            } else {
                SlotState::Cooldown(1.0 - self.dash_timer / moves.dash_recharge)
            };
            slot("F", "Dash", state, Some((ready, moves.dash_charges)));
        }
        if moves.grapple_range > 0.0 {
            let state = if p.hooked() { SlotState::Active(1.0) } else { SlotState::Ready };
            slot("E", "Grapple", state, None);
        }
        if moves.pads > 0 {
            let placed = self.gadgets.pad_count() as u32;
            slot("P", "Pad", SlotState::Ready, Some((placed, moves.pads as u32)));
        }
        if moves.max_clones > 0 {
            let out = self.clones.count() as u32;
            slot("C", "Clone", SlotState::Ready, Some((out, moves.max_clones as u32)));
            if moves.mirror_clones {
                let mirror = self.clones.mode == clones::Mode::Mirror;
                let state = if mirror { SlotState::Active(1.0) } else { SlotState::Ready };
                slot("M", if mirror { "Mirror" } else { "Still" }, state, None);
            }
            if moves.switch_places {
                let state = if out > 0 { SlotState::Ready } else { SlotState::Unavailable };
                slot("X", "Swap", state, None);
            }
        }
        if moves.recall {
            let state = if self.marker.is_some() { SlotState::Active(1.0) } else { SlotState::Ready };
            slot("B", if self.marker.is_some() { "Recall" } else { "Marker" }, state, None);
        }
        if moves.rewind > 0.0 {
            let state = if self.rewind_cooldown > 0.0 {
                SlotState::Cooldown(self.rewind_cooldown / 2.0)
            } else {
                SlotState::Ready
            };
            slot("Z", "Rewind", state, None);
        }
        if moves.bullet_time {
            let (name, length) = if moves.time_stop { ("Stop", moves.time_stop_length) } else { ("Slow-mo", 5.0) };
            let state = match self.time_effect {
                Some((_, left)) => SlotState::Active(left / length),
                None if self.time_cooldown > 0.0 => SlotState::Cooldown(self.time_cooldown / moves.time_cooldown),
                None => SlotState::Ready,
            };
            slot("T", name, state, None);
        }
        if moves.low_gravity {
            let state = if p.low_gravity() { SlotState::Active(1.0) } else { SlotState::Ready };
            slot("G", "Low grav", state, None);
        }
        if moves.free_flight || moves.hover_time > 0.0 {
            let name = if moves.free_flight {
                "Fly"
            } else if moves.levitate {
                "Levitate"
            } else {
                "Hover"
            };
            let state = if moves.free_flight || p.hover_left() > 0.05 || p.grounded() {
                SlotState::Ready
            } else {
                SlotState::Unavailable
            };
            slot("RMB", name, state, None);
        }

        let weapons = weapons::Weapon::ALL
            .into_iter()
            .map(|w| ui::WeaponSlot {
                weapon: w,
                unlocked: moves.weapons >= w.unlocked_at(),
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
        if moves.glide && !p.grounded() {
            meters.rockets = Some((moves.boosts.saturating_sub(p.boosts_used()), moves.boosts));
        }
        if p.wall_running() {
            meters.status.push("Wall run");
        }
        if p.low_gravity() {
            meters.status.push("Low gravity");
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
        let result = match action {
            MenuAction::Host(port) => net::Net::host(*port),
            MenuAction::Join(address) => net::Net::join(address),
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
        let (colliders, terrain) = (&self.scene.colliders, &self.scene.terrain);
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
                net::Msg::Fire(f) => {
                    let shooters = [(f.origin, f.dir)];
                    let events = match weapons::Weapon::from_index(f.weapon) {
                        Some(weapon) => {
                            let volley = weapons::Volley {
                                weapon,
                                damage: f.damage,
                                pellets: f.pellets as u32,
                                blast_radius: f.blast_radius,
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
                net::Msg::Died if is_host => joiner_fell = true,
                net::Msg::ResetAll => reset = true,
                net::Msg::Drones(states) if !is_host => {
                    for at in self.drones.apply_states(&states) {
                        self.weapons.add_blast(at, 1.6);
                    }
                }
                net::Msg::Kills(n) if !is_host => {
                    self.weapons.kills += n;
                    if let Some(tree) = self.progress.record(abilities::Stat::Kills, n as f32) {
                        self.toast = Some((trial_message(tree), 5.0));
                        self.progress.save();
                    }
                    let gained = self.progress.add_xp(n as f32 * 6.0);
                    let scrap = n * items::DRONE_SCRAP * self.moves().scrap_multiplier;
                    self.progress.scrap += scrap;
                    self.toast = Some((format!("Drone destroyed! +{} XP, +{scrap} scrap", n * 6), 3.0));
                    if gained {
                        self.progress.save();
                        self.toast = Some(("+1 skill point from experience! Esc to spend it.".into(), 3.0));
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
                (arena::Phase::Explore, arena::Phase::Fight) => self.notify("FIGHT!"),
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
            self.notify("FIGHT!");
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

    /// Back in the round: at `spawn`, full health, air abilities refilled.
    fn respawn(&mut self, spawn: glam::Vec3) {
        self.player.teleport(spawn);
        self.player.refill(&self.moves());
        self.health = arena::MAX_HEALTH;
        self.dead = false;
        self.pending_damage = 0.0;
        self.hook = None;
        self.trail.clear();
        self.last_pos = spawn;
        self.body.reset_motion();
        let round = self.arena.state.round;
        self.notify(format!("Round {round}: {:.0} s to explore, then fight!", arena::EXPLORE_TIME));
    }

    /// Applies damage from the other player's shots (only during a fight).
    fn take_damage(&mut self) {
        let damage = std::mem::take(&mut self.pending_damage);
        if damage <= 0.0 || self.dead || !self.arena.fighting() {
            return;
        }
        self.health -= damage;
        self.shake = self.shake.max(0.15);
        if self.health > 0.0 {
            return;
        }
        // Out of the round.
        self.health = 0.0;
        self.dead = true;
        self.weapons.add_blast(self.player.pos + glam::Vec3::Y, 2.5);
        match &self.net {
            Some(net) if net.is_host() => {
                self.arena.report_death(0);
                self.round_result();
            }
            Some(net) => net.send(&net::Msg::Died),
            None => {}
        }
    }

    /// Host: announce who took the round.
    fn round_result(&mut self) {
        if let Some(w) = self.arena.state.winner {
            let name = ["Gold", "Teal"][w as usize];
            self.notify(format!("{name} wins the round!"));
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
        let Some(net) = &mut self.net else { return };
        if net.state_due(dt) {
            net.send(&net::Msg::State(net::PlayerState {
                pos: self.player.pos,
                yaw: self.camera.yaw,
                grounded: self.player.grounded(),
                flight: self.player.flight_direction(),
                health: if self.dead { 0 } else { self.health.ceil().clamp(1.0, 255.0) as u8 },
            }));
        }
        if net.drones_due(dt) {
            net.send(&net::Msg::Drones(self.drones.states()));
        }
    }

    /// Sends shots you fired so the other player sees them (and, if they
    /// host, applies their damage).
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
        self.receive(dt);
        self.run_match(dt);

        // Escape toggles the pause/settings menu.
        if self.input.pressed(KeyCode::Escape) {
            if self.menu_open {
                self.close_menu();
            } else {
                self.open_menu();
            }
        } else if !self.menu_open {
            if self.input.pressed(KeyCode::KeyQ) {
                self.quit(event_loop);
                return;
            }
            if self.input.pressed(KeyCode::KeyV) {
                self.third_person = !self.third_person;
            }
            if self.input.left_clicked() && !self.mouse_captured {
                self.set_mouse_captured(true);
            }
        }

        // The world is paused while the menu is open. Bullet time and time
        // stop slow the world down more than you.
        let mut world_dt = if self.menu_open { 0.0 } else { dt };
        let mut player_dt = world_dt;
        if !self.menu_open {
            let input = &self.input;
            let mut message: Option<String> = None;
            // A capstone trial completed this frame.
            let mut trial: Option<abilities::Tree> = None;
            if self.mouse_captured {
                let mut delta = input.mouse_delta();
                if self.settings.invert_y {
                    delta.y = -delta.y;
                }
                self.camera
                    .rotate(delta, MOUSE_SENSITIVITY * self.settings.mouse_sensitivity);
            }
            let shift = [KeyCode::ShiftLeft, KeyCode::ShiftRight];
            let controls = Controls {
                forward: input.axis(&[KeyCode::KeyS, KeyCode::ArrowDown], &[KeyCode::KeyW, KeyCode::ArrowUp]),
                right: input.axis(&[KeyCode::KeyA, KeyCode::ArrowLeft], &[KeyCode::KeyD, KeyCode::ArrowRight]),
                sprint: input.any_held(&shift),
                jump: input.pressed(KeyCode::Space),
                pitch: self.camera.pitch,
                slam: input.pressed(KeyCode::ControlLeft) || input.pressed(KeyCode::ControlRight),
                boost: input.pressed(KeyCode::ShiftLeft) || input.pressed(KeyCode::ShiftRight),
                levitate: input.right_held(),
                gravity_toggle: input.pressed(KeyCode::KeyG),
            };
            // Out of the round: you can look around, but not move.
            let controls = if self.dead { Controls { pitch: self.camera.pitch, ..Default::default() } } else { controls };
            let moves = self.moves();
            let (colliders, terrain) = (&self.scene.colliders, &self.scene.terrain);

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
            if input.pressed(KeyCode::KeyT) && self.time_effect.is_none() && self.time_cooldown <= 0.0 {
                if moves.time_stop {
                    self.time_effect = Some((TimeEffect::Stop, moves.time_stop_length));
                    message = Some("Time stopped.".into());
                } else if moves.bullet_time {
                    self.time_effect = Some((TimeEffect::Bullet, 5.0));
                    message = Some("Bullet time.".into());
                }
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
            if input.pressed(KeyCode::KeyF)
                && self.dash_ready > 0
                && !self.dead
                && if moves.dash_any_direction {
                    self.player.blink_free(self.camera.forward(), moves.dash_distance, colliders, terrain)
                } else {
                    self.player.blink(self.camera.forward(), moves.dash_distance, colliders, terrain)
                }
            {
                self.dash_ready -= 1;
                trial = trial.or(self.progress.record(abilities::Stat::Dashes, 1.0));
                self.body.reset_motion();
            }

            // Recall marker and rewind.
            self.play_time += player_dt;
            self.trail.push_back((self.play_time, self.player.pos));
            while self.trail.front().is_some_and(|(t, _)| self.play_time - t > 6.5) {
                self.trail.pop_front();
            }
            self.rewind_cooldown = (self.rewind_cooldown - dt).max(0.0);
            let mut jumped_back = None;
            if input.pressed(KeyCode::KeyB) && moves.recall {
                match self.marker.take() {
                    None => {
                        self.marker = Some(self.player.pos);
                        message = Some("Marker dropped. B to return to it.".into());
                    }
                    Some(p) => jumped_back = Some(p),
                }
            }
            if input.pressed(KeyCode::KeyZ) && moves.rewind > 0.0 && self.rewind_cooldown <= 0.0 {
                jumped_back = self
                    .trail
                    .iter()
                    .find(|(t, _)| self.play_time - t <= moves.rewind)
                    .map(|&(_, p)| p);
                self.rewind_cooldown = 2.0;
                if jumped_back.is_some() {
                    trial = trial.or(self.progress.record(abilities::Stat::Rewinds, 1.0));
                }
            }
            if let Some(p) = jumped_back {
                self.player.teleport(p);
                if moves.time_refill {
                    self.player.refill(&moves);
                }
                self.trail.clear();
                self.body.reset_motion();
            }

            // Clones.
            let (spawn, switch, mode, dismiss) = (
                input.pressed(KeyCode::KeyC),
                input.pressed(KeyCode::KeyX),
                input.pressed(KeyCode::KeyM),
                input.pressed(KeyCode::KeyR),
            );
            if mode && moves.mirror_clones {
                self.clones.mode = match self.clones.mode {
                    clones::Mode::Stationary => clones::Mode::Mirror,
                    clones::Mode::Mirror => clones::Mode::Stationary,
                };
                message = Some(format!("Next clone: {}", self.clones.mode.name()));
            }
            if !moves.mirror_clones {
                self.clones.mode = clones::Mode::Stationary;
            }
            if spawn && moves.max_clones > 0 {
                self.clones
                    .spawn(&self.player, self.camera.yaw, moves.max_clones, &mut self.renderer);
            }
            if dismiss {
                self.clones.dismiss_all(&mut self.renderer);
            }
            if switch && moves.switch_places && self.clones.switch_with_newest(&mut self.player) {
                self.body.reset_motion();
                trial = trial.or(self.progress.record(abilities::Stat::Swaps, 1.0));
            }

            // Gadgets: pads (P) and the grapple (hold E).
            if input.pressed(KeyCode::KeyP) && moves.pads > 0 {
                self.gadgets.place_pad(self.player.pos, moves.pads);
            }
            if moves.pads == 0 {
                self.gadgets.clear_pads();
            }
            let e_held = input.any_held(&[KeyCode::KeyE]);
            if input.pressed(KeyCode::KeyE) && moves.grapple_range > 0.0 {
                let aim = gadgets::raycast(self.camera.position, self.camera.forward(), moves.grapple_range, colliders, terrain);
                match aim {
                    Some(point) => {
                        self.player.set_hook(point);
                        self.hook = Some(point);
                    }
                    None => message = Some("Nothing in reach to hook.".into()),
                }
            } else if self.player.hooked() && (!e_held || moves.grapple_range <= 0.0) {
                self.player.release_hook(moves.slingshot, &moves);
            }
            if !self.player.hooked() {
                self.hook = None;
            }
            // Weapons: 1-4 choose, left mouse fires at the crosshair.
            let mut kills = 0;
            let mut remote_kills = 0;
            let mut fired = None;
            let authority = self.net.as_ref().is_none_or(|n| n.is_host() || !n.connected());
            let picks = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4];
            for (key, weapon) in picks.into_iter().zip(weapons::Weapon::ALL) {
                if input.pressed(key) && self.weapons.select(weapon, &moves) {
                    message = Some(weapon.name().into());
                }
            }
            self.weapons.reload(dt);
            if captured_before && input.left_held() && moves.weapons > 0 && !self.dead {
                let (eye, aim) = (self.camera.position, self.camera.forward());
                let world = gadgets::raycast(eye, aim, 250.0, colliders, terrain).map_or(250.0, |p| p.distance(eye));
                let reach = self.drones.ray_hit(eye, aim, world).map_or(world, |(_, t)| t);
                let target = eye + aim * reach;
                let side = glam::Vec3::new(-self.camera.yaw.sin(), 0.0, self.camera.yaw.cos());
                let hand = self.player.pos + glam::Vec3::Y * 1.35 + side * 0.3;
                let mut shooters = vec![(hand, target - hand)];
                if moves.clones_fire {
                    let pitch = self.camera.pitch;
                    for (pos, yaw) in self.clones.shooters() {
                        let dir = glam::Vec3::new(yaw.cos() * pitch.cos(), pitch.sin(), yaw.sin() * pitch.cos());
                        shooters.push((pos + glam::Vec3::Y * 1.35, dir));
                    }
                }
                let authority = self.net.as_ref().is_none_or(|n| n.is_host() || !n.connected());
                let (events, volley) = self.weapons.fire(&shooters, &moves, authority, &mut self.drones, colliders, terrain);
                kills += events.kills;
                remote_kills += events.remote_kills;
                fired = volley.map(|v| (v, shooters));
            }

            self.gadgets.bounce(&mut self.player, moves.pad_launch);
            for clone in self.clones.players_mut() {
                self.gadgets.bounce(clone, moves.pad_launch);
            }

            let slammed = self
                .player
                .update(player_dt, &controls, self.camera.yaw, colliders, terrain, &moves);
            if slammed {
                self.shake = 0.35;
                kills += self
                    .weapons
                    .shockwave(self.player.pos, moves.weapon_damage, false, authority, &mut self.drones)
                    .kills;
                if let Some(net) = self.net.as_ref().filter(|n| n.connected()) {
                    // Weapon number 255: a slam's shockwave.
                    net.send(&net::Msg::Fire(net::FireMsg {
                        weapon: 255,
                        origin: self.player.pos,
                        dir: glam::Vec3::Y,
                        damage: moves.weapon_damage,
                        pellets: 0,
                        blast_radius: 6.0,
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
                self.shake = self.shake.max(0.2);
            }
            kills += shots.kills;
            remote_kills += shots.remote_kills;
            self.drones.update(&mut self.renderer, world_dt, self.player.pos, authority);
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
                &self.scene.colliders,
                &self.scene.terrain,
                &moves,
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
            if p.grounded() {
                let v = p.velocity();
                let foot = glam::Vec2::new(v.x, v.z).length();
                trial = trial.or(self.progress.record_best(abilities::Stat::FootSpeed, foot));
            }
            if let Some(tree) = trial {
                self.progress.save();
                message = Some(trial_message(tree));
            }
            if kills > 0 {
                xp += kills as f32 * 6.0;
                let scrap = kills * items::DRONE_SCRAP * moves.scrap_multiplier;
                self.progress.scrap += scrap;
                message = Some(format!("Drone destroyed! +{} XP, +{scrap} scrap", kills * 6));
            }
            if self.progress.add_xp(xp) {
                self.progress.save();
                message = Some("+1 skill point from experience! Esc to spend it.".into());
            }

            // Skill crystals.
            let chest = self.player.pos + glam::Vec3::Y * 1.0;
            if let Some(crystal) = self.orbs.update(&mut self.renderer, world_dt, chest)
                && self.progress.collect(crystal)
            {
                self.progress.save();
                let (found, total) = (self.progress.found.len(), self.orbs.total());
                self.progress.scrap += items::CRYSTAL_SCRAP;
                message = Some(format!("Skill crystal found! +1 point, +{} scrap ({found}/{total}). Esc to spend.", items::CRYSTAL_SCRAP));
            }

            self.camera.position = self.player.eye();
            if self.shake > 0.0 {
                let t = self.shake * 60.0;
                self.camera.position += glam::Vec3::new(t.sin(), (t * 1.3).cos(), 0.0) * self.shake * 0.25;
                self.shake -= dt;
            }
            if let Some(m) = message {
                self.notify(m);
            }
        }
        let hand = self.player.pos + glam::Vec3::Y * 1.2;
        self.gadgets.draw(&mut self.renderer, dt, hand, self.hook, self.marker);
        self.weapons.draw(&mut self.renderer);
        self.world_time += world_dt;
        if let Some((_, left)) = &mut self.toast {
            *left -= dt;
            if *left <= 0.0 {
                self.toast = None;
            }
        }
        if self.third_person {
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
        let hud = ui::Hud {
            match_view: in_match.then(|| ui::MatchView {
                round: self.arena.state.round,
                phase: self.arena.state.phase,
                time_left: self.arena.state.time_left,
                scores: self.arena.state.scores,
                winner: self.arena.state.winner,
                me,
            }),
            health: in_match.then_some(self.health / arena::MAX_HEALTH),
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
                connected: self.net.as_ref().is_some_and(|n| n.connected()),
                status: self.net.as_ref().map(|n| n.status()).unwrap_or_default(),
                my_color: net::PLAYER_COLORS[self.net.as_ref().map_or(0, |n| n.role.color_index())],
                other_color: net::PLAYER_COLORS[1 - self.net.as_ref().map_or(0, |n| n.role.color_index())],
                lan_ip: self.lan_ip.clone(),
            },
            slots,
            weapons: weapon_slots,
            meters,
            toast: self.toast.as_ref().map(|(t, _)| t.as_str()),
        };

        let levels_before = self.progress.levels();
        let items_before = self.progress.items().len();
        let (settings, progress, menu, menu_open, fps) =
            (&mut self.settings, &mut self.progress, &mut self.menu, self.menu_open, self.fps);
        let mut action = MenuAction::None;
        let output = self.egui_ctx.run_pass(raw_input, |ctx| {
            action = ui::draw(ctx, settings, progress, menu, &hud, &moves, menu_open, fps);
        });
        // An upgrade in the Abilities panel: save it and announce it.
        if let Some(tree) = abilities::Tree::ALL
            .into_iter()
            .find(|t| self.progress.level(*t) != levels_before[t.index()])
        {
            self.progress.save();
            let level = tree.level_info(self.progress.level(tree));
            self.notify(format!("Unlocked: {} - {}", level.name, level.description));
        }
        // An item bought on the Items tab.
        if self.progress.items().len() > items_before
            && let Some(&i) = self.progress.items().last()
        {
            self.progress.save();
            let it = &items::ITEMS[i];
            self.notify(format!("Unlocked {}: {}", it.name, it.description));
        }
        // Only let egui change the cursor while the menu is up; during play
        // the cursor stays hidden and captured.
        if self.menu_open {
            self.egui_state
                .handle_platform_output(&self.window, output.platform_output);
        }
        let mut ui_frame = UiFrame {
            primitives: self.egui_ctx.tessellate(output.shapes, output.pixels_per_point),
            textures_delta: output.textures_delta,
            pixels_per_point: output.pixels_per_point,
        };
        match action {
            MenuAction::Host(_) | MenuAction::Join(_) => self.start_net(&action),
            MenuAction::Disconnect => self.stop_net(),
            MenuAction::ResetAll => self.reset_everyone(true),
            MenuAction::Restart => {
                self.progress.save();
                self.save_settings_if_changed();
                updater::restart();
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
        let body_dt = player_dt;
        self.body.update(
            &mut self.renderer,
            body_dt,
            self.player.pos,
            self.camera.yaw,
            self.player.grounded(),
            self.player.flight_direction(),
            self.third_person,
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
            .render(&self.camera, &self.scene.lighting, self.world_time, ui_frame)
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
                // Releases always go through so no key stays stuck "held".
                let released = event.state == winit::event::ElementState::Released;
                if let PhysicalKey::Code(code) = event.physical_key
                    && (!game.menu_open || code == KeyCode::Escape || released)
                {
                    game.input.on_key(code, event.state, event.repeat);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if !game.menu_open {
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
