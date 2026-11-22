use crate::settings::MouseButton;

#[derive(Clone, Copy)]
struct Press {
    point: (f64, f64),
    pixels_per_point: f64,
    time: u64,
}

#[derive(Default)]
pub struct ButtonTracker {
    presses: [Option<Press>; 5],
}

impl ButtonTracker {
    /// Windows supplies physical coordinates. AppKit supplies screen points
    /// and the backing scale of the display on which the drag began.
    pub fn should_display(
        &mut self,
        button: MouseButton,
        down: bool,
        point: (f64, f64),
        pixels_per_point: f64,
        time: u64,
        enabled: bool,
    ) -> bool {
        let press = &mut self.presses[button.index()];
        if down {
            *press = enabled.then_some(Press {
                point,
                pixels_per_point,
                time,
            });
            return enabled;
        }
        let Some(start) = press.take() else {
            return false;
        };
        let dx = (point.0 - start.point.0) * start.pixels_per_point;
        let dy = (point.1 - start.point.1) * start.pixels_per_point;
        enabled && (dx * dx + dy * dy > 200.0 * 200.0 || time.saturating_sub(start.time) > 500)
    }
}

#[cfg(test)]
impl ButtonTracker {
    fn on_down(&mut self, button: MouseButton, x: i32, y: i32, time: u64) {
        self.should_display(button, true, (x.into(), y.into()), 1.0, time, true);
    }

    fn on_up(&mut self, button: MouseButton, x: i32, y: i32, time: u64) -> bool {
        self.should_display(button, false, (x.into(), y.into()), 1.0, time, true)
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
    fn retina_drag_uses_initial_display_scale_and_strict_threshold() {
        let mut tracker = ButtonTracker::default();
        let button = MouseButton::Left;
        for (end_x, expected) in [(20.0, false), (20.5, true)] {
            assert!(tracker.should_display(button, true, (-80.0, 45.0), 2.0, 0, true));
            assert_eq!(
                tracker.should_display(button, false, (end_x, 45.0), 1.0, 50, true),
                expected
            );
        }
        assert!(!tracker.should_display(button, true, (0.0, 0.0), 2.0, 0, false));
        assert!(!tracker.should_display(button, false, (500.0, 0.0), 2.0, 501, true));
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
