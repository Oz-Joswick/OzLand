//! Sound: every effect is synthesised on the fly (no audio files).
//!
//! The game thread describes sounds as a handful of [`Voice`]s (an
//! oscillator or noise, a pitch sweep, a filter sweep and an envelope) and
//! sends them to the audio thread, which mixes them with the ambience (wind,
//! the rush of speed, birdsong) and a gentle generative soundtrack, adds a
//! little echo, and plays the result through cpal.
//!
//! If there is no audio device the game simply stays silent.

use std::f32::consts::{PI, TAU};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, mpsc};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use glam::Vec3;

/// Which volume slider a voice answers to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Bus {
    Sfx,
    Ambient,
    Music,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Wave {
    Sine,
    Triangle,
    Square,
    Saw,
    Noise,
}

/// One synthesised sound layer.
#[derive(Clone, Copy, Debug)]
struct Voice {
    wave: Wave,
    bus: Bus,
    /// Pitch sweeps exponentially from f0 to f1 over the duration (Hz).
    f0: f32,
    f1: f32,
    /// Low-pass cutoff sweep (Hz); 0 = no filter.
    lp0: f32,
    lp1: f32,
    /// High-pass cutoff (Hz); 0 = none.
    hp: f32,
    /// Seconds before it starts (for arpeggios and layered hits).
    delay: f32,
    dur: f32,
    attack: f32,
    /// Decay shape after the attack: higher falls away faster.
    curve: f32,
    gain: f32,
    /// -1 left .. 1 right.
    pan: f32,
    vib_rate: f32,
    vib_depth: f32,
    /// How much goes to the echo.
    send: f32,
    // Running state.
    t: f32,
    phase: f32,
    lp_a: f32,
    lp_b: f32,
    hp_y: f32,
    hp_x: f32,
}

fn tone(wave: Wave, f0: f32, f1: f32, dur: f32, gain: f32) -> Voice {
    Voice {
        wave,
        bus: Bus::Sfx,
        f0,
        f1,
        lp0: 0.0,
        lp1: 0.0,
        hp: 0.0,
        delay: 0.0,
        dur,
        attack: 0.004,
        curve: 2.0,
        gain,
        pan: 0.0,
        vib_rate: 0.0,
        vib_depth: 0.0,
        send: 0.18,
        t: 0.0,
        phase: 0.0,
        lp_a: 0.0,
        lp_b: 0.0,
        hp_y: 0.0,
        hp_x: 0.0,
    }
}

fn noise(dur: f32, gain: f32) -> Voice {
    tone(Wave::Noise, 1.0, 1.0, dur, gain)
}

impl Voice {
    fn lp(mut self, from: f32, to: f32) -> Self {
        (self.lp0, self.lp1) = (from, to);
        self
    }
    fn hp(mut self, cutoff: f32) -> Self {
        self.hp = cutoff;
        self
    }
    fn delay(mut self, s: f32) -> Self {
        self.delay = s;
        self
    }
    fn attack(mut self, s: f32) -> Self {
        self.attack = s;
        self
    }
    fn curve(mut self, c: f32) -> Self {
        self.curve = c;
        self
    }
    fn vib(mut self, rate: f32, depth: f32) -> Self {
        (self.vib_rate, self.vib_depth) = (rate, depth);
        self
    }
    fn send(mut self, amount: f32) -> Self {
        self.send = amount;
        self
    }
    fn pan(mut self, pan: f32) -> Self {
        self.pan = pan;
        self
    }
    fn bus(mut self, bus: Bus) -> Self {
        self.bus = bus;
        self
    }

    /// One sample, or None once finished.
    fn next(&mut self, dt: f32, rng: &mut u32) -> Option<f32> {
        if self.delay > 0.0 {
            self.delay -= dt;
            return Some(0.0);
        }
        if self.t >= self.dur {
            return None;
        }
        let x = self.t / self.dur;
        self.t += dt;
        let mut f = self.f0 * (self.f1 / self.f0).powf(x);
        if self.vib_depth > 0.0 {
            f *= 1.0 + self.vib_depth * (TAU * self.vib_rate * self.t).sin();
        }
        self.phase = (self.phase + f * dt).fract();
        let p = self.phase;
        let mut s = match self.wave {
            Wave::Sine => (TAU * p).sin(),
            Wave::Triangle => 1.0 - 4.0 * (p - 0.5).abs(),
            Wave::Square => {
                if p < 0.5 {
                    0.7
                } else {
                    -0.7
                }
            }
            Wave::Saw => (2.0 * p - 1.0) * 0.7,
            Wave::Noise => random(rng) * 2.0 - 1.0,
        };
        if self.lp0 > 0.0 {
            let cutoff = self.lp0 * (self.lp1 / self.lp0).powf(x);
            let a = one_pole(cutoff, dt);
            self.lp_a += (s - self.lp_a) * a;
            self.lp_b += (self.lp_a - self.lp_b) * a;
            s = self.lp_b;
        }
        if self.hp > 0.0 {
            let a = 1.0 / (1.0 + TAU * self.hp * dt);
            let y = a * (self.hp_y + s - self.hp_x);
            self.hp_x = s;
            self.hp_y = y;
            s = y;
        }
        let env = if self.t < self.attack {
            self.t / self.attack
        } else {
            ((self.dur - self.t) / (self.dur - self.attack).max(1e-4)).clamp(0.0, 1.0).powf(self.curve)
        };
        Some(s * env * self.gain)
    }
}

fn one_pole(cutoff: f32, dt: f32) -> f32 {
    let w = TAU * cutoff * dt;
    (w / (1.0 + w)).min(1.0)
}

fn random(rng: &mut u32) -> f32 {
    *rng ^= *rng << 13;
    *rng ^= *rng >> 17;
    *rng ^= *rng << 5;
    *rng as f32 / u32::MAX as f32
}

fn midi(note: f32) -> f32 {
    440.0 * 2f32.powf((note - 69.0) / 12.0)
}

