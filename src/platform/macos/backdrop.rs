use std::cell::RefCell;

use eframe::egui;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameVibrantDark, NSView,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
    NSWindowCollectionBehavior, NSWindowOrderingMode,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

thread_local! {
    static CARD_BACKDROP: RefCell<Option<Retained<NSVisualEffectView>>> =
        const { RefCell::new(None) };
}

// Mission Control does not affect a Stationary window, so it stays visible and in place.
// proof: https://developer.apple.com/documentation/appkit/nswindow/collectionbehavior-swift.struct/stationary
const POPUP_COLLECTION_BEHAVIOR: NSWindowCollectionBehavior =
    NSWindowCollectionBehavior::CanJoinAllSpaces
        .union(NSWindowCollectionBehavior::FullScreenAuxiliary)
        .union(NSWindowCollectionBehavior::Stationary)
        .union(NSWindowCollectionBehavior::IgnoresCycle);

// AppKit draws a view before its subviews, so a subview of `content` lands in front of
// what `content` draws; the blur is a sibling ordered below `content` instead.
// proof: https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/CocoaViewsGuide/WorkingWithAViewHierarchy/WorkingWithAViewHierarchy.html
pub fn sync_card_backdrop(ctx: &egui::Context, frame: &eframe::Frame, card: egui::Rect) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Ok(handle) = frame.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return;
    };
    let content: &NSView = unsafe { &*appkit.ns_view.as_ptr().cast::<NSView>() };
    let Some(parent) = (unsafe { content.superview() }) else {
        return;
    };

    if let Some(window) = content.window() {
        window.setHasShadow(true);
        if unsafe { window.collectionBehavior() } != POPUP_COLLECTION_BEHAVIOR {
            unsafe { window.setCollectionBehavior(POPUP_COLLECTION_BEHAVIOR) };
        }
    }

    // winit's content view is flipped: origin at the top-left, y down, as in egui.
    // proof: https://docs.rs/crate/winit/0.30.13/source/src/platform_impl/macos/view.rs
    let zoom = f64::from(ctx.zoom_factor());
    let in_content = NSRect::new(
        NSPoint::new(f64::from(card.min.x) * zoom, f64::from(card.min.y) * zoom),
        NSSize::new(
            f64::from(card.width()) * zoom,
            f64::from(card.height()) * zoom,
        ),
    );
    let target = content.convertRect_toView(in_content, Some(&parent));

    CARD_BACKDROP.with_borrow_mut(|slot| {
        let view = slot.get_or_insert_with(|| unsafe {
            let view = NSVisualEffectView::new(mtm);
            view.setMaterial(NSVisualEffectMaterial::HUDWindow);
            view.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
            // Following the window's active state, the view draws its inactive material
            // whenever the window is not active.
            // proof: https://developer.apple.com/documentation/appkit/nsvisualeffectview/state-swift.enum/followswindowactivestate
            view.setState(NSVisualEffectState::Active);
            view.setAppearance(
                NSAppearance::appearanceNamed(NSAppearanceNameVibrantDark).as_deref(),
            );
            view.setWantsLayer(true);
            if let Some(layer) = view.layer() {
                layer.setCornerRadius(f64::from(crate::app::CARD_RADIUS));
                layer.setMasksToBounds(true);
            }
            parent.addSubview_positioned_relativeTo(
                &view,
                NSWindowOrderingMode::NSWindowBelow,
                Some(content),
            );
            view
        });

        view.setHidden(false);
        if view.frame() != target {
            unsafe { view.setFrame(target) };
        }
    });
}

pub(super) fn hide_card_backdrop() {
    if MainThreadMarker::new().is_none() {
        return;
    }
    CARD_BACKDROP.with_borrow(|slot| {
        if let Some(view) = slot {
            view.setHidden(true);
        }
    });
}
