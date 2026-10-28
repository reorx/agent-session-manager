//! In-app updates through Sparkle 2, plus the standard About panel that shows the installed version.
//!
//! Sparkle.framework ships only inside the release bundle (`scripts/release/bundle-app.sh`), so it is
//! loaded at runtime instead of linked: the bare debug binary and the tests run without it and get no
//! updater and no "Check for Updates…" item. Sparkle reads everything else from the bundle's Info.plist
//! (feed, public key, a check once a day, always ask before installing) and draws its own AppKit windows.
//! Its "Install and Relaunch" quits through `NSApp terminate:`, the same path as our Quit.

use gpui_kit::{App, Global};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2::{class, msg_send};
use objc2_foundation::{NSBundle, NSString};

gpui_kit::actions!(updater, [About, CheckForUpdates]);

/// Sparkle's `SPUStandardUpdaterController`, kept for the life of the app.
pub struct Updater {
    controller: Retained<AnyObject>,
}

impl Global for Updater {}

impl Updater {
    /// Starts Sparkle's scheduled checks, or `None` when this process is not a bundle carrying Sparkle.framework.
    pub fn start() -> Option<Self> {
        let frameworks = NSBundle::mainBundle().privateFrameworksPath()?;
        let framework = NSBundle::bundleWithPath(&NSString::from_str(&format!("{frameworks}/Sparkle.framework")))?;
        // SAFETY: the framework is signed by our team; hardened runtime refuses to load anything else.
        if !unsafe { framework.load() } {
            return None;
        }
        let class: &AnyClass = framework.classNamed(&NSString::from_str("SPUStandardUpdaterController"))?;
        // SAFETY: -[SPUStandardUpdaterController initWithStartingUpdater:updaterDelegate:userDriverDelegate:],
        // both delegates optional. We are on the main thread, inside the running NSApplication.
        let controller: Option<Retained<AnyObject>> = unsafe {
            let allocated: Allocated<AnyObject> = msg_send![class, alloc];
            msg_send![allocated, initWithStartingUpdater: Bool::YES, updaterDelegate: None::<&AnyObject>, userDriverDelegate: None::<&AnyObject>]
        };
        Some(Self { controller: controller? })
    }

    /// Sparkle's own "Check for Updates…": progress, result and install prompts are its windows.
    pub fn check_for_updates(&self) {
        // SAFETY: -[SPUStandardUpdaterController checkForUpdates:] takes an ignored sender.
        unsafe {
            let _: () = msg_send![&*self.controller, checkForUpdates: None::<&AnyObject>];
        }
    }
}

/// Registers About and, when the bundle carries Sparkle, starts it and registers Check for Updates.
/// Returns whether the app menu should offer "Check for Updates…".
pub fn install(cx: &mut App) -> bool {
    cx.on_action(|_: &About, _| show_about_panel());
    let Some(updater) = Updater::start() else {
        println!("updater: off (not running from a bundle with Sparkle.framework)");
        return false;
    };
    cx.set_global(updater);
    cx.on_action(|_: &CheckForUpdates, cx| cx.global::<Updater>().check_for_updates());
    true
}

/// The standard About panel: icon, name and version from Info.plist.
fn show_about_panel() {
    // SAFETY: plain AppKit calls on the main thread, where GPUI dispatches actions.
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, orderFrontStandardAboutPanel: None::<&AnyObject>];
    }
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use super::*;

    #[test]
    fn no_updater_outside_a_release_bundle() {
        assert!(Updater::start().is_none());
    }
}