/// Everything the game can make a noise about.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Sfx {
    Footstep,
    Jump,
    AirJump,
    /// Landing, 0..1 by how hard.
    Land(f32),
    Wings,
    Boost,
    Slam,
    Dash,
    Blaster,
    Scatter,
    RocketFire,
    Railgun,
    /// An explosion, by blast radius in metres.
    Explosion(f32),
    DroneDown,
    Hit,
    Kill,
    Crystal,
    SkillPoint,
    Unlock,
    Click,
    Toast,
    Hurt,
    Death,
    Fight,
    Beep,
    Grapple,
    Hooked,
    PadPlace,
    Bounce,
    Clone,
    Swap,
    TimeStop,
    TimeSlow,
    TimeResume,
    Rewind,
    Marker,
    Denied,
    WeaponSwitch,
    Victory,
    Defeat,
    /// A quick melee swing (blade, scythe) and a heavy one (hammer).
    Swing,
    HeavySwing,
    /// A melee blow landing.
    MeleeHit,
    HammerHit,
    /// Status effects: an electric shock, a critical hit.
    Zap,
    Crit,
    /// A drone firing at you.
    DroneShot,
    /// A barrier, ceiling or dome forming; struck; shattering.
    Build,
    ShieldHit,
    Shatter,
    /// A spell gathering, going off, lightning, and ice.
    Cast,
    SpellRelease,
    Thunder,
    Freeze,
}

