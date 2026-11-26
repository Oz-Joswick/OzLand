//! Two-player networking over UDP.
//!
//! One player **hosts** (listens on a port) and the other **joins** (sends
//! to the host's address). Each side sends its player's state about 30
//! times a second and an event for every shot fired; the host is in charge
//! of the practice drones: it applies all drone damage, sends drone
//! positions to the joiner, and credits the joiner's kills back to them.
//!
//! Packets are small hand-packed binary messages, each starting with a
//! magic byte and protocol version, so stray traffic is ignored.

use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, ToSocketAddrs, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use glam::Vec3;
use igd_next::{Gateway, PortMappingProtocol, SearchOptions, search_gateway};

const MAGIC: u8 = 0xB7;
const VERSION: u8 = 2;
/// This build's version: both players must run the same one.
pub const GAME_VERSION: &str = env!("CARGO_PKG_VERSION");
/// No packets for this long and the other player is considered gone.
const TIMEOUT: Duration = Duration::from_secs(5);
/// Seconds between player-state packets.
const STATE_INTERVAL: f32 = 1.0 / 30.0;
/// Seconds between drone updates from the host.
const DRONE_INTERVAL: f32 = 1.0 / 15.0;
pub const DEFAULT_PORT: u16 = 7777;

/// Each player's colour (linear RGB): the host is gold, the joiner teal.
pub const PLAYER_COLORS: [Vec3; 2] = [Vec3::new(0.59, 0.29, 0.038), Vec3::new(0.02, 0.36, 0.42)];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Host,
    Client,
}

impl Role {
    /// Index into [`PLAYER_COLORS`] for this side's player.
    pub fn color_index(self) -> usize {
        match self {
            Role::Host => 0,
            Role::Client => 1,
        }
    }
}

/// Where a player is and what they're doing (enough to draw them).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerState {
    pub pos: Vec3,
    pub yaw: f32,
    pub grounded: bool,
    /// Direction of flight while gliding or flying (the body leans into it).
    pub flight: Option<Vec3>,
    /// 0 when out of the round.
    pub health: u8,
}

/// One shot fired (one per shooter: player and Overcharge clones).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FireMsg {
    pub weapon: u8,
    pub origin: Vec3,
    pub dir: Vec3,
    pub damage: f32,
    pub pellets: u8,
    pub blast_radius: f32,
    /// Weapon upgrades that change its effects (see `weapons::Mods`).
    pub mods: u8,
    /// Seeking bolts (Arcane Missiles).
    pub homing: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GatePoint {
    pub pos: Vec3,
    pub yaw: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GatePair {
    pub a: GatePoint,
    pub b: GatePoint,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Msg {
    /// Joiner -> host, repeated until welcomed, with the game version.
    Hello(String),
    /// Host -> joiner: different game versions (the host's).
    Reject(String),
    /// Host -> joiner.
    Welcome,
    State(PlayerState),
    Fire(FireMsg),
    /// Host -> joiner: every drone's position and whether it's up.
    Drones(Vec<(Vec3, bool)>),
    /// Host -> joiner: drones the joiner destroyed.
    Kills(u32),
    /// Host -> joiner: the match, and the joiner's spawn for this round.
    Round(crate::arena::MatchState, Vec3),
    /// Joiner -> host: the joiner fell this round.
    Died,
    /// Either way: reset every player's progress and the score.
    ResetAll,
    /// Either way: something built (a barrier, ceiling or dome).
    Build(crate::structures::Structure),
    /// Either way: a structure broke.
    Unbuild(u32),
    /// Either way: you were yanked or thrown (velocity change), pinned to the
    /// ground and stunned for so many seconds.
    Push { impulse: Vec3, pin: f32, stun: f32 },
    /// Either way: one player's linked Rift Gate endpoints.
    RiftGate { owner: u8, pair: GatePair },
    /// A crater blasted in the ground: where, how wide and deep.
    Crater { pos: Vec3, radius: f32, depth: f32 },
    Bye,
}

// ---------------------------------------------------------------- encoding

struct Writer(Vec<u8>);

impl Writer {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn vec3(&mut self, v: Vec3) {
        self.f32(v.x);
        self.f32(v.y);
        self.f32(v.z);
    }
    fn text(&mut self, s: &str) {
        let bytes = &s.as_bytes()[..s.len().min(64)];
        self.u8(bytes.len() as u8);
        self.0.extend_from_slice(bytes);
    }
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let (head, rest) = self.0.split_first_chunk::<N>()?;
        self.0 = rest;
        Some(*head)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take::<1>().map(|b| b[0])
    }
    fn f32(&mut self) -> Option<f32> {
        let v = f32::from_le_bytes(self.take()?);
        v.is_finite().then_some(v)
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take()?))
    }
    fn vec3(&mut self) -> Option<Vec3> {
        Some(Vec3::new(self.f32()?, self.f32()?, self.f32()?))
    }
    fn text(&mut self) -> Option<String> {
        let n = self.u8()? as usize;
        if self.0.len() < n {
            return None;
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        String::from_utf8(head.to_vec()).ok()
    }
}

