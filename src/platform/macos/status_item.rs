//! The menu-bar status item, owned directly rather than through a crate.
//!
//! # Why not `tray-icon`
//!
//! The daisy is drawn per display density (spec §5): an 18 px bitmap for a
//! 1x bar and a 36 px one for a 2x bar, both declared 18 pt, so AppKit picks
//! the bitmap that matches the screen and draws it pixel for pixel. That
//! needs an `NSImage` with two representations. `tray-icon` takes *one*
//! bitmap, PNG-encodes it, builds an `NSImage` from the file and `setSize`s
//! it to 18 pt — so whichever density is handed over, the other one is a
//! resample, and the `NSStatusItem` it sizes is private. The honest variant
//! is to own the item. The menu still comes from `muda`, which is what
//! `tray-icon` builds its menus on; only the item and its image are ours.
//!
//! # Click routing
//!
//! The item's button gets one target/action, sent on both left and right
//! mouse-up. The action reads `NSApplication.currentEvent` to tell them
//! apart: a right-up (or a control-click) shows the menu, a plain left-up
//! summons the popup through `super::request_manual_open` — the Windows
//! tray's gestures, unchanged. The menu is attached to the item only for the
//! duration of its tracking: with a permanent `menu` AppKit swallows every
//! click into it and the action never fires.

use std::cell::Cell;

use muda::ContextMenu;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{ClassType, DeclaredClass, declare_class, msg_send_id, mutability, sel};
use objc2_app_kit::{
    NSApplication, NSBitmapFormat, NSBitmapImageRep, NSDeviceRGBColorSpace, NSEventMask,
    NSEventModifierFlags, NSEventType, NSImage, NSMenu, NSStatusBar, NSStatusBarButton,
    NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::{MainThreadMarker, NSSize, NSString};

use crate::daisy::{self, DriftState};

/// The menu bar's icon size, in points. Every status-item image is this
/// square whatever the display density; the density only picks which bitmap
/// fills it.
pub const STATUS_ICON_POINTS: f64 = 18.0;

/// The backing scales a bitmap is drawn for: 1x and 2x, the two densities a
/// Mac menu bar comes in.
const DENSITIES: [u32; 2] = [1, 2];

/// A live status item. Removing it from the bar is what dropping it does.
pub struct StatusItem {
    item: Retained<NSStatusItem>,
    button: Retained<NSStatusBarButton>,
    /// AppKit does not retain a control's target, so the click target lives
    /// exactly as long as the item does.
    _target: Retained<ClickTarget>,
}

impl StatusItem {
    /// Put the daisy in the menu bar with `menu` as its right-click menu.
    /// **Main thread only**, and only where an `NSApplication` exists — the
    /// eframe creator closure, the precondition `start_on_event_loop`
    /// establishes.
    ///
    /// `menu` stays the caller's: the item retains the `NSMenu` behind it,
    /// and `muda` keeps routing its item clicks to `MenuEvent` regardless of
    /// who shows the menu.
    pub fn new(mtm: MainThreadMarker, menu: &muda::Menu) -> Result<StatusItem, String> {
        // SAFETY: `ns_menu` is the `NSMenu` muda built this menu on, retained
        // by muda for the menu's lifetime; the cast is between two views of
        // the same Objective-C object.
        let ns_menu = unsafe { Retained::retain(menu.ns_menu().cast::<NSMenu>()) }
            .ok_or_else(|| "the menu has no NSMenu behind it".to_string())?;

        // SAFETY: main thread (`mtm`), `NSApplication` is up.
        let item = unsafe {
            NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength)
        };
        let Some(button) = (unsafe { item.button(mtm) }) else {
            unsafe { NSStatusBar::systemStatusBar().removeStatusItem(&item) };
            return Err("the status item has no button".to_string());
        };

        let target = ClickTarget::new(mtm, item.clone(), ns_menu);
        let target_obj: &AnyObject = &target;
        unsafe {
            button.setTarget(Some(target_obj));
            button.setAction(Some(sel!(statusItemClicked:)));
            button.sendActionOn(NSEventMask::LeftMouseUp | NSEventMask::RightMouseUp);
        }

        let this = StatusItem {
            item,
            button,
            _target: target,
        };
        // Refreshed by `tick` as soon as a timer is armed. Until then the
        // launch popup is up, so nobody reads this placeholder.
        this.set_frame(daisy::PETAL_COUNT, None);
        this.set_tooltip("Nudge");
        Ok(this)
    }

    /// Draw one frame of the countdown.
    pub fn set_frame(&self, petals_remaining: u8, drift: Option<DriftState>) {
        let image = status_image(petals_remaining, drift);
        unsafe { self.button.setImage(Some(&image)) };
    }

    pub fn set_tooltip(&self, text: &str) {
        unsafe { self.button.setToolTip(Some(&NSString::from_str(text))) };
    }
}