impl Sfx {
    /// The layers of the sound, at pitch multiplier `k` (small random
    /// variation keeps repeats from sounding mechanical).
    fn voices(self, k: f32) -> Vec<Voice> {
        use Wave::*;
        match self {
            Sfx::Footstep => vec![
                noise(0.085, 0.20).lp(2200.0 * k, 500.0).hp(120.0).attack(0.002).curve(3.0).send(0.05),
                tone(Sine, 95.0 * k, 55.0, 0.06, 0.12).send(0.0),
            ],
            Sfx::Jump => vec![
                noise(0.14, 0.10).lp(2600.0, 700.0).attack(0.01),
                tone(Triangle, 200.0 * k, 380.0 * k, 0.12, 0.08),
            ],
            Sfx::AirJump => vec![
                noise(0.2, 0.12).lp(1200.0, 5000.0).attack(0.02).curve(1.5),
                tone(Triangle, 330.0 * k, 700.0 * k, 0.15, 0.09),
                tone(Sine, 660.0 * k, 1320.0 * k, 0.14, 0.05).delay(0.03),
            ],
            Sfx::Land(s) => {
                let s = s.clamp(0.0, 1.0);
                vec![
                    noise(0.12 + 0.15 * s, 0.14 + 0.3 * s).lp(1100.0 * k, 160.0).attack(0.002).curve(2.5).send(0.05),
                    tone(Sine, 120.0 * k, 42.0, 0.16 + 0.12 * s, 0.10 + 0.3 * s).send(0.0),
                ]
            }
            Sfx::Wings => vec![noise(0.28, 0.18).lp(3200.0, 500.0).hp(200.0).attack(0.04).curve(1.6)],
            Sfx::Boost => vec![
                noise(0.7, 0.28).lp(500.0, 3200.0).attack(0.02).curve(1.4),
                tone(Saw, 70.0 * k, 150.0 * k, 0.6, 0.12).lp(450.0, 700.0),
                tone(Sine, 240.0, 900.0, 0.25, 0.05),
            ],
            Sfx::Slam => vec![
                tone(Sine, 85.0, 28.0, 0.7, 0.6).send(0.1),
                noise(0.8, 0.5).lp(1500.0, 90.0).curve(2.2).send(0.3),
                tone(Square, 62.0, 34.0, 0.3, 0.12).lp(320.0, 200.0),
                noise(0.06, 0.25).lp(7000.0, 2000.0),
            ],
            Sfx::Dash => vec![
                noise(0.2, 0.22).lp(700.0, 7000.0).attack(0.01).curve(1.8),
                tone(Sine, 1400.0 * k, 260.0 * k, 0.2, 0.09),
                tone(Sine, 2600.0 * k, 2100.0 * k, 0.18, 0.035).delay(0.05).send(0.4),
            ],
            Sfx::Blaster => vec![
                tone(Square, 1500.0 * k, 240.0 * k, 0.11, 0.10).lp(4500.0, 1500.0),
                tone(Sine, 720.0 * k, 170.0 * k, 0.1, 0.12),
                noise(0.03, 0.08).lp(6000.0, 3000.0),
            ],
            Sfx::Scatter => vec![
                noise(0.3, 0.42).lp(3500.0, 350.0).curve(2.5).send(0.25),
                tone(Sine, 150.0 * k, 45.0, 0.22, 0.3),
                tone(Square, 320.0 * k, 80.0, 0.08, 0.1).lp(1600.0, 600.0),
            ],
            Sfx::RocketFire => vec![
                noise(0.5, 0.3).lp(1700.0, 500.0).attack(0.01).curve(1.6),
                tone(Saw, 130.0 * k, 55.0, 0.4, 0.14).lp(520.0, 300.0),
                tone(Sine, 420.0 * k, 140.0, 0.15, 0.1),
            ],
            Sfx::Railgun => vec![
                tone(Saw, 220.0 * k, 3200.0 * k, 0.08, 0.08).lp(6000.0, 6000.0).attack(0.06).curve(0.5),
                tone(Sine, 2700.0 * k, 800.0, 0.55, 0.11).delay(0.07).vib(30.0, 0.01).send(0.4),
                noise(0.28, 0.16).hp(1800.0).lp(9000.0, 3000.0).delay(0.07),
                tone(Sine, 95.0, 38.0, 0.35, 0.32).delay(0.07),
            ],
            Sfx::Explosion(r) => {
                let size = (r / 4.0).clamp(0.4, 2.5);
                vec![
                    noise(0.9 * size.sqrt(), 0.55).lp(2400.0 / size.sqrt(), 110.0).curve(2.2).send(0.35),
                    tone(Sine, 75.0 / size.sqrt(), 26.0, 0.8 * size.sqrt(), 0.55).send(0.1),
                    noise(0.08, 0.3).lp(7000.0, 1200.0),
                ]
            }
            Sfx::DroneDown => vec![
                noise(0.55, 0.35).lp(2600.0, 140.0).curve(2.0).send(0.3),
                tone(Square, 950.0 * k, 90.0, 0.4, 0.07).lp(3000.0, 600.0),
                tone(Sine, 70.0, 30.0, 0.45, 0.35),
            ],
            Sfx::Hit => vec![
                tone(Sine, 1900.0 * k, 1700.0 * k, 0.05, 0.10).send(0.0),
                tone(Triangle, 2850.0 * k, 2850.0 * k, 0.04, 0.05).send(0.0),
            ],
            Sfx::Kill => vec![
                tone(Sine, 880.0, 880.0, 0.12, 0.13),
                tone(Sine, 1320.0, 1320.0, 0.22, 0.13).delay(0.07),
                tone(Triangle, 1760.0, 1760.0, 0.26, 0.05).delay(0.07).send(0.4),
            ],
            Sfx::Crystal => {
                let notes = [84.0, 88.0, 91.0, 96.0, 100.0];
                let mut v: Vec<Voice> = notes
                    .iter()
                    .enumerate()
                    .map(|(i, &n)| tone(Sine, midi(n), midi(n), 0.6, 0.09).delay(i as f32 * 0.06).curve(2.5).send(0.5))
                    .collect();
                v.push(tone(Triangle, midi(108.0), midi(108.0), 0.9, 0.025).delay(0.3).vib(7.0, 0.004).send(0.6));
                v.push(noise(0.5, 0.04).hp(6000.0).attack(0.1).curve(1.5).send(0.6));
                v
            }
            Sfx::SkillPoint => {
                let notes = [67.0, 72.0, 76.0, 79.0];
                let mut v: Vec<Voice> = notes
                    .iter()
                    .enumerate()
                    .map(|(i, &n)| tone(Saw, midi(n), midi(n), 0.22, 0.06).lp(2600.0, 1200.0).delay(i as f32 * 0.09).send(0.35))
                    .collect();
                for n in [72.0, 76.0, 79.0, 84.0] {
                    v.push(tone(Saw, midi(n), midi(n), 1.3, 0.045).lp(2400.0, 700.0).delay(0.36).attack(0.02).curve(1.4).vib(5.0, 0.003).send(0.45));
                }
                v.push(tone(Sine, midi(48.0), midi(48.0), 1.2, 0.12).delay(0.36).curve(1.5));
                v
            }
            Sfx::Unlock => vec![
                tone(Triangle, midi(76.0), midi(76.0), 0.25, 0.09).send(0.4),
                tone(Triangle, midi(83.0), midi(83.0), 0.5, 0.09).delay(0.08).send(0.4),
                tone(Sine, midi(88.0), midi(88.0), 0.7, 0.05).delay(0.16).send(0.5),
                noise(0.4, 0.03).hp(5000.0).delay(0.1).send(0.5),
            ],
            Sfx::Click => vec![tone(Sine, 1300.0 * k, 800.0, 0.035, 0.07).send(0.0), noise(0.012, 0.03).hp(3000.0).send(0.0)],
            Sfx::Toast => vec![
                tone(Triangle, midi(84.0), midi(84.0), 0.09, 0.035).send(0.3),
                tone(Triangle, midi(91.0), midi(91.0), 0.15, 0.03).delay(0.05).send(0.3),
            ],
            Sfx::Hurt => vec![
                tone(Square, 230.0 * k, 110.0, 0.16, 0.12).lp(1300.0, 500.0),
                noise(0.12, 0.2).lp(1600.0, 300.0),
                tone(Sine, 90.0, 50.0, 0.15, 0.25),
            ],
            Sfx::Death => vec![
                tone(Saw, 440.0, 55.0, 1.3, 0.16).lp(2200.0, 250.0).vib(6.0, 0.02).send(0.4),
                noise(0.9, 0.3).lp(2000.0, 100.0).send(0.4),
            ],
            Sfx::Fight => {
                let mut v: Vec<Voice> = [57.0, 61.0, 64.0, 69.0]
                    .iter()
                    .map(|&n| tone(Saw, midi(n), midi(n), 0.9, 0.06).lp(800.0, 2600.0).attack(0.03).curve(1.3).send(0.4))
                    .collect();
                v.push(noise(0.5, 0.3).lp(3000.0, 200.0).send(0.4));
                v.push(tone(Sine, 110.0, 55.0, 0.6, 0.35));
                v
            }
            Sfx::Beep => vec![tone(Square, 880.0, 880.0, 0.1, 0.06).lp(3000.0, 3000.0).send(0.2)],
            Sfx::Grapple => vec![
                noise(0.18, 0.14).hp(1500.0).lp(8000.0, 2000.0).attack(0.005),
                tone(Square, 1700.0 * k, 420.0, 0.12, 0.04).lp(4500.0, 2000.0),
            ],
            Sfx::Hooked => vec![
                tone(Triangle, 2300.0 * k, 2200.0 * k, 0.08, 0.08).send(0.3),
                tone(Sine, 1150.0 * k, 1100.0 * k, 0.12, 0.08),
                noise(0.03, 0.1).hp(3000.0),
            ],
            Sfx::PadPlace => vec![tone(Sine, 280.0, 520.0, 0.12, 0.12), noise(0.06, 0.12).lp(1500.0, 400.0)],
            Sfx::Bounce => vec![
                tone(Sine, 170.0 * k, 760.0 * k, 0.38, 0.22).vib(13.0, 0.04).curve(1.5),
                tone(Triangle, 340.0 * k, 1520.0 * k, 0.3, 0.06),
                noise(0.08, 0.15).lp(1200.0, 300.0),
            ],
            Sfx::Clone => vec![
                tone(Sine, 600.0 * k, 1250.0 * k, 0.35, 0.09).vib(9.0, 0.02).send(0.5),
                tone(Triangle, 900.0 * k, 1850.0 * k, 0.3, 0.04).delay(0.05).send(0.5),
                noise(0.35, 0.04).hp(4500.0).attack(0.05).send(0.5),
            ],
            Sfx::Swap => vec![
                tone(Sine, 1600.0, 380.0, 0.25, 0.08).pan(-0.4).send(0.4),
                tone(Sine, 380.0, 1600.0, 0.25, 0.08).pan(0.4).send(0.4),
                noise(0.2, 0.1).lp(800.0, 6000.0),
            ],
            Sfx::TimeStop => vec![
                tone(Saw, 620.0, 55.0, 1.1, 0.12).lp(3200.0, 180.0).curve(1.2).send(0.6),
                tone(Sine, 1250.0, 90.0, 1.0, 0.09).send(0.6),
                tone(Sine, 60.0, 40.0, 1.2, 0.3),
            ],
            Sfx::TimeSlow => vec![
                tone(Sine, 520.0, 130.0, 0.8, 0.12).send(0.6),
                noise(0.8, 0.1).lp(3000.0, 200.0).attack(0.05).send(0.5),
            ],
            Sfx::TimeResume => vec![
                tone(Sine, 110.0, 820.0, 0.4, 0.1).send(0.4),
                noise(0.35, 0.08).lp(300.0, 5000.0).attack(0.1),
            ],
            Sfx::Rewind => vec![
                tone(Saw, 1100.0, 280.0, 0.5, 0.07).lp(3200.0, 1500.0).vib(18.0, 0.15).send(0.5),
                noise(0.45, 0.08).hp(2000.0).lp(9000.0, 3000.0),
            ],
            Sfx::Marker => vec![
                tone(Triangle, midi(88.0), midi(88.0), 0.15, 0.06).send(0.4),
                tone(Triangle, midi(93.0), midi(93.0), 0.25, 0.06).delay(0.08).send(0.4),
            ],
            Sfx::Denied => vec![
                tone(Square, 170.0, 150.0, 0.09, 0.07).lp(1300.0, 1000.0).send(0.0),
                tone(Square, 150.0, 130.0, 0.12, 0.07).lp(1300.0, 1000.0).delay(0.1).send(0.0),
            ],
            Sfx::WeaponSwitch => vec![
                noise(0.03, 0.12).hp(2000.0).send(0.0),
                tone(Sine, 900.0, 1100.0, 0.05, 0.06).delay(0.04).send(0.0),
                noise(0.025, 0.1).hp(2500.0).delay(0.07).send(0.0),
            ],
            Sfx::Victory => {
                let mut v = Vec::new();
                for (i, n) in [60.0, 64.0, 67.0, 72.0].into_iter().enumerate() {
                    v.push(tone(Saw, midi(n), midi(n), 0.2, 0.06).lp(3000.0, 1500.0).delay(i as f32 * 0.11).send(0.4));
                }
                for n in [60.0, 64.0, 67.0, 72.0, 76.0] {
                    v.push(tone(Saw, midi(n), midi(n), 1.6, 0.04).lp(3000.0, 700.0).delay(0.44).attack(0.02).curve(1.3).vib(5.0, 0.004).send(0.5));
                }
                v
            }
            Sfx::Swing => vec![
                noise(0.16, 0.2).lp(900.0 * k, 5000.0 * k).hp(300.0).attack(0.03).curve(1.6).send(0.1),
                tone(Sine, 520.0 * k, 260.0 * k, 0.12, 0.04),
            ],
            Sfx::HeavySwing => vec![
                noise(0.32, 0.26).lp(400.0 * k, 2200.0 * k).attack(0.08).curve(1.4).send(0.1),
                tone(Sine, 140.0 * k, 70.0, 0.3, 0.08).attack(0.05),
            ],
            Sfx::MeleeHit => vec![
                noise(0.09, 0.3).lp(4000.0 * k, 600.0).attack(0.001).curve(2.5),
                tone(Square, 420.0 * k, 160.0, 0.08, 0.06).lp(2500.0, 900.0),
                tone(Sine, 140.0 * k, 60.0, 0.1, 0.22),
            ],
            Sfx::HammerHit => vec![
                tone(Sine, 95.0, 32.0, 0.45, 0.55).send(0.15),
                noise(0.35, 0.45).lp(2200.0, 120.0).curve(2.2).send(0.3),
                tone(Triangle, 1800.0 * k, 1700.0 * k, 0.4, 0.03).vib(9.0, 0.01).send(0.4),
            ],
            Sfx::Zap => vec![
                tone(Saw, 1900.0 * k, 600.0, 0.16, 0.06).lp(7000.0, 3000.0).vib(55.0, 0.2).send(0.2),
                noise(0.1, 0.1).hp(3500.0),
            ],
            Sfx::Crit => vec![
                tone(Triangle, 2600.0, 2600.0, 0.18, 0.07).send(0.3),
                tone(Sine, 1300.0, 1300.0, 0.12, 0.08),
                noise(0.05, 0.08).hp(5000.0),
            ],
            Sfx::DroneShot => vec![
                tone(Square, 760.0 * k, 380.0 * k, 0.16, 0.06).lp(3000.0, 1200.0),
                tone(Sine, 1500.0 * k, 900.0 * k, 0.12, 0.05),
            ],
            Sfx::Build => vec![
                tone(Saw, 110.0, 440.0, 0.4, 0.07).lp(600.0, 4000.0).attack(0.05).curve(1.2).send(0.4),
                tone(Sine, 220.0, 880.0, 0.35, 0.08).vib(14.0, 0.02).send(0.4),
                noise(0.35, 0.06).hp(2500.0).attack(0.1).send(0.5),
                tone(Triangle, midi(88.0), midi(88.0), 0.4, 0.04).delay(0.3).send(0.6),
            ],
            Sfx::ShieldHit => vec![
                tone(Sine, 1250.0 * k, 900.0 * k, 0.14, 0.06).vib(40.0, 0.03).send(0.3),
                noise(0.06, 0.05).hp(3000.0),
            ],
            Sfx::Shatter => {
                let mut v = vec![noise(0.6, 0.3).hp(2500.0).lp(12000.0, 3000.0).curve(2.0).send(0.5), tone(Sine, 140.0, 50.0, 0.4, 0.3)];
                for (i, n) in [96.0, 101.0, 91.0, 99.0, 94.0].into_iter().enumerate() {
                    v.push(tone(Triangle, midi(n), midi(n), 0.25, 0.035).delay(i as f32 * 0.035).curve(3.0).send(0.6));
                }
                v
            }
            Sfx::Cast => vec![
                tone(Sine, 220.0, 880.0, 1.4, 0.07).attack(0.6).curve(0.8).vib(6.0, 0.02).send(0.6),
                tone(Triangle, 330.0, 1320.0, 1.4, 0.04).attack(0.8).curve(0.8).vib(9.0, 0.03).send(0.6),
                noise(1.3, 0.05).hp(4000.0).attack(0.9).curve(0.8).send(0.6),
            ],
            Sfx::SpellRelease => {
                let mut v = vec![noise(0.7, 0.25).lp(500.0, 8000.0).attack(0.01).curve(1.8).send(0.6), tone(Sine, 80.0, 40.0, 0.6, 0.3)];
                for (i, n) in [76.0, 83.0, 88.0, 95.0].into_iter().enumerate() {
                    v.push(tone(Triangle, midi(n), midi(n), 0.6, 0.05).delay(i as f32 * 0.04).send(0.7));
                }
                v
            }
            Sfx::Thunder => vec![
                noise(0.12, 0.5).hp(1500.0).lp(12000.0, 4000.0).attack(0.002),
                noise(1.6, 0.45).lp(900.0, 80.0).delay(0.05).attack(0.05).curve(1.6).send(0.5),
                tone(Saw, 2400.0 * k, 300.0, 0.2, 0.06).lp(8000.0, 2000.0).vib(70.0, 0.3),
            ],
            Sfx::Freeze => vec![
                noise(1.0, 0.18).hp(3000.0).lp(14000.0, 6000.0).attack(0.05).curve(1.4).send(0.6),
                tone(Sine, 1800.0, 2600.0, 0.8, 0.05).vib(20.0, 0.02).send(0.7),
                tone(Triangle, midi(100.0), midi(100.0), 0.9, 0.03).delay(0.2).send(0.7),
            ],
            Sfx::Defeat => [67.0, 63.0, 60.0, 55.0]
                .into_iter()
                .enumerate()
                .map(|(i, n)| tone(Triangle, midi(n), midi(n) * 0.98, 0.5, 0.08).lp(2000.0, 800.0).delay(i as f32 * 0.22).send(0.5))
                .collect(),
        }
    }

