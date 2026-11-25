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
use std::net::{IpAddr, SocketAddr, ToSocketAddrs, UdpSocket};
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
            _ => return None,
        };
        Some(msg)
    }
}

// ---------------------------------------------------------------- connection

pub struct Net {
    socket: UdpSocket,
    pub role: Role,
    /// The other player's address (for a joiner, the host from the start).
    peer: Option<SocketAddr>,
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
    Failed,
    Closed,
}

struct PortMapping {
    gateway: Gateway,
    port: u16,
    address: String,
}

impl Net {
    /// Listens for a joiner on `port` (all network interfaces).
    pub fn host(port: u16) -> std::io::Result<Self> {
        let socket = UdpSocket::bind(("0.0.0.0", port))?;
        Self::with(socket, Role::Host, None)
    }

    /// Joins a host at `address` ("ip:port", or "ip" for the default port).
    pub fn join(address: &str) -> std::io::Result<Self> {
        let address = address.trim();
        let full = if address.contains(':') { address.to_owned() } else { format!("{address}:{DEFAULT_PORT}") };
        let peer = full
            .to_socket_addrs()?
            .find(|a| a.is_ipv4())
            .ok_or_else(|| std::io::Error::new(ErrorKind::NotFound, "no IPv4 address for that host"))?;
        let socket = UdpSocket::bind(("0.0.0.0", 0))?;
        Self::with(socket, Role::Client, Some(peer))
    }

    fn with(socket: UdpSocket, role: Role, peer: Option<SocketAddr>) -> std::io::Result<Self> {
        socket.set_nonblocking(true)?;
        let port = socket.local_addr()?.port();
        let port_mapping = Arc::new(Mutex::new(if role == Role::Host {
            PortMappingState::Searching
        } else {
            PortMappingState::Disabled
        }));
        let net = Self {
            socket,
            role,
            peer,
            connected: false,
            rejected: None,
            last_heard: Instant::now(),
            hello_timer: 0.0,
            state_timer: 0.0,
            drone_timer: 0.0,
            port_mapping: port_mapping.clone(),
        };
        if role == Role::Host {
            std::thread::spawn(move || {
                let result = map_udp_port(port);
                let mut state = port_mapping.lock().unwrap_or_else(|e| e.into_inner());
                match result {
                    Ok(mapping) => match &*state {
                        PortMappingState::Closed => {
                            let _ = mapping.gateway.remove_port(PortMappingProtocol::UDP, mapping.port);
                        }
                        _ => {
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
                            *state = PortMappingState::Failed;
                        }
                    }
                }
            });
        }
        Ok(net)
    }

    pub fn port(&self) -> u16 {
        self.socket.local_addr().map_or(0, |a| a.port())
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

    /// Human-readable state for automatic router configuration.
    pub fn port_forwarding_status(&self) -> &'static str {
        let state = self.port_mapping.lock().unwrap_or_else(|e| e.into_inner());
        match &*state {
            PortMappingState::Searching => "Checking the router for automatic UDP forwarding…",
            PortMappingState::Mapped { .. } => "The router opened the UDP port automatically.",
            PortMappingState::Failed => "Automatic forwarding unavailable. Forward UDP manually or enable UPnP on the router.",
            PortMappingState::Disabled | PortMappingState::Closed => "",
        }
    }

    /// A line for the menu.
    pub fn status(&self) -> String {
        match (self.role, self.connected, self.peer) {
            (Role::Host, true, Some(p)) => format!("Hosting on port {}: {} joined", self.port(), p.ip()),
            (Role::Host, _, _) => format!("Hosting on port {}: waiting for a player to join", self.port()),
            (Role::Client, true, Some(p)) => format!("Joined the game at {p}"),
            (Role::Client, false, _) if self.rejected.is_some() => format!(
                "The host runs version {} and you have {GAME_VERSION}: both players need the same version",
                self.rejected.as_deref().unwrap_or("?")
            ),
            (Role::Client, _, Some(p)) => format!("Looking for a host at {p}..."),
            (Role::Client, _, None) => "Not connected".into(),
        }
    }

    pub fn send(&self, msg: &Msg) {
        if let Some(peer) = self.peer {
            // Losing a packet is fine: state is resent constantly.
            let _ = self.socket.send_to(&msg.encode(), peer);
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
                self.send(&Msg::Hello(GAME_VERSION.to_owned()));
            }
        }
        let mut out = Vec::new();
        let mut buf = [0u8; 2048];
        loop {
            let (len, from) = match self.socket.recv_from(&mut buf) {
                Ok(r) => r,
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                // Windows reports "connection reset" when the other side
                // isn't listening yet; just keep trying.
                Err(_) => break,
            };
            let Some(msg) = Msg::decode(&buf[..len]) else { continue };
            match self.role {
                Role::Host => {
                    let from_peer = self.peer == Some(from);
                    if let Msg::Hello(version) = &msg
                        && version != GAME_VERSION
                    {
                        // Different builds can't play together.
                        let _ = self.socket.send_to(&Msg::Reject(GAME_VERSION.to_owned()).encode(), from);
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
                    if self.peer != Some(from) {
                        continue;
                    }
                    if msg == Msg::Welcome {
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
    let gateway = search_gateway(options).map_err(|e| e.to_string())?;

    // Route a harmless UDP probe toward the router to learn the LAN interface
    // address that must receive the forwarded traffic.
    let probe = UdpSocket::bind(("0.0.0.0", 0)).map_err(|e| e.to_string())?;
    probe.connect(gateway.addr).map_err(|e| e.to_string())?;
    let mut local_addr = probe.local_addr().map_err(|e| e.to_string())?;
    local_addr.set_port(port);

    gateway
        .add_port(PortMappingProtocol::UDP, port, local_addr, 0, "OzLand multiplayer")
        .map_err(|e| e.to_string())?;
    let external_ip = match gateway.get_external_ip() {
        Ok(ip) => ip,
        Err(e) => {
            let _ = gateway.remove_port(PortMappingProtocol::UDP, port);
            return Err(e.to_string());
        }
    };
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

#[cfg(test)]
mod tests {
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
            Msg::Fire(FireMsg { weapon: 2, origin: Vec3::ONE, dir: Vec3::X, damage: 1.5, pellets: 8, blast_radius: 4.5 }),
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
}