impl Msg {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer(vec![MAGIC, VERSION]);
        match self {
            Msg::Hello(v) => {
                w.u8(0);
                w.text(v);
            }
            Msg::Welcome => w.u8(1),
            Msg::State(s) => {
                w.u8(2);
                w.vec3(s.pos);
                w.f32(s.yaw);
                w.u8(s.grounded as u8 | (s.flight.is_some() as u8) << 1);
                w.vec3(s.flight.unwrap_or(Vec3::ZERO));
                w.u8(s.health);
            }
            Msg::Fire(f) => {
                w.u8(3);
                w.u8(f.weapon);
                w.vec3(f.origin);
                w.vec3(f.dir);
                w.f32(f.damage);
                w.u8(f.pellets);
                w.f32(f.blast_radius);
                w.u8(f.mods);
                w.u8(f.homing as u8);
            }
            Msg::Drones(list) => {
                w.u8(4);
                w.u8(list.len().min(255) as u8);
                for &(pos, alive) in list.iter().take(255) {
                    w.vec3(pos);
                    w.u8(alive as u8);
                }
            }
            Msg::Kills(n) => {
                w.u8(5);
                w.u32(*n);
            }
            Msg::Bye => w.u8(6),
            Msg::Reject(v) => {
                w.u8(7);
                w.text(v);
            }
            Msg::Round(m, spawn) => {
                w.u8(8);
                w.u32(m.round);
                w.u8(m.phase.code());
                w.f32(m.time_left);
                w.u32(m.scores[0]);
                w.u32(m.scores[1]);
                w.u8(m.winner.map_or(255, |x| x));
                w.vec3(*spawn);
            }
            Msg::Died => w.u8(9),
            Msg::ResetAll => w.u8(10),
            Msg::Build(s) => {
                w.u8(11);
                w.u32(s.id);
                w.u8(s.kind.code());
                w.vec3(s.pos);
                w.f32(s.yaw);
                w.f32(s.health);
                w.f32(s.max_health);
                w.f32(s.life);
                w.u8(s.warden as u8);
                w.f32(s.scale);
                w.f32(s.power);
            }
            Msg::Unbuild(id) => {
                w.u8(12);
                w.u32(*id);
            }
            Msg::Crater { pos, radius, depth } => {
                w.u8(15);
                w.vec3(*pos);
                w.f32(*radius);
                w.f32(*depth);
            }
            Msg::Push { impulse, pin, stun } => {
                w.u8(13);
                w.vec3(*impulse);
                w.f32(*pin);
                w.f32(*stun);
            }
            Msg::RiftGate { owner, pair } => {
                w.u8(14);
                w.u8(*owner);
                w.vec3(pair.a.pos);
                w.f32(pair.a.yaw);
                w.vec3(pair.b.pos);
                w.f32(pair.b.yaw);
            }
        }
        w.0
    }

    /// Reads a packet; `None` for anything malformed or not ours.
    pub fn decode(bytes: &[u8]) -> Option<Msg> {
        let mut r = Reader(bytes);
        if r.u8()? != MAGIC || r.u8()? != VERSION {
            return None;
        }
        let msg = match r.u8()? {
            0 => Msg::Hello(r.text()?),
            1 => Msg::Welcome,
            2 => {
                let pos = r.vec3()?;
                let yaw = r.f32()?;
                let flags = r.u8()?;
                let flight = r.vec3()?;
                let health = r.u8()?;
                Msg::State(PlayerState {
                    pos,
                    yaw,
                    grounded: flags & 1 != 0,
                    flight: (flags & 2 != 0).then_some(flight),
                    health,
                })
            }
            3 => Msg::Fire(FireMsg {
                weapon: r.u8()?,
                origin: r.vec3()?,
                dir: r.vec3()?,
                damage: r.f32()?,
                pellets: r.u8()?,
                blast_radius: r.f32()?,
                mods: r.u8()?,
                homing: r.u8()? != 0,
            }),
            4 => {
                let n = r.u8()? as usize;
                let mut list = Vec::with_capacity(n);
                for _ in 0..n {
                    list.push((r.vec3()?, r.u8()? != 0));
                }
                Msg::Drones(list)
            }
            5 => Msg::Kills(r.u32()?),
            6 => Msg::Bye,
            7 => Msg::Reject(r.text()?),
            8 => {
                let round = r.u32()?;
                let phase = crate::arena::Phase::from_code(r.u8()?)?;
                let time_left = r.f32()?;
                let scores = [r.u32()?, r.u32()?];
                let winner = r.u8()?;
                let spawn = r.vec3()?;
                let state = crate::arena::MatchState { round, phase, time_left, scores, winner: (winner != 255).then_some(winner) };
                Msg::Round(state, spawn)
            }
            9 => Msg::Died,
            10 => Msg::ResetAll,
            11 => Msg::Build(crate::structures::Structure {
                id: r.u32()?,
                kind: crate::structures::Kind::from_code(r.u8()?)?,
                pos: r.vec3()?,
                yaw: r.f32()?,
                health: r.f32()?,
                max_health: r.f32()?,
                life: r.f32()?,
                warden: r.u8()? != 0,
                scale: r.f32()?,
                power: r.f32()?,
            }),
            12 => Msg::Unbuild(r.u32()?),
            15 => Msg::Crater { pos: r.vec3()?, radius: r.f32()?, depth: r.f32()? },
            13 => Msg::Push { impulse: r.vec3()?, pin: r.f32()?, stun: r.f32()? },
            14 => Msg::RiftGate {
                owner: r.u8()?,
                pair: GatePair {
                    a: GatePoint { pos: r.vec3()?, yaw: r.f32()? },
                    b: GatePoint { pos: r.vec3()?, yaw: r.f32()? },
                },
            },
            _ => return None,
        };
        Some(msg)
    }
}