impl Drop for StatusItem {
    fn drop(&mut self) {
        // SAFETY: on the thread that created the item, which is the only one
        // that can hold a `StatusItem` (it is `!Send`).
        unsafe {
            self.button.setTarget(None);
            NSStatusBar::systemStatusBar().removeStatusItem(&self.item);
        }
    }
}

/// One frame of the daisy as the `NSImage` the status item shows: 18 pt
/// square, with one bitmap per density in [`DENSITIES`] — each rendered at
/// its own pixel size by [`daisy::render`] and declared 18 pt, so AppKit
/// selects by backing scale and nothing is resampled.
///
/// Not a template image: the daisy is the one coloured thing in the app.
pub fn status_image(petals_remaining: u8, drift: Option<DriftState>) -> Retained<NSImage> {
    let points = NSSize::new(STATUS_ICON_POINTS, STATUS_ICON_POINTS);
    // SAFETY: plain AppKit object construction; `NSImage` is not
    // main-thread-only.
    let image = unsafe { NSImage::initWithSize(NSImage::alloc(), points) };
    for scale in DENSITIES {
        let pixels = STATUS_ICON_POINTS as u32 * scale;
        let rep = bitmap_rep(pixels, &daisy::render(pixels, petals_remaining, drift));
        unsafe {
            rep.setSize(points);
            image.addRepresentation(&rep);
        }
    }
    image
}

/// An `NSBitmapImageRep` over a `size`×`size` straight-alpha RGBA buffer.
///
/// The daisy's buffer is *not* premultiplied (`render` demultiplies what
/// resvg draws), and the rep says so with
/// `NSBitmapFormat::AlphaNonpremultiplied`: handed over as premultiplied,
/// AppKit would read a half-covered pink edge pixel as a darker pink and
/// the petal tips would come out fringed.
fn bitmap_rep(size: u32, rgba: &[u8]) -> Retained<NSBitmapImageRep> {
    const BYTES_PER_PIXEL: u32 = 4;
    assert_eq!(
        rgba.len(),
        (size * size * BYTES_PER_PIXEL) as usize,
        "a {size}x{size} RGBA buffer"
    );
    // SAFETY: with `planes` null the rep allocates its own buffer of
    // `bytesPerRow * height`, which is exactly the size copied into it below.
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bitmapFormat_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            size as isize,
            size as isize,
            8,
            4,
            true,
            false,
            NSDeviceRGBColorSpace,
            NSBitmapFormat::AlphaNonpremultiplied,
            (size * BYTES_PER_PIXEL) as isize,
            32,
        )
    }
    .expect("NSBitmapImageRep refused an 8-bit RGBA layout");
    unsafe { std::ptr::copy_nonoverlapping(rgba.as_ptr(), rep.bitmapData(), rgba.len()) };
    rep
}

// ---------- the click target -----------------------------------------------

struct ClickTargetIvars {
    item: Retained<NSStatusItem>,
    menu: Retained<NSMenu>,
    /// Set while the menu is being tracked, so a click the tracking itself
    /// synthesises cannot re-enter and stack a second menu.
    showing_menu: Cell<bool>,
}

declare_class!(
    struct ClickTarget;

    // SAFETY:
    // - `NSObject` has no subclassing requirements.
    // - `MainThreadOnly`: it is reached from the status item's button, which
    //   is main-thread-only, and it holds main-thread-only objects.
    // - `ClickTarget` does not implement `Drop`.
    unsafe impl ClassType for ClickTarget {
        type Super = NSObject;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "NudgeStatusItemTarget";
    }

    impl DeclaredClass for ClickTarget {
        type Ivars = ClickTargetIvars;
    }

    unsafe impl NSObjectProtocol for ClickTarget {}

    unsafe impl ClickTarget {
        #[method(statusItemClicked:)]
        fn status_item_clicked(&self, _sender: Option<&AnyObject>) {
            self.clicked();
        }
    }
);