    /// How much the pitch wanders between repeats.
    fn variation(self) -> f32 {
        match self {
            Sfx::Footstep => 0.14,
            Sfx::Swing | Sfx::HeavySwing | Sfx::MeleeHit | Sfx::DroneShot => 0.08,
            Sfx::Land(_) | Sfx::Blaster | Sfx::Scatter | Sfx::Hit | Sfx::Jump => 0.06,
            Sfx::Explosion(_) | Sfx::DroneDown | Sfx::Bounce | Sfx::Grapple | Sfx::Hooked | Sfx::Dash => 0.05,
            _ => 0.0,
        }
    }
}

/// Values shared with the audio thread, as f32 bits.
#[derive(Default)]
struct Params {
    master: AtomicU32,
    sfx: AtomicU32,
    ambient: AtomicU32,
    music: AtomicU32,
    /// Breeze strength, 0..1.
    wind: AtomicU32,
    /// Air rushing past at speed, 0..1.
    rush: AtomicU32,
    /// Birdsong, 0..1.
    birds: AtomicU32,
    /// 1 = muffled (paused).
    muffle: AtomicU32,
    /// Music intensity: 0 calm, 1 tense (a fight).
    tension: AtomicU32,
}

fn load(a: &AtomicU32) -> f32 {
    f32::from_bits(a.load(Ordering::Relaxed))
}

