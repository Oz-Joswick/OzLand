//! The multiplayer fight: rounds of 15 s exploring (each player dropped at
//! a random spot, no damage), then a fight until one player falls. The
//! survivor scores, and after a short pause the next round starts.
//!
//! The host runs the match and sends its state to the joiner, along with
//! the joiner's spawn point for each new round. Damage is decided by the
//! player who gets hit (their machine replays every shot against their
//! own body), and a joiner that falls tells the host.

use glam::Vec3;

pub const EXPLORE_TIME: f32 = 15.0;
/// Pause after a round, showing who won.
pub const OVER_TIME: f32 = 4.0;
pub const MAX_HEALTH: f32 = 100.0;
/// Player health lost per unit of weapon damage (a drone has 6 health, a
/// player 100: a blaster bolt takes 6, a railgun beam 48, more with
/// upgrades).
pub const PVP_SCALE: f32 = 6.0;
/// Seconds between match-state packets from the host.
const SYNC_INTERVAL: f32 = 0.25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// No opponent yet.
    Waiting,
    /// Look around the map from a random spawn; no damage.
    Explore,
    /// Until one player falls.
    Fight,
    /// Showing the round's winner.
    Over,
}

impl Phase {
    pub fn code(self) -> u8 {
        match self {
            Phase::Waiting => 0,
            Phase::Explore => 1,
            Phase::Fight => 2,
            Phase::Over => 3,
        }
    }

    pub fn from_code(code: u8) -> Option<Phase> {
        [Phase::Waiting, Phase::Explore, Phase::Fight, Phase::Over].get(code as usize).copied()
    }
}

/// What both players see: the round, its phase and the score.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MatchState {
    pub round: u32,
    pub phase: Phase,
    /// Seconds left in Explore or Over.
    pub time_left: f32,
    /// Rounds won: [host, joiner].
    pub scores: [u32; 2],
    /// Who won the last round (player index).
    pub winner: Option<u8>,
}

impl Default for MatchState {
    fn default() -> Self {
        Self { round: 0, phase: Phase::Waiting, time_left: 0.0, scores: [0, 0], winner: None }
    }
}

pub struct Arena {
    pub state: MatchState,
    sync_timer: f32,
}

/// What the game should do after an update.
#[derive(Debug, PartialEq)]
pub enum Event {
    None,
    /// A round begins: respawn at a random spot with full health.
    NewRound,
}

impl Arena {
    pub fn new() -> Self {
        Self { state: MatchState::default(), sync_timer: 0.0 }
    }

    pub fn in_match(&self) -> bool {
        self.state.phase != Phase::Waiting
    }

    /// Damage only counts while fighting.
    pub fn fighting(&self) -> bool {
        self.state.phase == Phase::Fight
    }

    /// Host: advances the match.
    pub fn host_update(&mut self, dt: f32, connected: bool) -> Event {
        let s = &mut self.state;
        if !connected {
            s.phase = Phase::Waiting;
            return Event::None;
        }
        match s.phase {
            Phase::Waiting => return self.start_round(),
            Phase::Explore => {
                s.time_left -= dt;
                if s.time_left <= 0.0 {
                    s.phase = Phase::Fight;
                    s.time_left = 0.0;
                }
            }
            Phase::Fight => {}
            Phase::Over => {
                s.time_left -= dt;
                if s.time_left <= 0.0 {
                    return self.start_round();
                }
            }
        }
        Event::None
    }

    fn start_round(&mut self) -> Event {
        let s = &mut self.state;
        s.round += 1;
        s.phase = Phase::Explore;
        s.time_left = EXPLORE_TIME;
        s.winner = None;
        self.sync_timer = 0.0; // tell the joiner right away
        Event::NewRound
    }

    /// Host: player `fallen` (0 host, 1 joiner) is out; the other scores.
    pub fn report_death(&mut self, fallen: usize) {
        let s = &mut self.state;
        if s.phase != Phase::Fight {
            return;
        }
        let winner = 1 - fallen.min(1);
        s.scores[winner] += 1;
        s.winner = Some(winner as u8);
        s.phase = Phase::Over;
        s.time_left = OVER_TIME;
        self.sync_timer = 0.0;
    }