// ---------------------------------------------------------------- connection

/// Who a packet comes from or goes to: a computer on the network, or a
/// Steam user (through Steam's relays, see `steam.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Peer {
    Ip(SocketAddr),
    Steam(u64),
}

/// How packets travel: a UDP socket, or Steam.
enum Link {
    Udp(UdpSocket),
    Steam(crate::steam::SteamLink),
}

impl Link {
    fn send(&self, bytes: &[u8], to: Peer) {
        match (self, to) {
            (Link::Udp(socket), Peer::Ip(addr)) => {
                let _ = socket.send_to(bytes, addr);
            }
            (Link::Steam(steam), Peer::Steam(id)) => steam.send(bytes, id),
            _ => {}
        }
    }

    /// Everything that has arrived.
    fn receive(&self) -> Vec<(Vec<u8>, Peer)> {
        match self {
            Link::Udp(socket) => {
                let mut out = Vec::new();
                let mut buf = [0u8; 2048];
                // Windows reports "connection reset" when the other side
                // isn't listening yet; any error just ends this batch.
                while let Ok((len, from)) = socket.recv_from(&mut buf) {
                    out.push((buf[..len].to_vec(), Peer::Ip(from)));
                }
                out
            }
            Link::Steam(steam) => steam.receive().into_iter().map(|(b, id)| (b, Peer::Steam(id))).collect(),
        }
    }
}

pub struct Net {
    link: Link,
    pub role: Role,
    /// The other player (for a joiner, once the host answers).
    peer: Option<Peer>,
    /// Joiner: everywhere the host might be (from a join code: its
    /// internet and local addresses). The first to answer wins.
    candidates: Vec<Peer>,
    started: Instant,
    connected: bool,
    /// The host turned us away: its game version.
    pub rejected: Option<String>,
    last_heard: Instant,
    hello_timer: f32,
    state_timer: f32,
    drone_timer: f32,
    port_mapping: Arc<Mutex<PortMappingState>>,
}

enum PortMappingState {
    Disabled,
    Searching,
    Mapped { gateway: Gateway, port: u16, address: String },
    /// Why players from the internet can't get in automatically.
    Failed(String),
    Closed,
}

