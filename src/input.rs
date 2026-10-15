use crate::settings::MouseButton;

#[derive(Clone, Copy)]
struct Press {
    x: i32,
    y: i32,
    time: u64,
}

#[derive(Default)]
pub struct ButtonTracker {
    presses: [Option<Press>; 5],
}

impl ButtonTracker {
    pub fn on_down(&mut self, button: MouseButton, x: i32, y: i32, time: u64) {
        self.presses[button.index()] = Some(Press { x, y, time });
    }

    /// Show a release ripple after moving over 200 pixels or holding over 500 ms.
    pub fn on_up(&mut self, button: MouseButton, x: i32, y: i32, time: u64) -> bool {
        let Some(press) = self.presses[button.index()].take() else {
            return false;
        };
        // Widen before subtraction so coordinates across the virtual desktop cannot overflow.
        let dx = i128::from(x) - i128::from(press.x);
        let dy = i128::from(y) - i128::from(press.y);
        dx * dx + dy * dy > 200 * 200 || time.saturating_sub(press.time) > 500
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_distance_and_hold_thresholds_are_strict() {
        let mut tracker = ButtonTracker::default();
        for (x, y, elapsed, expected) in [
            (0, 0, 100, false),
            (200, 0, 500, false),
            (120, 160, 500, false),
            (201, 0, 100, true),
            (121, 160, 100, true),
            (0, 0, 501, true),
        ] {
            tracker.on_down(MouseButton::Left, -300, 50, 1000);
            assert_eq!(
                tracker.on_up(MouseButton::Left, -300 + x, 50 + y, 1000 + elapsed),
                expected
            );
        }
    }

    #[test]
    fn simultaneous_buttons_have_independent_positions_and_times() {
        let mut tracker = ButtonTracker::default();
        tracker.on_down(MouseButton::Left, 0, 0, 0);
        tracker.on_down(MouseButton::Right, 500, 500, 400);
        assert!(!tracker.on_up(MouseButton::Right, 500, 500, 600));
        assert!(tracker.on_up(MouseButton::Left, 0, 0, 600));
        assert!(!tracker.on_up(MouseButton::Left, 1000, 1000, 1000));
        assert!(!tracker.on_up(MouseButton::Middle, 1000, 1000, 1000));
    }

    #[test]
    fn negative_and_extreme_monitor_coordinates_do_not_overflow() {
        let mut tracker = ButtonTracker::default();
        tracker.on_down(MouseButton::XButton1, i32::MIN, i32::MIN, 0);
        assert!(tracker.on_up(MouseButton::XButton1, i32::MAX, i32::MAX, 0));
        tracker.on_down(MouseButton::XButton2, -2000, -1000, 100);
        assert!(!tracker.on_up(MouseButton::XButton2, -2000, -1000, 99));
    }
}