fn store(a: &AtomicU32, v: f32) {
    a.store(v.to_bits(), Ordering::Relaxed);
}

/// The game's handle on the sound system.
pub struct Audio {
    _stream: Option<cpal::Stream>,
    tx: Option<mpsc::Sender<Vec<Voice>>>,
    params: Arc<Params>,
    listener: Vec3,
    right: Vec3,
    rng: u32,
}

impl Audio {
    /// Opens the default output device; silent if there isn't one.
    pub fn new() -> Self {
        let params = Arc::new(Params::default());
        for (p, v) in [(&params.master, 0.8), (&params.sfx, 0.8), (&params.ambient, 0.7), (&params.music, 0.45)] {
            store(p, v);
        }
        let (tx, rx) = mpsc::channel();
        let stream = match open_stream(rx, params.clone()) {
            Ok(s) => Some(s),
            Err(e) => {
                log::warn!("no sound: {e:#}");
                None
            }
        };
        let tx = stream.as_ref().map(|_| tx);
        Self { _stream: stream, tx, params, listener: Vec3::ZERO, right: Vec3::X, rng: 0x1234_5679 }
    }

    pub fn set_volumes(&self, master: f32, sfx: f32, ambient: f32, music: f32) {
        store(&self.params.master, master);
        store(&self.params.sfx, sfx);
        store(&self.params.ambient, ambient);
        store(&self.params.music, music);
    }

    /// Where the ears are and which way they face (for 3D sounds).
    pub fn set_listener(&mut self, pos: Vec3, forward: Vec3) {
        self.listener = pos;
        self.right = Vec3::new(-forward.z, 0.0, forward.x).normalize_or(Vec3::X);
    }

    pub fn set_ambience(&self, wind: f32, rush: f32, birds: f32) {
        store(&self.params.wind, wind);
        store(&self.params.rush, rush);
        store(&self.params.birds, birds);
    }

    pub fn set_muffled(&self, muffled: bool) {
        store(&self.params.muffle, if muffled { 1.0 } else { 0.0 });
    }

    pub fn set_tension(&self, tension: f32) {
        store(&self.params.tension, tension);
    }