struct PortMapping {
    gateway: Gateway,
    port: u16,
    address: String,
}

impl Net {
    /// Listens for a joiner on `port` (all network interfaces), and asks
    /// the router in the background to let players from the internet in.
    pub fn host(port: u16) -> std::io::Result<Self> {
        let socket = UdpSocket::bind(("0.0.0.0", port))?;
        socket.set_nonblocking(true)?;
        let port = socket.local_addr()?.port();
        Ok(Self::with(Link::Udp(socket), Role::Host, Vec::new(), Some(port)))
    }

    /// Hosts through Steam: friends join from their friends list.
    pub fn host_steam(steam: &crate::steam::Steam) -> Self {
        steam.advertise(true);
        Self::with(Link::Steam(steam.link()), Role::Host, Vec::new(), None)
    }

    /// Joins a Steam friend's game.
    pub fn join_steam(steam: &crate::steam::Steam, host: u64) -> Self {
        Self::with(Link::Steam(steam.link()), Role::Client, vec![Peer::Steam(host)], None)
    }

    pub fn over_steam(&self) -> bool {
        matches!(self.link, Link::Steam(_))
    }

    /// The other player's Steam ID, when playing through Steam.
    pub fn peer_steam_id(&self) -> Option<u64> {
        match self.peer {
            Some(Peer::Steam(id)) => Some(id),
            _ => None,
        }
    }

    /// Host: the code to give the other player (holds the internet and
    /// local addresses and the port).
    pub fn join_code(&self) -> Option<String> {
        let lan = lan_address().and_then(|a| a.parse().ok());
        let public = self.public_address().and_then(|a| match a.parse::<SocketAddr>() {
            Ok(SocketAddr::V4(v4)) => Some(*v4.ip()),
            _ => None,
        });
        (lan.is_some() || public.is_some()).then(|| encode_code(public, lan, self.port()))
    }

    /// Joins a host from a join code, or an `address` ("ip:port", or
    /// "ip" for the default port).
    pub fn join(address: &str) -> std::io::Result<Self> {
        let address = address.trim();
        if let Some((public, lan, port)) = decode_code(address) {
            let candidates: Vec<Peer> = [public, lan]
                .into_iter()
                .flatten()
                .map(|ip| Peer::Ip(SocketAddr::V4(SocketAddrV4::new(ip, port))))
                .collect();
            let socket = UdpSocket::bind(("0.0.0.0", 0))?;
            socket.set_nonblocking(true)?;
            return Ok(Self::with(Link::Udp(socket), Role::Client, candidates, None));
        }
        let full = if address.contains(':') { address.to_owned() } else { format!("{address}:{DEFAULT_PORT}") };
        let peer = full
            .to_socket_addrs()?
            .find(|a| a.is_ipv4())
            .ok_or_else(|| std::io::Error::new(ErrorKind::NotFound, "no IPv4 address for that host"))?;
        let socket = UdpSocket::bind(("0.0.0.0", 0))?;
        socket.set_nonblocking(true)?;
        Ok(Self::with(Link::Udp(socket), Role::Client, vec![Peer::Ip(peer)], None))
    }

    /// `upnp_port`: a UDP host's port, to ask the router to open.
    fn with(link: Link, role: Role, candidates: Vec<Peer>, upnp_port: Option<u16>) -> Self {
        let port_mapping = Arc::new(Mutex::new(if upnp_port.is_some() {
            PortMappingState::Searching
        } else {
            PortMappingState::Disabled
        }));
        let net = Self {
            link,
            role,
            peer: None,
            candidates,
            started: Instant::now(),
            connected: false,
            rejected: None,
            last_heard: Instant::now(),
            hello_timer: 0.0,
            state_timer: 0.0,
            drone_timer: 0.0,
            port_mapping: port_mapping.clone(),
        };
        if let Some(port) = upnp_port {
            std::thread::spawn(move || {
                let result = map_udp_port(port);
                let mut state = port_mapping.lock().unwrap_or_else(|e| e.into_inner());
                match result {
                    Ok(mapping) => match &*state {
                        PortMappingState::Closed => {
                            let _ = mapping.gateway.remove_port(PortMappingProtocol::UDP, mapping.port);
                        }
                        _ => {
                            log::info!("UPnP: the router forwards UDP {} (internet address {})", mapping.port, mapping.address);
                            *state = PortMappingState::Mapped {
                                gateway: mapping.gateway,
                                port: mapping.port,
                                address: mapping.address,
                            };
                        }
                    },
                    Err(e) => {
                        log::info!("automatic UDP port forwarding unavailable: {e}");
                        if !matches!(&*state, PortMappingState::Closed) {
                            *state = PortMappingState::Failed(e);
                        }
                    }
                }
            });
        }
        net
    }

