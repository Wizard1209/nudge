//! Frosted glass behind the card (spec §2): an `NSVisualEffectView` kept
//! under the card while the popup is up and hidden with it. The panel is
//! this module's alone; the backend reaches it through
//! [`sync_card_backdrop`] each frame and [`hide_card_backdrop`] on close.

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
    /// The one backdrop panel, kept across frames. `Retained` is `!Send`, and
    /// AppKit views are main-thread-only anyway — the same reason the
    /// backend's `UI` state is thread-local.
    static CARD_BACKDROP: RefCell<Option<Retained<NSVisualEffectView>>> =
        const { RefCell::new(None) };
}

/// How the popup window behaves toward Spaces, Mission Control and the
/// desktop. A window above the normal level is implicitly `Transient` unless
/// told otherwise, and transient windows are what "click wallpaper to reveal
/// desktop" sweeps off the screen.
///
/// * `CanJoinAllSpaces` — the popup appears on whichever Space is current,
///   rather than switching the user back to the one it was created on.
/// * `FullScreenAuxiliary` — it may appear over another app's full-screen
///   window.
/// * `Stationary` — reveal-desktop and Mission Control leave it in place.
/// * `IgnoresCycle` — never a target of the ⌘` window cycle.
const POPUP_COLLECTION_BEHAVIOR: NSWindowCollectionBehavior =
    NSWindowCollectionBehavior::CanJoinAllSpaces
        .union(NSWindowCollectionBehavior::FullScreenAuxiliary)
        .union(NSWindowCollectionBehavior::Stationary)
        .union(NSWindowCollectionBehavior::IgnoresCycle);

/// Put a real blur under the card, and keep it there.
///
/// Spec §2 asks for frosted glass: what is behind the window shows through
/// *blurred*. egui cannot do that — it draws into a texture and has no access
/// to the pixels behind the window — so the blur has to come from AppKit, and
/// `NSVisualEffectView` with `BehindWindow` blending is the only thing that
/// samples the desktop behind a window.
///
/// The panel is a **sibling** of winit's content view, ordered below it, not a
/// child of it. That is not a preference: glutin binds the GL context to the
/// content view itself rather than to a subview, and an AppKit subview always
/// composites over its superview's own drawing — so a child would frost the
/// card's text instead of what is behind the window.
///
/// Every step degrades to "no blur" rather than panicking. The parent here is
/// the window's frame view, whose shape is AppKit's business and not a
/// documented contract, so a future layout that this code does not recognise
/// must leave a working popup behind.
pub fn sync_card_backdrop(ctx: &egui::Context, frame: &eframe::Frame, card: egui::Rect) {
    // Off the main thread there is no AppKit to talk to. `tests/kittest_card.rs`
    // drives the real `NudgeApp` from a libtest worker, and it must not crash.
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Ok(handle) = frame.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return;
    };
    // SAFETY: the handle is winit's own content view, alive for as long as the
    // window is, and we are on the thread that owns it.
    let content: &NSView = unsafe { &*appkit.ns_view.as_ptr().cast::<NSView>() };
    let Some(parent) = (unsafe { content.superview() }) else {
        return;
    };

    // The shadow is AppKit's here, and egui draws none (`CARD_SHADOW`). The
    // window's alpha mask is then the card and nothing else, so the system
    // shadow traces the card's own shape. Enabled explicitly rather than left
    // to the default, because the pairing is what makes it correct: with
    // egui's shadow also in that alpha the silhouette is bigger than the card
    // and AppKit outlines *it*, laying a hard rim beside the contour.
    if let Some(window) = content.window() {
        window.setHasShadow(true);
        if unsafe { window.collectionBehavior() } != POPUP_COLLECTION_BEHAVIOR {
            unsafe { window.setCollectionBehavior(POPUP_COLLECTION_BEHAVIOR) };
        }
    }

    // egui points → AppKit points. Only the zoom factor separates them:
    // AppKit points already carry the display's backing scale, so this is the
    // identity at the default zoom on Retina and non-Retina alike.
    //
    // The rect is built in content-view space — which winit makes flipped, so
    // it is top-left-origin exactly like egui's — and AppKit performs the flip
    // into the unflipped parent. Doing the arithmetic by hand instead is how
    // the card ends up mirrored about the window's horizontal centre.
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
            // Not the default `FollowsWindowActiveState`: this is a menu-bar
            // agent that is almost never the frontmost app, and following the
            // window state would render the inactive, washed-out material
            // nearly every time the popup appears.
            view.setState(NSVisualEffectState::Active);
            // The card is dark whatever the system theme is (`set_theme(Dark)`
            // in `NudgeApp::new`), so the material behind it has to be too —
            // otherwise a light-mode Mac frosts it pale under a dark card.
            view.setAppearance(
                NSAppearance::appearanceNamed(NSAppearanceNameVibrantDark).as_deref(),
            );
            view.setWantsLayer(true);
            if let Some(layer) = view.layer() {
                // Clip the material to the card. Circular, matching egui's
                // `CornerRadius` — a continuous curve would visibly diverge
                // from the contour drawn on top of it.
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

/// Take the blur away with the popup. Called by `set_popup_visible(false)`,
/// which is the app's single "the popup is going away" edge.
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
