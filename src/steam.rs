//! Steam: playing over the internet without any router setup.
//!
//! Runs as Steam's free test app, **Spacewar (App ID 480)**, which any Steam
//! user can use, so no Steam store page is needed. Steam carries the game's
//! messages (`net.rs` decides what they say) through Valve's relay network,
//! so it works behind any router. Friends join by right-clicking the host in
//! their Steam friends list > **Join Game**, or from an invite.
//!
//! Optional: if `steam_api64.dll` isn't beside the game, or Steam isn't
//! running, [`Steam::init`] returns `None` and direct connections still work.
//! (While the game runs with Steam, friends see you playing "Spacewar".)

use std::sync::{Arc, Mutex};

use steamworks::networking_types::{NetworkingIdentity, SendFlags};
use steamworks::{CallbackHandle, Client, FriendFlags, FriendState, GameRichPresenceJoinRequested, SteamId};

/// Steam's public test app (Spacewar).
const APP_ID: u32 = 480;
/// Our messages travel on this channel.
const CHANNEL: u32 = 0;
/// Rich presence `connect` strings look like this, followed by the host's
/// Steam ID (Spacewar is shared by many developers, so it's prefixed).
const CONNECT_PREFIX: &str = "+ozland_host ";

/// A Steam friend you could invite.
#[derive(Clone, Debug, PartialEq)]
pub struct SteamFriend {
    pub id: u64,
    pub name: String,
    pub status: &'static str,
    /// They have the game (Spacewar) open, so an invite joins them straight away.
    pub in_game: bool,
    /// Signed in to Steam now (offline friends can't get an invite).
    pub online: bool,
    /// Sort order: 0 in the game, 1 online, 2 away or busy, 3 offline.
    rank: u8,
}

pub struct Steam {
    client: Client,
    /// Hosts this player asked to join from the friends list / an invite.
    join_requests: Arc<Mutex<Vec<u64>>>,
    _join_callback: CallbackHandle,
}

/// Reads a host's Steam ID from a rich presence connect string.
fn parse_connect(connect: &str) -> Option<u64> {
    connect.trim().strip_prefix(CONNECT_PREFIX.trim_end())?.trim().parse().ok()
}