    /// The UDP port (0 through Steam).
    pub fn port(&self) -> u16 {
        match &self.link {
            Link::Udp(socket) => socket.local_addr().map_or(0, |a| a.port()),
            Link::Steam(_) => 0,
        }
    }

    pub fn connected(&self) -> bool {
        self.connected
    }

    pub fn is_host(&self) -> bool {
        self.role == Role::Host
    }

    /// Public address to share when UPnP successfully forwarded this port.
    pub fn public_address(&self) -> Option<String> {
        let state = self.port_mapping.lock().unwrap_or_else(|e| e.into_inner());
        match &*state {
            PortMappingState::Mapped { address, .. } => Some(address.clone()),
            _ => None,
        }
    }

    /// Human-readable state for automatic router configuration, and whether
    /// players from the internet can join (None while still checking).
    pub fn port_forwarding_status(&self) -> (String, Option<bool>) {
        let state = self.port_mapping.lock().unwrap_or_else(|e| e.into_inner());
        match &*state {
            PortMappingState::Searching => ("Asking your router to let players from the internet in...".into(), None),
            PortMappingState::Mapped { .. } => ("Your router let the game in: friends anywhere can join with the code.".into(), Some(true)),
            PortMappingState::Failed(why) => (
                format!("Only players on your network can join ({why}). See \"Playing over the internet\" below."),
                Some(false),
            ),
            PortMappingState::Disabled | PortMappingState::Closed => (String::new(), None),
        }
    }

    /// A line for the menu.
    pub fn status(&self) -> String {
        let over_steam = self.over_steam();
        match (self.role, self.connected, self.peer) {
            (Role::Host, true, Some(Peer::Ip(p))) => format!("Hosting on port {}: {} joined", self.port(), p.ip()),
            (Role::Host, true, _) => "Hosting through Steam: your friend joined".into(),
            (Role::Host, _, _) if over_steam => {
                "Hosting through Steam: friends can right-click you in Steam > Join Game, or use Invite".into()
            }
            (Role::Host, _, _) => format!("Hosting on port {}: waiting for a player to join", self.port()),
            (Role::Client, true, Some(Peer::Ip(p))) => format!("Joined the game at {}", p.ip()),
            (Role::Client, true, _) => "Joined your friend's game through Steam".into(),
            (Role::Client, false, _) if over_steam && self.rejected.is_none() && self.started.elapsed() > Duration::from_secs(10) => {
                "Can't reach your friend through Steam yet: are they still hosting?".into()
            }
            (Role::Client, false, _) if self.rejected.is_some() => format!(
                "The host runs version {} and you have {GAME_VERSION}: both players need the same version",
                self.rejected.as_deref().unwrap_or("?")
            ),
            (Role::Client, _, _) if self.started.elapsed() > Duration::from_secs(8) => {
                "Can't reach the host. Check the code; the host's router may not let players in (see the help below).".into()
            }
            (Role::Client, _, _) => "Looking for the host...".into(),
        }
    }

    pub fn send(&self, msg: &Msg) {
        if let Some(peer) = self.peer {
            // Losing a packet is fine: state is resent constantly.
            self.link.send(&msg.encode(), peer);
        }
    }

    /// Joiner, before connecting: knocks everywhere the host might be.
    fn send_to_candidates(&self, msg: &Msg) {
        let bytes = msg.encode();
        for &to in &self.candidates {
            self.link.send(&bytes, to);
        }
    }

    /// True when it's time to send this player's state again.
    pub fn state_due(&mut self, dt: f32) -> bool {
        self.state_timer -= dt;
        if self.state_timer <= 0.0 && self.connected {
            self.state_timer = STATE_INTERVAL;
            return true;
        }
        false
    }

    /// True when the host should send drone positions again.
    pub fn drones_due(&mut self, dt: f32) -> bool {
        self.drone_timer -= dt;
        if self.drone_timer <= 0.0 && self.connected && self.is_host() {
            self.drone_timer = DRONE_INTERVAL;
            return true;
        }
        false
    }

