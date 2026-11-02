// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
//! AppKit's standard About panel, including metadata for unbundled cargo runs.

use objc2::{AllocAnyThread, MainThreadMarker, runtime::AnyObject};
use objc2_app_kit::{
    NSAboutPanelOptionApplicationIcon, NSAboutPanelOptionApplicationName,
    NSAboutPanelOptionApplicationVersion, NSAboutPanelOptionCredits, NSAboutPanelOptionVersion,
    NSApplication, NSImage,
};
use objc2_foundation::{NSAttributedString, NSData, NSDictionary, NSString, ns_string};

pub(crate) fn show() -> Result<(), &'static str> {
    let mtm = MainThreadMarker::new().ok_or("AppKit requires the main thread")?;
    let app = NSApplication::sharedApplication(mtm);
    let name = NSString::from_str("Rustfus");
    let version = NSString::from_str(env!("CARGO_PKG_VERSION"));
    let copyright = NSString::from_str("© 2026 Rustfus contributors");
    let credits = NSAttributedString::initWithString(
        NSAttributedString::alloc(),
        &NSString::from_str(
            "The Reliable USB Formatting Utility, rewritten in Rust.\n\n\
             Based on Rufus © 2011-2026 Pete Batard. Not affiliated with Rufus.\n\
             https://rufus.ie\n\n\
             License: GNU General Public License (GPL) version 3 or later.\n\n\
             Includes ms-sys boot records, Syslinux, GRUB 2, FreeDOS, UEFI:NTFS \
             (ntfs-3g, EfiFs), uefi-md5sum and the Rufus translations. \
             See THIRD_PARTY_NOTICES.md.",
        ),
    );
    let data = NSData::with_bytes(include_bytes!("../../../res/rustfus.png"));
    let icon = NSImage::initWithData(NSImage::alloc(), &data)
        .ok_or("the embedded Rustfus icon could not be decoded")?;

    // SAFETY: Each key is paired with AppKit's documented value type:
    // NSString for text, NSAttributedString for credits, NSImage for the icon.
    unsafe {
        let options = NSDictionary::<NSString, AnyObject>::from_slices(
            &[
                NSAboutPanelOptionApplicationName,
                NSAboutPanelOptionApplicationVersion,
                NSAboutPanelOptionVersion,
                ns_string!("Copyright"),
                NSAboutPanelOptionCredits,
                NSAboutPanelOptionApplicationIcon,
            ],
            &[
                (*name).as_ref(),
                (*version).as_ref(),
                // Suppress any host bundle's unrelated build version.
                ns_string!("").as_ref(),
                (*copyright).as_ref(),
                (*credits).as_ref(),
                (*icon).as_ref(),
            ],
        );
        app.orderFrontStandardAboutPanelWithOptions(&options);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn native_about_rejects_non_main_thread_without_opening_a_panel() {
        let result = std::thread::spawn(super::show).join().unwrap();
        assert_eq!(result, Err("AppKit requires the main thread"));
    }
}
