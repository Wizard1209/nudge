// While an NSStatusItem has a menu set, its button's click action is not used, so the menu
// is attached only for the duration of its tracking.
// proof: https://developer.apple.com/documentation/appkit/nsstatusitem/menu

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

pub const STATUS_ICON_POINTS: f64 = 18.0;

const DENSITIES: [u32; 2] = [1, 2];

pub struct StatusItem {
    item: Retained<NSStatusItem>,
    button: Retained<NSStatusBarButton>,
    // A control does not retain its target.
    // proof: https://developer.apple.com/library/archive/documentation/General/Conceptual/CocoaEncyclopedia/Target-Action/Target-Action.html
    _target: Retained<ClickTarget>,
}

impl StatusItem {
    pub fn new(mtm: MainThreadMarker, menu: &muda::Menu) -> Result<StatusItem, String> {
        let ns_menu = unsafe { Retained::retain(menu.ns_menu().cast::<NSMenu>()) }
            .ok_or_else(|| "the menu has no NSMenu behind it".to_string())?;

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
        this.set_frame(daisy::PETAL_COUNT, None);
        this.set_tooltip("Nudge");
        Ok(this)
    }

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
        unsafe {
            self.button.setTarget(None);
            NSStatusBar::systemStatusBar().removeStatusItem(&self.item);
        }
    }
}

pub fn status_image(petals_remaining: u8, drift: Option<DriftState>) -> Retained<NSImage> {
    let points = NSSize::new(STATUS_ICON_POINTS, STATUS_ICON_POINTS);
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

fn bitmap_rep(size: u32, rgba: &[u8]) -> Retained<NSBitmapImageRep> {
    const BYTES_PER_PIXEL: u32 = 4;
    assert_eq!(
        rgba.len(),
        (size * size * BYTES_PER_PIXEL) as usize,
        "a {size}x{size} RGBA buffer"
    );
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

struct ClickTargetIvars {
    item: Retained<NSStatusItem>,
    menu: Retained<NSMenu>,
    showing_menu: Cell<bool>,
}

declare_class!(
    struct ClickTarget;

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
        unsafe { msg_send_id![super(this), init] }
    }

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

    fn reps(image: &NSImage) -> Vec<(isize, isize, NSSize)> {
        let reps = unsafe { image.representations() };
        (0..reps.count())
            .map(|i| unsafe {
                let rep = reps.objectAtIndex(i);
                (rep.pixelsWide(), rep.pixelsHigh(), rep.size())
            })
            .collect()
    }

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
