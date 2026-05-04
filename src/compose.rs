//! Minimal compose state machine. Only handles `dead_acute` + c -> ç override.
//! Everything else passes through transparently via the caller.

use std::collections::HashSet;

pub const KEY_DEAD_ACUTE: u32 = 0xfe51;
pub const KEY_C_LOWER: u32 = 0x0063;
pub const KEY_C_UPPER: u32 = 0x0043;

#[derive(Clone, Copy, Debug)]
pub struct PendingKey {
    pub keycode: u32,
    pub time: u32,
}

#[derive(Debug)]
pub enum Action {
    Forward,
    Swallow,
    Commit(&'static str),
    ReplayThenForward(PendingKey),
    ReplayThenBuffer(PendingKey),
}

#[derive(Default)]
pub struct ComposeState {
    pending: Option<PendingKey>,
    swallowed_releases: HashSet<u32>,
}

impl ComposeState {
    pub fn on_press(&mut self, sym: u32, keycode: u32, time: u32) -> Action {
        match self.pending.take() {
            None => self.handle_idle_press(sym, keycode, time),
            Some(prev) => self.handle_after_acute(prev, sym, keycode, time),
        }
    }

    fn handle_idle_press(&mut self, sym: u32, keycode: u32, time: u32) -> Action {
        if sym == KEY_DEAD_ACUTE {
            self.pending = Some(PendingKey { keycode, time });
            self.swallowed_releases.insert(keycode);
            Action::Swallow
        } else {
            Action::Forward
        }
    }

    fn handle_after_acute(
        &mut self,
        prev: PendingKey,
        sym: u32,
        keycode: u32,
        time: u32,
    ) -> Action {
        match sym {
            KEY_C_LOWER => {
                self.swallowed_releases.insert(keycode);
                Action::Commit("\u{00E7}")
            }
            KEY_C_UPPER => {
                self.swallowed_releases.insert(keycode);
                Action::Commit("\u{00C7}")
            }
            KEY_DEAD_ACUTE => {
                self.pending = Some(PendingKey { keycode, time });
                self.swallowed_releases.insert(keycode);
                Action::ReplayThenBuffer(prev)
            }
            _ => Action::ReplayThenForward(prev),
        }
    }

    pub fn on_release(&mut self, keycode: u32) -> bool {
        self.swallowed_releases.remove(&keycode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_letter_forwards() {
        let mut s = ComposeState::default();
        assert!(matches!(s.on_press(0x0061, 38, 0), Action::Forward));
    }

    #[test]
    fn dead_acute_then_c_emits_cedilla() {
        let mut s = ComposeState::default();
        assert!(matches!(s.on_press(KEY_DEAD_ACUTE, 48, 0), Action::Swallow));
        assert!(matches!(
            s.on_press(KEY_C_LOWER, 54, 1),
            Action::Commit("ç")
        ));
    }

    #[test]
    fn dead_acute_then_c_upper_emits_capital_cedilla() {
        let mut s = ComposeState::default();
        s.on_press(KEY_DEAD_ACUTE, 48, 0);
        assert!(matches!(
            s.on_press(KEY_C_UPPER, 54, 1),
            Action::Commit("Ç")
        ));
    }

    #[test]
    fn dead_acute_then_other_replays() {
        let mut s = ComposeState::default();
        s.on_press(KEY_DEAD_ACUTE, 48, 0);
        match s.on_press(0x0061, 38, 1) {
            Action::ReplayThenForward(p) => assert_eq!(p.keycode, 48),
            other => panic!("expected ReplayThenForward, got {other:?}"),
        }
    }

    #[test]
    fn release_of_swallowed_press_is_swallowed() {
        let mut s = ComposeState::default();
        s.on_press(KEY_DEAD_ACUTE, 48, 0);
        assert!(s.on_release(48));
    }

    #[test]
    fn release_of_unrelated_key_passes_through() {
        let mut s = ComposeState::default();
        assert!(!s.on_release(99));
    }
}
