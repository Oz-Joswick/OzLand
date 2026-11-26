//! The first-time tutorial: a few short steps, each finished by actually
//! doing it (look, walk, jump, sprint, swing, destroy a drone, open the
//! menu). Shown on a card at the top of the screen; Enter skips it. The
//! game tells it what the player did with [`Tutorial::note`].

/// Something the player did, as far as the tutorial cares.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Did {
    /// Turned the view by this many pixels of mouse movement.
    Looked(f32),
    /// Walked this many metres on the ground.
    Walked(f32),
    Jumped,
    /// Sprinted (while moving) for this many seconds.
    Sprinted(f32),
    Swung,
    Destroyed,
    OpenedMenu,
}

struct Step {
    title: &'static str,
    hint: &'static str,
    keys: &'static [&'static str],
    /// How much of it makes the step done.
    goal: f32,
}

const STEPS: [Step; 7] = [
    Step { title: "Look around", hint: "Move the mouse to look.", keys: &["Mouse"], goal: 900.0 },
    Step { title: "Walk", hint: "Walk around the village square.", keys: &["W", "A", "S", "D"], goal: 10.0 },
    Step { title: "Jump", hint: "Jump a couple of times. Climbing onto things comes later.", keys: &["Space"], goal: 2.0 },
    Step { title: "Sprint", hint: "Hold Shift while moving to run.", keys: &["Shift"], goal: 2.0 },
    Step { title: "Swing your Blade", hint: "Click to swing. Weapons 1-7 unlock as you buy them.", keys: &["Left click"], goal: 3.0 },
    Step {
        title: "Destroy a drone",
        hint: "The red Hoverers near the village are the gentlest. Get close and swing. They get tougher the longer you play.",
        keys: &["Left click"],
        goal: 1.0,
    },
    Step { title: "Open the menu", hint: "Spend skill points, buy items and change settings here.", keys: &["Esc"], goal: 1.0 },
];

/// Seconds the "you're ready" card stays after the last step.
const DONE_TIME: f32 = 6.0;

pub struct Tutorial {
    step: usize,
    progress: f32,
    /// Seconds since the steps were all done.
    done: Option<f32>,
}

/// What the HUD shows of the tutorial.
pub struct TutorialView {
    pub title: &'static str,
    pub hint: &'static str,
    pub keys: &'static [&'static str],
    /// Step number (from 1) and how many there are; 0 when finished.
    pub step: (usize, usize),
    /// How far through this step, 0..1.
    pub progress: f32,
}

impl Tutorial {
    pub fn new() -> Self {
        Self { step: 0, progress: 0.0, done: None }
    }

    /// The player did something; true when that finished a step.
    pub fn note(&mut self, did: Did) -> bool {
        if self.done.is_some() {
            return false;
        }
        let amount = match (self.step, did) {
            (0, Did::Looked(px)) => px,
            (1, Did::Walked(m)) => m,
            (2, Did::Jumped) => 1.0,
            (3, Did::Sprinted(s)) => s,
            (4, Did::Swung) => 1.0,
            (5, Did::Destroyed) => 1.0,
            (6, Did::OpenedMenu) => 1.0,
            _ => return false,
        };
        self.progress += amount;
        if self.progress < STEPS[self.step].goal {
            return false;
        }
        self.step += 1;
        self.progress = 0.0;
        if self.step >= STEPS.len() {
            self.done = Some(0.0);
        }
        true
    }

    /// Counts down the closing card. True once it has gone.
    pub fn tick(&mut self, dt: f32) -> bool {
        if let Some(t) = &mut self.done {
            *t += dt;
            return *t > DONE_TIME;
        }
        false
    }

    #[cfg(test)]
    pub fn finished(&self) -> bool {
        self.done.is_some()
    }

    pub fn view(&self) -> TutorialView {
        match STEPS.get(self.step) {
            Some(s) => TutorialView {
                title: s.title,
                hint: s.hint,
                keys: s.keys,
                step: (self.step + 1, STEPS.len()),
                progress: (self.progress / s.goal).clamp(0.0, 1.0),
            },
            None => TutorialView {
                title: "You're ready!",
                hint: "Explore the valley, find the glowing skill crystals and grow stronger. More dangerous drones join as the threat level rises.",
                keys: &[],
                step: (0, STEPS.len()),
                progress: 1.0,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn steps_finish_by_doing_them_in_order() {
        let mut t = Tutorial::new();
        assert!(!t.note(Did::Jumped), "jumping doesn't count before its step");
        assert!(!t.note(Did::Looked(500.0)));
        assert!(t.note(Did::Looked(500.0)), "enough looking");
        assert!(t.note(Did::Walked(11.0)));
        t.note(Did::Jumped);
        assert!(t.note(Did::Jumped));
        assert!(t.note(Did::Sprinted(2.5)));
        for _ in 0..3 {
            t.note(Did::Swung);
        }
        assert_eq!(t.view().step.0, 6, "on to destroying a drone");
        t.note(Did::Destroyed);
        assert!(t.note(Did::OpenedMenu));
        assert!(t.finished() && t.view().step.0 == 0);
        assert!(!t.tick(1.0) && t.tick(DONE_TIME));
    }
}