impl Steam {
    /// Connects to Steam, if it's available (see the module docs).
    pub fn init() -> Option<Self> {
        let beside_exe = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|d| d.join("steam_api64.dll")))
            .is_some_and(|p| p.exists());
        if !beside_exe {
            log::info!("Steam: steam_api64.dll isn't beside the game; Steam multiplayer is off");
            return None;
        }
        let client = match Client::init_app(APP_ID) {
            Ok(c) => c,
            Err(e) => {
                log::info!("Steam: not available ({e}); is Steam running?");
                return None;
            }
        };
        // Accept message sessions from anyone; `net.rs` checks who it is.
        client.networking_messages().session_request_callback(|request| {
            request.accept();
        });
        let join_requests = Arc::new(Mutex::new(Vec::new()));
        // Launched from an invite: the connect string is on the command line.
        for arg in std::env::args().collect::<Vec<_>>().windows(2) {
            if arg[0] == CONNECT_PREFIX.trim_end()
                && let Ok(id) = arg[1].parse()
            {
                join_requests.lock().unwrap_or_else(|e| e.into_inner()).push(id);
            }
        }
        // Already running: Steam calls back when "Join Game" is clicked.
        let requests = join_requests.clone();
        let join_callback = client.register_callback(move |j: GameRichPresenceJoinRequested| {
            if let Some(id) = parse_connect(&j.connect) {
                requests.lock().unwrap_or_else(|e| e.into_inner()).push(id);
            }
        });
        log::info!("Steam: signed in as {}", client.friends().name());
        Some(Self { client, join_requests, _join_callback: join_callback })
    }

    /// Call every frame: Steam's events (join requests, message sessions).
    pub fn run_callbacks(&self) {
        self.client.run_callbacks();
    }

    pub fn my_id(&self) -> u64 {
        self.client.user().steam_id().raw()
    }

    pub fn my_name(&self) -> String {
        self.client.friends().name()
    }

    /// A Steam user's name (for "X joined").
    pub fn name_of(&self, id: u64) -> String {
        self.client.friends().get_friend(SteamId::from_raw(id)).name()
    }

    /// A host the player chose to join from Steam, if any.
    pub fn take_join_request(&self) -> Option<u64> {
        self.join_requests.lock().unwrap_or_else(|e| e.into_inner()).pop()
    }

    /// Hosting: lets friends see "Join Game" on you in their friends list.
    pub fn advertise(&self, hosting: bool) {
        let friends = self.client.friends();
        if hosting {
            let connect = format!("{CONNECT_PREFIX}{}", self.my_id());
            friends.set_rich_presence("connect", Some(&connect));
            friends.set_rich_presence("status", Some("Hosting an OzLand match"));
        } else {
            friends.set_rich_presence("connect", None);
            friends.set_rich_presence("status", None);
        }
    }

    /// Opens Steam's invite dialog. Only works when the Steam overlay is
    /// in the game, which needs Steam to have launched it; started on its
    /// own (as Spacewar) it silently does nothing, so this is only tried
    /// when [`Steam::overlay`] says it's there. Otherwise invite friends
    /// one by one with [`Steam::invite`].
    pub fn invite_friends(&self) {
        let connect = format!("{CONNECT_PREFIX}{}", self.my_id());
        self.client.friends().activate_invite_dialog_connect_string(&connect);
    }

    /// The Steam overlay is in the game (so Steam's own invite dialog works).
    pub fn overlay(&self) -> bool {
        self.client.utils().is_overlay_enabled()
    }

    /// All your Steam friends: those with the game open first, then online,
    /// then away or busy, then offline; by name within each.
    pub fn friends(&self) -> Vec<SteamFriend> {
        let mut list: Vec<SteamFriend> = self
            .client
            .friends()
            .get_friends(FriendFlags::IMMEDIATE)
            .into_iter()
            .map(|f| {
                let in_game = f.game_played().is_some_and(|g| g.game.app_id().0 == APP_ID);
                let state = f.state();
                let (status, rank) = match state {
                    _ if in_game => ("has the game open", 0),
                    FriendState::Offline | FriendState::Invisible => ("offline", 3),
                    FriendState::Away | FriendState::Snooze => ("away", 2),
                    FriendState::Busy => ("busy", 2),
                    _ => ("online", 1),
                };
                SteamFriend { id: f.id().raw(), name: f.name(), status, in_game, online: rank < 3, rank }
            })
            .collect();
        list.sort_by(|a, b| a.rank.cmp(&b.rank).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        list
    }

    /// Sends one friend an invite to your game: a Steam chat message with a
    /// Join button. Works without the overlay. If they have the game open,
    /// accepting it joins you; otherwise they need to start the game first
    /// (Steam would launch the real Spacewar instead).
    pub fn invite(&self, friend: u64) {
        let connect = format!("{CONNECT_PREFIX}{}", self.my_id());
        self.client.friends().get_friend(SteamId::from_raw(friend)).invite_user_to_game(&connect);
    }

    /// The link `net.rs` sends and receives packets through.
    pub fn link(&self) -> SteamLink {
        SteamLink { client: self.client.clone() }
    }
}

impl Drop for Steam {
    fn drop(&mut self) {
        self.advertise(false);
    }
}

/// Packets to and from Steam users (by Steam ID).
pub struct SteamLink {
    client: Client,
}

impl SteamLink {
    pub fn send(&self, bytes: &[u8], to: u64) {
        let who = NetworkingIdentity::from(SteamId::from_raw(to));
        // Unreliable, like UDP: the game resends state constantly.
        let flags = SendFlags::UNRELIABLE_NO_NAGLE | SendFlags::AUTO_RESTART_BROKEN_SESSION;
        let _ = self.client.networking_messages().send_message_to_user(who, flags, bytes, CHANNEL);
    }

    /// Everything that arrived: (bytes, sender's Steam ID).
    pub fn receive(&self) -> Vec<(Vec<u8>, u64)> {
        self.client
            .networking_messages()
            .receive_messages_on_channel(CHANNEL, 64)
            .into_iter()
            .filter_map(|m| {
                let from = m.identity_peer().steam_id()?.raw();
                Some((m.data().to_vec(), from))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_connect_strings() {
        assert_eq!(parse_connect("+ozland_host 76561198000000001"), Some(76561198000000001));
        assert_eq!(parse_connect("  +ozland_host   42 "), Some(42));
        assert_eq!(parse_connect("+connect 1.2.3.4:27015"), None, "another Spacewar game's string");
        assert_eq!(parse_connect("+ozland_host nope"), None);
    }
}