    /// Handles the handshake and timeouts and returns the messages that
    /// arrived from the other player.
    pub fn poll(&mut self, dt: f32) -> Vec<Msg> {
        if self.role == Role::Client && !self.connected && self.rejected.is_none() {
            self.hello_timer -= dt;
            if self.hello_timer <= 0.0 {
                self.hello_timer = 0.5;
                self.send_to_candidates(&Msg::Hello(GAME_VERSION.to_owned()));
            }
        }
        let mut out = Vec::new();
        for (bytes, from) in self.link.receive() {
            let Some(msg) = Msg::decode(&bytes) else { continue };
            match self.role {
                Role::Host => {
                    let from_peer = self.peer == Some(from);
                    if let Msg::Hello(version) = &msg
                        && version != GAME_VERSION
                    {
                        // Different builds can't play together.
                        self.link.send(&Msg::Reject(GAME_VERSION.to_owned()).encode(), from);
                        continue;
                    }
                    if matches!(msg, Msg::Hello(_)) && (!self.connected || from_peer) {
                        self.peer = Some(from);
                        self.connected = true;
                        self.last_heard = Instant::now();
                        self.send(&Msg::Welcome);
                        continue;
                    }
                    if !from_peer {
                        continue; // a second joiner, or noise
                    }
                }
                Role::Client => {
                    // Before connecting, any of the host's addresses may
                    // answer; after, only the one that did.
                    let known = match self.peer {
                        Some(p) => p == from,
                        None => self.candidates.contains(&from),
                    };
                    if !known {
                        continue;
                    }
                    if msg == Msg::Welcome {
                        self.peer = Some(from);
                        self.connected = true;
                    }
                    if let Msg::Reject(version) = &msg {
                        self.rejected = Some(version.clone());
                        continue;
                    }
                }
            }
            self.last_heard = Instant::now();
            if msg == Msg::Bye {
                self.drop_peer();
                continue;
            }
            if self.connected && !matches!(msg, Msg::Hello(_) | Msg::Welcome) {
                out.push(msg);
            }
        }
        if self.connected && self.last_heard.elapsed() > TIMEOUT {
            self.drop_peer();
        }
        out
    }

    fn drop_peer(&mut self) {
        self.connected = false;
        if self.role == Role::Host {
            self.peer = None;
        }
    }
}

impl Drop for Net {
    fn drop(&mut self) {
        if self.connected {
            self.send(&Msg::Bye);
        }
        let mut state = self.port_mapping.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::mem::replace(&mut *state, PortMappingState::Closed);
        if let PortMappingState::Mapped { gateway, port, .. } = previous {
            if let Err(e) = gateway.remove_port(PortMappingProtocol::UDP, port) {
                log::debug!("couldn't remove UPnP port mapping: {e}");
            }
        }
    }
}

fn map_udp_port(port: u16) -> Result<PortMapping, String> {
    let mut options = SearchOptions::default();
    options.timeout = Some(Duration::from_secs(4));
    options.single_search_timeout = Some(Duration::from_secs(2));
    let gateway = search_gateway(options).map_err(|_| "your router has automatic setup (UPnP) turned off".to_string())?;

    // Route a harmless UDP probe toward the router to learn the LAN interface
    // address that must receive the forwarded traffic.
    let probe = UdpSocket::bind(("0.0.0.0", 0)).map_err(|e| e.to_string())?;
    probe.connect(gateway.addr).map_err(|e| e.to_string())?;
    let mut local_addr = probe.local_addr().map_err(|e| e.to_string())?;
    local_addr.set_port(port);

    gateway
        .add_port(PortMappingProtocol::UDP, port, local_addr, 0, "OzLand multiplayer")
        .or_else(|_| {
            // Some routers only accept mappings that expire.
            gateway.add_port(PortMappingProtocol::UDP, port, local_addr, 4 * 3600, "OzLand multiplayer")
        })
        .map_err(|e| format!("your router refused to open the port: {e}"))?;
    let external_ip = match gateway.get_external_ip() {
        Ok(ip) => ip,
        Err(e) => {
            let _ = gateway.remove_port(PortMappingProtocol::UDP, port);
            return Err(e.to_string());
        }
    };
    // A router behind another router, or the internet provider's shared
    // network (CGNAT), has no address the internet can reach.
    if let IpAddr::V4(ip) = external_ip
        && is_private(ip)
    {
        let _ = gateway.remove_port(PortMappingProtocol::UDP, port);
        return Err("your internet provider shares one address between customers (CGNAT)".into());
    }
    let address = match external_ip {
        IpAddr::V4(ip) => format!("{ip}:{port}"),
        IpAddr::V6(ip) => format!("[{ip}]:{port}"),
    };
    Ok(PortMapping { gateway, port, address })
}