impl ClickTarget {
    fn new(
        mtm: MainThreadMarker,
        item: Retained<NSStatusItem>,
        menu: Retained<NSMenu>,
    ) -> Retained<Self> {
        let this = mtm.alloc().set_ivars(ClickTargetIvars {
            item,
            menu,
            showing_menu: Cell::new(false),
        });
        // SAFETY: `NSObject`'s `init`, on an allocation with its ivars set.
        unsafe { msg_send_id![super(this), init] }
    }

    /// The one action. Which gesture it was is in the event AppKit is
    /// dispatching right now.
    fn clicked(&self) {
        let mtm = MainThreadMarker::from(self);
        let event = NSApplication::sharedApplication(mtm).currentEvent();
        let wants_menu = event.is_some_and(|e| unsafe {
            e.r#type() == NSEventType::RightMouseUp
                || e.modifierFlags()
                    .contains(NSEventModifierFlags::NSEventModifierFlagControl)
        });
        if wants_menu {
            self.show_menu(mtm);
        } else {
            super::request_manual_open();
        }
    }

    /// Drop the menu below the item, with the item highlighted, the way a
    /// menu-bearing status item does it: attach the menu, click the button,
    /// detach once the tracking loop returns.
    fn show_menu(&self, mtm: MainThreadMarker) {
        let ivars = self.ivars();
        if ivars.showing_menu.replace(true) {
            return;
        }
        unsafe {
            ivars.item.setMenu(Some(&ivars.menu));
            if let Some(button) = ivars.item.button(mtm) {
                button.performClick(None);
            }
            ivars.item.setMenu(None);
        }
        ivars.showing_menu.set(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reps of `image`, as `(pixelsWide, pixelsHigh, size)`.
    fn reps(image: &NSImage) -> Vec<(isize, isize, NSSize)> {
        let reps = unsafe { image.representations() };
        (0..reps.count())
            .map(|i| unsafe {
                let rep = reps.objectAtIndex(i);
                (rep.pixelsWide(), rep.pixelsHigh(), rep.size())
            })
            .collect()
    }

    /// The contract the module exists for: one image, 18 pt, with the 1x
    /// and 2x bitmaps both present and both declared 18 pt, so the bar's
    /// density picks a bitmap instead of a resample.
    #[test]
    fn status_image_is_18pt_with_one_bitmap_per_density() {
        let image = status_image(daisy::PETAL_COUNT, None);
        let points = NSSize::new(18.0, 18.0);
        assert_eq!(unsafe { image.size() }, points);

        let mut reps = reps(&image);
        reps.sort_by_key(|(w, _, _)| *w);
        assert_eq!(reps.len(), 2, "exactly one rep per density: {reps:?}");
        assert_eq!(
            reps.iter().map(|(w, h, _)| (*w, *h)).collect::<Vec<_>>(),
            vec![(18, 18), (36, 36)],
            "pixel sizes: {reps:?}"
        );
        for (_, _, size) in &reps {
            assert_eq!(*size, points, "every rep is declared 18 pt: {reps:?}");
        }
    }

    /// The alpha contract, observed on the bitmap AppKit holds: the daisy's
    /// bytes are straight alpha, so the rep must say `AlphaNonpremultiplied`
    /// and must carry the bytes untouched. The probe pixel is one where a
    /// colour channel exceeds the alpha — legal straight alpha, impossible
    /// premultiplied — so a rep that premultiplied on the way in, or was
    /// declared premultiplied and "fixed" by AppKit, could not hold it.
    #[test]
    fn the_2x_rep_holds_the_daisys_straight_alpha_bytes_untouched() {
        let size = 36;
        let rgba = daisy::render(size, daisy::PETAL_COUNT, None);
        let probe = rgba
            .chunks_exact(4)
            .position(|px| px[3] > 0 && px[3] < 255 && px[..3].iter().any(|c| *c > px[3]))
            .expect("the daisy has an edge pixel with colour above its alpha");

        let rep = bitmap_rep(size, &rgba);
        unsafe {
            assert!(rep.hasAlpha());
            assert!(
                rep.bitmapFormat()
                    .contains(NSBitmapFormat::AlphaNonpremultiplied),
                "the rep must declare straight alpha, got {:?}",
                rep.bitmapFormat()
            );
            assert_eq!(rep.bytesPerRow(), (size * 4) as isize);
            let held = std::slice::from_raw_parts(rep.bitmapData(), rgba.len());
            assert_eq!(
                &held[probe * 4..probe * 4 + 4],
                &rgba[probe * 4..probe * 4 + 4],
                "edge pixel {probe} must come back as the straight bytes it went in as"
            );
            assert_eq!(held, &rgba[..], "the whole bitmap is the daisy's buffer");
        }
    }
}