    /// Host: scores back to zero and a fresh round.
    pub fn reset(&mut self) -> Event {
        self.state.scores = [0, 0];
        if self.state.phase == Phase::Waiting {
            return Event::None;
        }
        self.start_round()
    }

    /// Host: true when it's time to send the match state.
    pub fn sync_due(&mut self, dt: f32) -> bool {
        self.sync_timer -= dt;
        if self.sync_timer <= 0.0 {
            self.sync_timer = SYNC_INTERVAL;
            return true;
        }
        false
    }

    /// Joiner: takes the host's match state.
    pub fn apply(&mut self, state: MatchState) -> Event {
        let new_round = state.round != self.state.round && state.phase == Phase::Explore;
        self.state = state;
        if new_round { Event::NewRound } else { Event::None }
    }

    /// Joiner: the host is gone.
    pub fn disconnect(&mut self) {
        self.state.phase = Phase::Waiting;
    }

    /// Joiner: counts down between the host's updates.
    pub fn tick(&mut self, dt: f32) {
        if matches!(self.state.phase, Phase::Explore | Phase::Over) {
            self.state.time_left = (self.state.time_left - dt).max(0.0);
        }
    }
}

/// Distance from `p` to a standing player's body (a capsule from just above
/// their feet to their head). Negative inside.
pub fn body_distance(p: Vec3, feet: Vec3) -> f32 {
    let (low, high) = (feet.y + 0.45, feet.y + 1.45);
    let axis = Vec3::new(feet.x, p.y.clamp(low, high), feet.z);
    p.distance(axis) - 0.45
}

/// How far outside a body a shot still counts as a hit: generous, so
/// players flying, gliding and dashing around can still be tagged.
pub const SHOT_SLACK: f32 = 0.65;

/// Where a segment first touches a player's body (with [`SHOT_SLACK`]),
/// as a distance along it.
pub fn segment_hits_body(from: Vec3, dir: Vec3, len: f32, feet: Vec3) -> Option<f32> {
    let steps = (len / 0.2).ceil().max(1.0) as usize;
    (0..=steps)
        .map(|k| len * k as f32 / steps as f32)
        .find(|&t| body_distance(from + dir * t, feet) <= SHOT_SLACK)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_round_explores_fights_and_scores() {
        let mut a = Arena::new();
        assert_eq!(a.host_update(0.1, false), Event::None, "nobody to fight");
        assert_eq!(a.host_update(0.1, true), Event::NewRound);
        assert_eq!(a.state.phase, Phase::Explore);
        a.report_death(1);
        assert_eq!(a.state.scores, [0, 0], "no damage while exploring");
        for _ in 0..16 {
            a.host_update(1.0, true);
        }
        assert!(a.fighting());
        a.report_death(1); // the joiner falls: host scores
        assert_eq!((a.state.scores, a.state.phase), ([1, 0], Phase::Over));
        a.report_death(0);
        assert_eq!(a.state.scores, [1, 0], "only one death per round");
        for _ in 0..3 {
            assert_eq!(a.host_update(1.0, true), Event::None);
        }
        assert_eq!(a.host_update(1.5, true), Event::NewRound);
        assert_eq!(a.state.round, 2);

        // The joiner follows the host's state and notices the new round.
        let mut joiner = Arena::new();
        assert_eq!(joiner.apply(a.state), Event::NewRound);
        assert_eq!(joiner.apply(a.state), Event::None);
    }

    #[test]
    fn shots_hit_bodies() {
        let feet = Vec3::new(10.0, 0.0, 0.0);
        assert!(segment_hits_body(Vec3::new(0.0, 1.0, 0.0), Vec3::X, 20.0, feet).is_some());
        assert!(segment_hits_body(Vec3::new(0.0, 3.0, 0.0), Vec3::X, 20.0, feet).is_none(), "over their head");
        assert!(segment_hits_body(Vec3::new(0.0, 1.0, 0.0), Vec3::X, 5.0, feet).is_none(), "out of reach");
    }
}