    /// A sound with no place in the world (UI, your own footsteps).
    pub fn play(&mut self, sfx: Sfx) {
        self.send(sfx, 1.0, 0.0, 1.0);
    }

    /// A sound at a point in the world: quieter, duller and panned with
    /// distance and direction.
    pub fn play_at(&mut self, sfx: Sfx, pos: Vec3) {
        let to = pos - self.listener;
        let distance = to.length();
        let loud = matches!(sfx, Sfx::Explosion(_) | Sfx::Slam | Sfx::DroneDown | Sfx::Railgun);
        let reach = if loud { 22.0 } else { 9.0 };
        let gain = 1.0 / (1.0 + (distance / reach).powf(1.6));
        if gain < 0.015 {
            return;
        }
        let pan = if distance > 0.5 { (to / distance).dot(self.right) * 0.75 } else { 0.0 };
        let dull = 1.0 / (1.0 + distance / 60.0);
        self.send(sfx, gain, pan, dull);
    }

    fn send(&mut self, sfx: Sfx, gain: f32, pan: f32, dull: f32) {
        let Some(tx) = &self.tx else { return };
        let k = 1.0 + (random(&mut self.rng) * 2.0 - 1.0) * sfx.variation();
        let voices = sfx
            .voices(k)
            .into_iter()
            .map(|mut v| {
                v.gain *= gain;
                v.pan = (v.pan + pan).clamp(-1.0, 1.0);
                if dull < 0.98 {
                    let (a, b) = if v.lp0 > 0.0 { (v.lp0, v.lp1) } else { (16000.0, 16000.0) };
                    (v.lp0, v.lp1) = (a * dull.max(0.08), b * dull.max(0.08));
                }
                v
            })
            .collect();
        let _ = tx.send(voices);
    }
}

/// The audio thread's state.
struct Mixer {
    rx: mpsc::Receiver<Vec<Voice>>,
    params: Arc<Params>,
    voices: Vec<Voice>,
    dt: f32,
    rng: u32,
    // Echo: two delay lines with crossed feedback.
    echo_l: Vec<f32>,
    echo_r: Vec<f32>,
    echo_i: usize,
    echo_lp: (f32, f32),
    // Ambience.
    wind_lp: (f32, f32),
    rush_lp: (f32, f32),
    gust: f32,
    gust_target: f32,
    gust_timer: f32,
    bird_timer: f32,
    // Music.
    chord: usize,
    chord_timer: f32,
    pluck_timer: f32,
    // Smoothed parameters.
    muffle: f32,
    muffle_lp: (f32, f32),
    rush: f32,
    wind: f32,
}

/// A slow, warm progression (A minor / C major): Am9, Fmaj7, C, G6.
const CHORDS: [[f32; 4]; 4] = [[57.0, 60.0, 64.0, 71.0], [53.0, 57.0, 60.0, 64.0], [48.0, 55.0, 60.0, 64.0], [55.0, 59.0, 62.0, 64.0]];
/// Tense: Am, Dm/A, E, Am in a lower, darker voicing.
const TENSE: [[f32; 4]; 4] = [[45.0, 52.0, 57.0, 60.0], [45.0, 50.0, 57.0, 62.0], [44.0, 52.0, 56.0, 59.0], [45.0, 52.0, 57.0, 60.0]];
const PENTATONIC: [f32; 5] = [0.0, 3.0, 5.0, 7.0, 10.0];
const CHORD_TIME: f32 = 6.0;

impl Mixer {
    fn new(rx: mpsc::Receiver<Vec<Voice>>, params: Arc<Params>, sample_rate: f32) -> Self {
        let len_l = (sample_rate * 0.137) as usize;
        let len_r = (sample_rate * 0.183) as usize;
        Self {
            rx,
            params,
            voices: Vec::with_capacity(256),
            dt: 1.0 / sample_rate,
            rng: 0x9e37_79b9,
            echo_l: vec![0.0; len_l.max(1)],
            echo_r: vec![0.0; len_r.max(1)],
            echo_i: 0,
            echo_lp: (0.0, 0.0),
            wind_lp: (0.0, 0.0),
            rush_lp: (0.0, 0.0),
            gust: 0.5,
            gust_target: 0.5,
            gust_timer: 0.0,
            bird_timer: 3.0,
            chord: 0,
            chord_timer: 0.5,
            pluck_timer: 2.0,
            muffle: 0.0,
            muffle_lp: (0.0, 0.0),
            rush: 0.0,
            wind: 0.0,
        }
    }

    fn rand(&mut self) -> f32 {
        random(&mut self.rng)
    }

