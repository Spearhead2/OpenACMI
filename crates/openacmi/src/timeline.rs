//! Playback clock: which instant of the recording is on screen.

/// Playback speeds offered in the UI.
pub const SPEEDS: [f64; 9] = [0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0];

#[derive(Debug, Clone, PartialEq)]
pub struct Timeline {
    /// Displayed time, in the recording's frame time (seconds).
    pub time: f64,
    pub playing: bool,
    /// Playback rate relative to real time.
    pub speed: f64,
    /// Stick to the newest data of a live stream.
    pub follow_live: bool,
}

impl Default for Timeline {
    fn default() -> Self {
        Self { time: 0.0, playing: false, speed: 1.0, follow_live: false }
    }
}

impl Timeline {
    /// Advances the clock by `dt` real seconds within `range`.
    pub fn advance(&mut self, dt: f64, range: Option<(f64, f64)>) {
        let Some((start, end)) = range else { return };
        if self.follow_live {
            self.time = end;
            return;
        }
        if self.playing {
            self.time += dt * self.speed;
            if self.time >= end {
                self.time = end;
                self.playing = false;
            }
        }
        self.time = self.time.clamp(start, end);
    }

    /// Jumps to `time`. Scrubbing away from the end leaves live-follow mode.
    pub fn seek(&mut self, time: f64, range: Option<(f64, f64)>) {
        let (start, end) = range.unwrap_or((time, time));
        self.time = time.clamp(start, end);
        self.follow_live = false;
    }

    pub fn toggle_play(&mut self, range: Option<(f64, f64)>) {
        self.follow_live = false;
        self.playing = !self.playing;
        // Pressing play at the very end restarts from the beginning.
        if let (true, Some((start, end))) = (self.playing, range) {
            if self.time >= end {
                self.time = start;
            }
        }
    }

    /// Selects the next slower (`-1`) or faster (`+1`) preset speed.
    pub fn step_speed(&mut self, direction: i32) {
        let current = SPEEDS.iter().position(|&s| s >= self.speed).unwrap_or(SPEEDS.len() - 1) as i32;
        self.speed = SPEEDS[(current + direction).clamp(0, SPEEDS.len() as i32 - 1) as usize];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plays_and_stops_at_the_end() {
        let range = Some((10.0, 20.0));
        let mut timeline = Timeline { time: 10.0, ..Default::default() };
        timeline.toggle_play(range);
        timeline.speed = 2.0;
        timeline.advance(3.0, range);
        assert_eq!(timeline.time, 16.0);
        timeline.advance(10.0, range);
        assert_eq!((timeline.time, timeline.playing), (20.0, false));
        timeline.toggle_play(range);
        assert_eq!(timeline.time, 10.0, "play at the end restarts");
    }

    #[test]
    fn live_follow_tracks_the_end_until_scrubbed() {
        let mut timeline = Timeline { follow_live: true, ..Default::default() };
        timeline.advance(0.1, Some((0.0, 42.0)));
        assert_eq!(timeline.time, 42.0);
        timeline.seek(5.0, Some((0.0, 42.0)));
        assert!(!timeline.follow_live);
        timeline.advance(0.1, Some((0.0, 50.0)));
        assert_eq!(timeline.time, 5.0);
    }

    #[test]
    fn speed_steps_through_presets() {
        let mut timeline = Timeline::default();
        timeline.step_speed(1);
        assert_eq!(timeline.speed, 2.0);
        timeline.step_speed(-1);
        timeline.step_speed(-1);
        assert_eq!(timeline.speed, 0.5);
    }
}