/// This computer's address on the local network, to tell a friend. Found
/// by asking the OS which interface would route outward; no packet is sent.
pub fn lan_address() -> Option<String> {
    let probe = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    probe.connect(("192.168.0.1", 9)).ok()?;
    let ip = probe.local_addr().ok()?.ip();
    (!ip.is_unspecified()).then(|| ip.to_string())
}

/// Addresses that can't be reached from the internet (including a
/// provider's shared CGNAT range, 100.64.0.0/10).
fn is_private(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    ip.is_private() || ip.is_loopback() || ip.is_unspecified() || ip.is_link_local() || (a == 100 && (64..128).contains(&b))
}

// ---------------------------------------------------------------- join codes

/// Crockford base32: no I, L, O or U, so codes are easy to read out.
const CODE_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// A 16-character code like `K7M2-QX4P-AB9C-D3FH` holding the host's
/// internet address, local address (either may be missing) and port.
pub fn encode_code(public: Option<Ipv4Addr>, lan: Option<Ipv4Addr>, port: u16) -> String {
    let mut bytes = [0u8; 10];
    bytes[..4].copy_from_slice(&public.unwrap_or(Ipv4Addr::UNSPECIFIED).octets());
    bytes[4..8].copy_from_slice(&lan.unwrap_or(Ipv4Addr::UNSPECIFIED).octets());
    bytes[8..].copy_from_slice(&port.to_be_bytes());
    let bits = bytes.iter().fold(0u128, |acc, b| acc << 8 | *b as u128);
    let chars: Vec<char> = (0..16).rev().map(|i| CODE_ALPHABET[((bits >> (i * 5)) & 31) as usize] as char).collect();
    chars.chunks(4).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join("-")
}

/// Reads a join code (any case, with or without dashes or spaces).
pub fn decode_code(code: &str) -> Option<(Option<Ipv4Addr>, Option<Ipv4Addr>, u16)> {
    let clean: Vec<u8> = code
        .chars()
        .filter(|c| !matches!(c, '-' | ' '))
        .map(|c| match c.to_ascii_uppercase() {
            'O' => b'0',
            'I' | 'L' => b'1',
            c => c as u8,
        })
        .collect();
    if clean.len() != 16 {
        return None;
    }
    let mut bits = 0u128;
    for c in clean {
        let v = CODE_ALPHABET.iter().position(|a| *a == c)? as u128;
        bits = bits << 5 | v;
    }
    let bytes: Vec<u8> = (0..10).rev().map(|i| (bits >> (i * 8)) as u8).collect();
    let ip = |s: &[u8]| {
        let ip = Ipv4Addr::new(s[0], s[1], s[2], s[3]);
        (!ip.is_unspecified()).then_some(ip)
    };
    let port = u16::from_be_bytes([bytes[8], bytes[9]]);
    let (public, lan) = (ip(&bytes[..4]), ip(&bytes[4..8]));
    (port != 0 && (public.is_some() || lan.is_some())).then_some((public, lan, port))
}