    /// Schedules ambience and music events; called once per buffer.
    fn schedule(&mut self, elapsed: f32) {
        while let Ok(vs) = self.rx.try_recv() {
            if self.voices.len() + vs.len() > 240 {
                // Too busy: drop the oldest sounds.
                let excess = self.voices.len() + vs.len() - 240;
                self.voices.drain(..excess.min(self.voices.len()));
            }
            self.voices.extend(vs);
        }
        let birds = load(&self.params.birds);
        self.bird_timer -= elapsed;
        if self.bird_timer <= 0.0 {
            self.bird_timer = 2.0 + self.rand() * 7.0;
            if birds > 0.01 {
                self.bird_call(birds);
            }
        }
        self.gust_timer -= elapsed;
        if self.gust_timer <= 0.0 {
            self.gust_timer = 1.5 + self.rand() * 4.0;
            self.gust_target = 0.25 + self.rand() * 0.75;
        }

        let music = load(&self.params.music);
        let tension = load(&self.params.tension);
        self.chord_timer -= elapsed;
        if self.chord_timer <= 0.0 {
            self.chord_timer = CHORD_TIME;
            self.chord = (self.chord + 1) % CHORDS.len();
            if music > 0.001 {
                let notes = if tension > 0.5 { TENSE[self.chord] } else { CHORDS[self.chord] };
                for (i, &n) in notes.iter().enumerate() {
                    let pan = (i as f32 - 1.5) * 0.3;
                    let f = midi(n);
                    self.voices.push(
                        tone(Wave::Triangle, f, f, CHORD_TIME + 2.5, 0.022)
                            .lp(700.0, 1100.0)
                            .attack(2.2)
                            .curve(1.3)
                            .vib(0.3 + i as f32 * 0.07, 0.002)
                            .pan(pan)
                            .send(0.5)
                            .bus(Bus::Music),
                    );
                    self.voices.push(
                        tone(Wave::Sine, f * 1.003, f * 1.003, CHORD_TIME + 2.5, 0.018)
                            .attack(2.5)
                            .curve(1.3)
                            .pan(-pan)
                            .send(0.5)
                            .bus(Bus::Music),
                    );
                }
                let root = midi(notes[0] - 12.0);
                self.voices.push(tone(Wave::Sine, root, root, CHORD_TIME + 1.5, 0.05).attack(1.5).curve(1.2).send(0.2).bus(Bus::Music));
                if tension > 0.5 {
                    // A low pulse under the fight.
                    for beat in 0..12 {
                        self.voices.push(
                            tone(Wave::Sine, root * 0.5 * 1.5, root * 0.5, 0.25, 0.09)
                                .delay(beat as f32 * 0.5)
                                .curve(3.0)
                                .send(0.1)
                                .bus(Bus::Music),
                        );
                    }
                }
            }
        }
        self.pluck_timer -= elapsed;
        if self.pluck_timer <= 0.0 {
            self.pluck_timer = 0.5;
            if music > 0.001 && self.rand() < 0.38 {
                let base = if tension > 0.5 { 69.0 } else { 69.0 + [0.0, 12.0][(self.rand() * 2.0) as usize % 2] };
                let n = base + PENTATONIC[(self.rand() * 5.0) as usize % 5];
                let f = midi(n);
                let pan = self.rand() * 1.2 - 0.6;
                self.voices.push(
                    tone(Wave::Triangle, f, f, 1.6, 0.03).lp(3500.0, 900.0).attack(0.004).curve(3.0).pan(pan).send(0.7).bus(Bus::Music),
                );
                self.voices.push(tone(Wave::Sine, f * 2.0, f * 2.0, 0.8, 0.008).attack(0.004).curve(3.0).pan(pan).send(0.7).bus(Bus::Music));
            }
        }
    }

    /// A little run of chirps from somewhere in the trees.
    fn bird_call(&mut self, level: f32) {
        let pan = self.rand() * 1.6 - 0.8;
        let base = 2600.0 + self.rand() * 2200.0;
        let chirps = 2 + (self.rand() * 5.0) as usize;
        let kind = self.rand();
        let mut at = 0.0;
        for _ in 0..chirps {
            let len = 0.05 + self.rand() * 0.08;
            let (f0, f1) = if kind < 0.5 { (base, base * 1.4) } else { (base * 1.3, base * 0.85) };
            let trill = 28.0 + self.rand() * 20.0;
            self.voices.push(
                tone(Wave::Sine, f0, f1, len, 0.03 * level)
                    .attack(0.01)
                    .curve(1.2)
                    .vib(trill, 0.03)
                    .pan(pan)
                    .send(0.45)
                    .bus(Bus::Ambient)
                    .delay(at),
            );
            at += len + 0.03 + self.rand() * 0.09;
        }
    }

    fn frame(&mut self) -> (f32, f32) {
        let dt = self.dt;
        let p = &self.params;
        let (master, sfx_v, amb_v, mus_v) = (load(&p.master), load(&p.sfx), load(&p.ambient), load(&p.music));
        let k = (dt * 3.0).min(1.0);
        self.muffle += (load(&p.muffle) - self.muffle) * k;
        self.rush += (load(&p.rush) - self.rush) * (dt * 4.0).min(1.0);
        self.wind += (load(&p.wind) - self.wind) * k;
        self.gust += (self.gust_target - self.gust) * (dt * 0.6);

        let (mut l, mut r, mut send_l, mut send_r) = (0.0, 0.0, 0.0, 0.0);
        let (mut world_l, mut world_r) = (0.0, 0.0);
        let mut rng = self.rng;
        self.voices.retain_mut(|v| {
            let Some(s) = v.next(dt, &mut rng) else { return false };
            let vol = match v.bus {
                Bus::Sfx => sfx_v,
                Bus::Ambient => amb_v,
                Bus::Music => mus_v,
            };
            let s = s * vol;
            let a = (v.pan + 1.0) * PI * 0.25;
            let (pl, pr) = (a.cos() * s, a.sin() * s);
            if v.bus == Bus::Music {
                l += pl;
                r += pr;
            } else {
                world_l += pl;
                world_r += pr;
            }
            send_l += pl * v.send;
            send_r += pr * v.send;
            true
        });

        // Wind: filtered noise that swells in gusts.
        let n1 = random(&mut rng) * 2.0 - 1.0;
        let n2 = random(&mut rng) * 2.0 - 1.0;
        self.rng = rng;
        let wind_cut = 180.0 + 500.0 * self.gust * self.wind;
        let a = one_pole(wind_cut, dt);
        self.wind_lp.0 += (n1 - self.wind_lp.0) * a;
        self.wind_lp.1 += (self.wind_lp.0 - self.wind_lp.1) * a;
        let wind = self.wind_lp.1 * (0.15 + 0.85 * self.gust) * self.wind * 0.9 * amb_v;
        // Rush: brighter and louder the faster you go.
        let rush_cut = 300.0 + 2500.0 * self.rush;
        let a = one_pole(rush_cut, dt);
        self.rush_lp.0 += (n2 - self.rush_lp.0) * a;
        self.rush_lp.1 += (self.rush_lp.0 - self.rush_lp.1) * a;
        let rush = self.rush_lp.1 * self.rush * self.rush * 1.4 * sfx_v;
        world_l += wind * 0.9 + rush;
        world_r += wind + rush * 0.9;

        // Paused: the world sounds far away.
        if self.muffle > 0.01 {
            let a = one_pole(16000.0 * (1.0 - self.muffle) + 500.0 * self.muffle, dt);
            self.muffle_lp.0 += (world_l - self.muffle_lp.0) * a;
            self.muffle_lp.1 += (world_r - self.muffle_lp.1) * a;
            let keep = 1.0 - 0.45 * self.muffle;
            world_l = self.muffle_lp.0 * keep;
            world_r = self.muffle_lp.1 * keep;
        } else {
            self.muffle_lp = (world_l, world_r);
        }
        l += world_l;
        r += world_r;

        // Echo.
        let (il, ir) = (self.echo_i % self.echo_l.len(), self.echo_i % self.echo_r.len());
        let (el, er) = (self.echo_l[il], self.echo_r[ir]);
        let a = one_pole(3000.0, dt);
        self.echo_lp.0 += (el - self.echo_lp.0) * a;
        self.echo_lp.1 += (er - self.echo_lp.1) * a;
        self.echo_l[il] = send_l + self.echo_lp.1 * 0.42;
        self.echo_r[ir] = send_r + self.echo_lp.0 * 0.42;
        self.echo_i = self.echo_i.wrapping_add(1);
        l += el * 0.35;
        r += er * 0.35;

        // Gentle limiter.
        ((l * master).tanh(), (r * master).tanh())
    }
}

