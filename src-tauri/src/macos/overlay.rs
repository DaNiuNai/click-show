//! Native, click-through panels in the AppKit screen coordinate system (points).
use crate::settings::Color;
use objc2::{MainThreadMarker, MainThreadOnly, rc::Retained};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSPanel, NSPopUpMenuWindowLevel,
    NSWindowCollectionBehavior as Behavior, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use objc2_quartz_core::{CALayer, CATransaction};

pub struct Indicator {
    panel: Retained<NSPanel>,
    circle: Retained<CALayer>,
    side: f64,
}

impl Indicator {
    pub fn new(
        mtm: MainThreadMarker,
        point: NSPoint,
        side: f64,
        color: Color,
        filled: bool,
    ) -> Self {
        let frame = NSRect::new(Self::origin(point, side), NSSize::new(side, side));
        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            frame,
            NSWindowStyleMask::NonactivatingPanel,
            NSBackingStoreType::Buffered,
            false,
        );
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(false);
        panel.setIgnoresMouseEvents(true);
        panel.setHidesOnDeactivate(false);
        panel.setBecomesKeyOnlyIfNeeded(true);
        panel.setLevel(NSPopUpMenuWindowLevel + 1);
        panel.setCollectionBehavior(
            Behavior::CanJoinAllSpaces
                | Behavior::Stationary
                | Behavior::FullScreenAuxiliary
                | Behavior::IgnoresCycle,
        );
        let view = panel.contentView().expect("NSPanel content view");
        view.setWantsLayer(true);
        let circle = CALayer::layer();
        let rgba = NSColor::colorWithSRGBRed_green_blue_alpha(
            color.r as f64 / 255.,
            color.g as f64 / 255.,
            color.b as f64 / 255.,
            color.a as f64 / 255.,
        );
        if filled {
            circle.setBackgroundColor(Some(&rgba.CGColor()));
        } else {
            circle.setBorderWidth(3.0);
            circle.setBorderColor(Some(&rgba.CGColor()));
        }
        view.layer()
            .expect("layer-backed NSView")
            .addSublayer(&circle);
        Self {
            panel,
            circle,
            side,
        }
    }

    fn origin(point: NSPoint, side: f64) -> NSPoint {
        NSPoint::new(point.x - side / 2., point.y - side / 2.)
    }

    pub fn place(&self, point: NSPoint) {
        self.panel.setFrameOrigin(Self::origin(point, self.side));
    }

    pub fn draw(&self, fraction: f64, opacity: f32) {
        let diameter = fraction * self.side;
        let inset = (self.side - diameter) / 2.;
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        self.circle.setFrame(NSRect::new(
            NSPoint::new(inset, inset),
            NSSize::new(diameter, diameter),
        ));
        self.circle.setCornerRadius(diameter / 2.);
        self.circle.setOpacity(opacity);
        CATransaction::commit();
    }

    pub fn show(&self) {
        self.panel.orderFrontRegardless();
    }
}

impl Drop for Indicator {
    fn drop(&mut self) {
        self.panel.orderOut(None);
    }
}