#[cfg(test)]mod tests {
    use super::*;

    #[test]
    fn messages_round_trip() {
        let msgs = [
            Msg::Hello(GAME_VERSION.into()),
            Msg::Reject("9.9.9".into()),
            Msg::Welcome,
            Msg::State(PlayerState { pos: Vec3::new(1.0, 2.0, 3.0), yaw: 0.5, grounded: true, flight: None, health: 100 }),
            Msg::State(PlayerState { pos: Vec3::ZERO, yaw: -1.0, grounded: false, flight: Some(Vec3::Y), health: 0 }),
            Msg::Round(
                crate::arena::MatchState { round: 3, phase: crate::arena::Phase::Fight, time_left: 0.0, scores: [2, 1], winner: Some(1) },
                Vec3::new(5.0, 6.0, 7.0),
            ),
            Msg::Died,
            Msg::ResetAll,
            Msg::Build(crate::structures::Structure {
                id: 0x0100_0007,
                kind: crate::structures::Kind::Dome,
                pos: Vec3::new(1.0, 2.0, 3.0),
                yaw: 0.5,
                health: 80.0,
                max_health: 100.0,
                life: 9.5,
                warden: true,
                scale: 1.4,
                power: 2.0,
            }),
            Msg::Unbuild(0x0200_0003),
            Msg::Push { impulse: Vec3::new(1.0, 5.0, -2.0), pin: 2.0, stun: 0.5 },
            Msg::Fire(FireMsg { weapon: 2, origin: Vec3::ONE, dir: Vec3::X, damage: 1.5, pellets: 8, blast_radius: 4.5, mods: 5, homing: true }),
            Msg::Drones(vec![(Vec3::ONE, true), (Vec3::NEG_ONE, false)]),
            Msg::Kills(3),
            Msg::Bye,
        ];
        for m in msgs {
            assert_eq!(Msg::decode(&m.encode()), Some(m.clone()), "{m:?}");
        }
        assert_eq!(Msg::decode(b"hello"), None);
        assert_eq!(Msg::decode(&[MAGIC, VERSION, 2, 0]), None, "truncated");
    }

    #[test]
    fn join_codes_round_trip_and_forgive_typing() {
        let (public, lan) = (Ipv4Addr::new(81, 2, 69, 160), Ipv4Addr::new(192, 168, 1, 20));
        let code = encode_code(Some(public), Some(lan), 7777);
        assert_eq!(code.len(), 19, "{code}");
        assert_eq!(decode_code(&code), Some((Some(public), Some(lan), 7777)));
        let sloppy = code.to_lowercase().replace('-', " ").replace('0', "o");
        assert_eq!(decode_code(&sloppy), Some((Some(public), Some(lan), 7777)));
        assert_eq!(decode_code(&encode_code(None, Some(lan), 7778)), Some((None, Some(lan), 7778)));
        assert_eq!(decode_code("192.168.1.20:7777"), None, "addresses aren't codes");
        assert!(is_private(Ipv4Addr::new(100, 72, 1, 1)) && !is_private(public));
    }

    #[test]
    fn a_join_code_with_a_dead_address_still_connects() {
        // The code's "internet" address goes nowhere; its local one works.
        let mut host = Net::host(0).expect("bind");
        let code = encode_code(Some(Ipv4Addr::new(127, 0, 0, 2)), Some(Ipv4Addr::LOCALHOST), host.port());
        let mut joiner = Net::join(&code).expect("join");
        for _ in 0..200 {
            joiner.poll(0.5);
            host.poll(0.016);
            if joiner.connected() && host.connected() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(joiner.connected() && host.connected());
    }

    #[test]
    fn host_and_joiner_connect_and_exchange_state() {
        let mut host = Net::host(0).expect("bind");
        let mut joiner = Net::join(&format!("127.0.0.1:{}", host.port())).expect("join");
        let state = PlayerState { pos: Vec3::new(4.0, 5.0, 6.0), yaw: 1.0, grounded: true, flight: None, health: 100 };
        let mut got = None;
        for _ in 0..200 {
            joiner.poll(0.5);
            for m in host.poll(0.016) {
                if let Msg::State(s) = m {
                    got = Some(s);
                }
            }
            if joiner.connected() {
                joiner.send(&Msg::State(state));
            }
            if got.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(host.connected() && joiner.connected());
        assert_eq!(got, Some(state));
    }

    /// Run by hand: `cargo test upnp_probe -- --ignored --nocapture`.
    /// Reports whether this network's router accepts automatic setup.
    #[test]
    #[ignore]
    fn upnp_probe() {
        let t = Instant::now();
        let mut options = SearchOptions::default();
        options.timeout = Some(Duration::from_secs(4));
        options.single_search_timeout = Some(Duration::from_secs(2));
        let found = search_gateway(options);
        eprintln!("PROBE search: {:?} after {:.1?}", found.as_ref().map(|g| g.addr), t.elapsed());
        let t = Instant::now();
        let result = map_udp_port(47777);
        eprintln!("PROBE map: {:?} after {:.1?}", result.as_ref().map(|m| m.address.clone()), t.elapsed());
        if let Ok(m) = result {
            let _ = m.gateway.remove_port(PortMappingProtocol::UDP, m.port);
        }
    }
}