fn open_stream(rx: mpsc::Receiver<Vec<Voice>>, params: Arc<Params>) -> anyhow::Result<cpal::Stream> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or_else(|| anyhow::anyhow!("no output device"))?;
    let config = device.default_output_config()?;
    let format = config.sample_format();
    let config: cpal::StreamConfig = config.into();
    let stream = match format {
        cpal::SampleFormat::F32 => build::<f32>(&device, config, rx, params)?,
        cpal::SampleFormat::I16 => build::<i16>(&device, config, rx, params)?,
        cpal::SampleFormat::U16 => build::<u16>(&device, config, rx, params)?,
        cpal::SampleFormat::I32 => build::<i32>(&device, config, rx, params)?,
        cpal::SampleFormat::F64 => build::<f64>(&device, config, rx, params)?,
        other => anyhow::bail!("unsupported sample format {other:?}"),
    };
    stream.play()?;
    Ok(stream)
}

fn build<T>(device: &cpal::Device, config: cpal::StreamConfig, rx: mpsc::Receiver<Vec<Voice>>, params: Arc<Params>) -> anyhow::Result<cpal::Stream>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let channels = config.channels as usize;
    let sample_rate = config.sample_rate as f32;
    let mut mixer = Mixer::new(rx, params, sample_rate);
    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            let frames = data.len() / channels.max(1);
            mixer.schedule(frames as f32 / sample_rate);
            for frame in data.chunks_mut(channels.max(1)) {
                let (l, r) = mixer.frame();
                for (i, out) in frame.iter_mut().enumerate() {
                    let v = match (channels, i) {
                        (1, _) => (l + r) * 0.5,
                        (_, 0) => l,
                        (_, 1) => r,
                        _ => 0.0,
                    };
                    *out = T::from_sample(v);
                }
            }
        },
        |e| log::warn!("audio stream error: {e}"),
        None,
    )?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Sfx; 55] = [
        Sfx::Cast,
        Sfx::SpellRelease,
        Sfx::Thunder,
        Sfx::Freeze,
        Sfx::Build,
        Sfx::ShieldHit,
        Sfx::Shatter,
        Sfx::Swing,
        Sfx::HeavySwing,
        Sfx::MeleeHit,
        Sfx::HammerHit,
        Sfx::Zap,
        Sfx::Crit,
        Sfx::DroneShot,
        Sfx::Footstep,
        Sfx::Jump,
        Sfx::AirJump,
        Sfx::Land(0.5),
        Sfx::Wings,
        Sfx::Boost,
        Sfx::Slam,
        Sfx::Dash,
        Sfx::Blaster,
        Sfx::Scatter,
        Sfx::RocketFire,
        Sfx::Railgun,
        Sfx::Explosion(4.0),
        Sfx::DroneDown,
        Sfx::Hit,
        Sfx::Kill,
        Sfx::Crystal,
        Sfx::SkillPoint,
        Sfx::Unlock,
        Sfx::Click,
        Sfx::Toast,
        Sfx::Hurt,
        Sfx::Death,
        Sfx::Fight,
        Sfx::Beep,
        Sfx::Grapple,
        Sfx::Hooked,
        Sfx::PadPlace,
        Sfx::Bounce,
        Sfx::Clone,
        Sfx::Swap,
        Sfx::TimeStop,
        Sfx::TimeSlow,
        Sfx::TimeResume,
        Sfx::Rewind,
        Sfx::Marker,
        Sfx::Denied,
        Sfx::WeaponSwitch,
        Sfx::Victory,
        Sfx::Defeat,
        Sfx::Explosion(12.0),
    ];

    #[test]
    fn every_sound_is_finite_bounded_and_ends() {
        let dt = 1.0 / 48_000.0;
        let mut rng = 7;
        for sfx in ALL {
            for mut v in sfx.voices(1.0) {
                let mut n = 0;
                while let Some(s) = v.next(dt, &mut rng) {
                    assert!(s.is_finite() && s.abs() < 2.0, "{sfx:?}: {s}");
                    n += 1;
                    assert!(n < 48_000 * 5, "{sfx:?} never ends");
                }
            }
        }
    }

    #[test]
    fn the_mixer_stays_finite_with_everything_playing() {
        let (tx, rx) = mpsc::channel();
        let params = Arc::new(Params::default());
        for p in [&params.master, &params.sfx, &params.ambient, &params.music, &params.wind, &params.rush, &params.birds] {
            store(p, 1.0);
        }
        let mut m = Mixer::new(rx, params, 48_000.0);
        for sfx in ALL {
            tx.send(sfx.voices(1.0)).unwrap();
        }
        for _ in 0..200 {
            m.schedule(0.01);
            for _ in 0..480 {
                let (l, r) = m.frame();
                assert!(l.is_finite() && r.is_finite() && l.abs() <= 1.0 && r.abs() <= 1.0);
            }
        }
    }
}
