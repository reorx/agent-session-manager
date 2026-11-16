//! The menu bar item that keeps the app around once its window is closed: an icon and today's
//! cost in the menu bar, a menu with today's numbers, the last scan, Open Sessions, Scan Now and
//! Quit. Closing the window only hides it (`AppRoot::close_requested`), so the store thread,
//! the automatic scans and everything on screen stay as they were.
//!
//! Neither GPUI nor gpui-component wraps `NSStatusItem`, so the item is built here with
//! `objc2-app-kit`. Its menu items' target (`MenuTarget`, a `define_class!` NSObject) only sends a
//! `ResidentCommand` into a channel; a GPUI task runs it. AppKit callbacks never touch GPUI's
//! `App`, which may be borrowed when they run. `install` runs in `main()` only: tests have no
//! `Resident` global, so nothing here runs under them (and the test window has no NSWindow).
//!
//! Hiding is `orderOut:` on the one window plus the Accessory activation policy (no Dock icon, not
//! in ⌘Tab); showing switches back to Regular, then GPUI activates the app and the window. Both
//! orders were tried 10/10 in the usage-p2-probe experiment. A click on Open Sessions and
//! opening the app again from Finder also ask for the front at once (`bring_forward`): GPUI's
//! activation alone was refused while a game was in front (usage-p2-accept).

use std::cell::Cell;
use std::time::Instant;

use agent_session_core::format::{fmt_tokens, fmt_usd};
use agent_session_core::usage::Totals;
use async_channel::Sender;
use chrono::NaiveTime;
use gpui_kit::{App, Global, Window};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSCellImagePosition, NSImage, NSMenu, NSMenuDelegate, NSMenuItem, NSScreen, NSStatusBar, NSStatusItem,
    NSVariableStatusItemLength, NSView, NSWindow,
};
use objc2_foundation::{NSPoint, NSString};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// The SF Symbol in the menu bar; a template image, so it follows light and dark menu bars.
const SYMBOL: &str = "chart.bar.xaxis";
/// Lets macOS (and menu bar managers like Thaw) remember where the user put the item.
const AUTOSAVE_NAME: &str = "AgentSessionManagerToday";

/// What a menu item asks the app to do. A menu item's target sends it over a channel; a GPUI
/// task hands it to `AppRoot::on_resident_command`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResidentCommand {
    OpenSessions,
    ScanNow,
    Quit,
}

/// The menu bar button's title, the menu's three read-only lines and whether Scan Now is
/// greyed out; `AppRoot::sync_resident` pushes it to the menu bar item when it changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResidentView {
    pub title: String,
    pub today: String,
    pub tokens: String,
    pub scanned: String,
    pub scanning: bool,
}

impl ResidentView {
    /// `today`: `None` without a usage ledger (or before it was read).
    pub fn new(today: Option<&Totals>, scans: &ScanRecord, scanning: bool) -> Self {
        ResidentView {
            title: title(today.map(|t| t.cost.total())),
            today: menu_today(today),
            tokens: menu_tokens(today),
            scanned: menu_scanned(scanning, scans),
            scanning,
        }
    }
}

/// `$204`, `$1,204`: today's cost in whole dollars. Empty without a ledger: only the icon shows.
pub fn title(today_usd: Option<f64>) -> String {
    match today_usd {
        Some(usd) => format!("${}", thousands(usd.max(0.0).round() as u64)),
        None => String::new(),
    }
}

/// `1,234,567`
fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (ix, digit) in digits.chars().enumerate() {
        if ix > 0 && (digits.len() - ix) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// `Today  $203.69`, with ` (2 unpriced)` when the status bar has it too; `Today  —` without a
/// ledger.
pub fn menu_today(today: Option<&Totals>) -> String {
    let Some(today) = today else { return "Today  —".into() };
    let unpriced = match today.unpriced_responses {
        0 => String::new(),
        n => format!(" ({n} unpriced)"),
    };
    format!("Today  {}{unpriced}", fmt_usd(today.cost.total()))
}

/// `In 492.7M · Out 2.4M`: In is every input token (uncached, cache writes, cache reads).
pub fn menu_tokens(today: Option<&Totals>) -> String {
    match today {
        Some(today) => format!("In {} · Out {}", fmt_tokens(today.tokens.input_total()), fmt_tokens(today.tokens.output)),
        None => "In — · Out —".into(),
    }
}

/// How the scans went, for the menu's third line: when the last one went through, and how many
/// in a row since then failed or were skipped (the index or the ledger was locked by `asm`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanRecord {
    pub done_at: Option<NaiveTime>,
    pub failed_at: Option<NaiveTime>,
    pub failures: u32,
}

impl ScanRecord {
    pub fn done(&mut self, at: NaiveTime) {
        self.done_at = Some(at);
        self.failures = 0;
    }

    pub fn failed(&mut self, at: NaiveTime) {
        self.failed_at = Some(at);
        self.failures += 1;
    }
}

/// `Scanning…` while a scan runs; `Scan failed 16:13` once two scans in a row failed (one is
/// retried 10 minutes later anyway) or when none ever went through; else `Scanned 15:53`.
pub fn menu_scanned(scanning: bool, scans: &ScanRecord) -> String {
    let hm = |t: NaiveTime| t.format("%H:%M").to_string();
    if scanning {
        return "Scanning…".into();
    }
    match (scans.done_at, scans.failed_at) {
        (done, Some(failed)) if scans.failures >= 2 || (done.is_none() && scans.failures > 0) => format!("Scan failed {}", hm(failed)),
        (Some(done), _) => format!("Scanned {}", hm(done)),
        _ => "Scanned —".into(),
    }
}

pub struct TargetIvars {
    commands: Sender<ResidentCommand>,
    /// Between `menuWillOpen:` and `menuDidClose:`.
    menu_open: Cell<bool>,
    /// When it opened: the log says how long it stayed open (an autoplay menu closed at once
    /// while a game was in front, usage-p2-accept).
    opened_at: Cell<Option<Instant>>,
    /// Where `popUp:` puts the menu (AppKit screen coordinates, the menu's top-left corner).
    pop_up_at: Cell<NSPoint>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and MenuTarget does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AsmMenuTarget"]
    #[ivars = TargetIvars]
    pub struct MenuTarget;

    impl MenuTarget {
        #[unsafe(method(openSessions:))]
        fn open_sessions(&self, _sender: Option<&AnyObject>) {
            // now, while the click is the latest thing the user did (`bring_forward`)
            bring_forward();
            self.send(ResidentCommand::OpenSessions);
        }

        #[unsafe(method(scanNow:))]
        fn scan_now(&self, _sender: Option<&AnyObject>) {
            self.send(ResidentCommand::ScanNow);
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: Option<&AnyObject>) {
            self.send(ResidentCommand::Quit);
        }

        /// The autoplay's stand-in for a click on the menu bar item (`Resident::pop_up_later`):
        /// menu tracking is a nested run loop, so this runs from the run loop, never inside a
        /// GPUI update.
        #[unsafe(method(popUp:))]
        fn pop_up(&self, menu: &NSMenu) {
            menu.popUpMenuPositioningItem_atLocation_inView(None, self.ivars().pop_up_at.get(), None);
        }
    }

    unsafe impl NSObjectProtocol for MenuTarget {}

    unsafe impl NSMenuDelegate for MenuTarget {
        #[unsafe(method(menuWillOpen:))]
        fn menu_will_open(&self, _menu: &NSMenu) {
            self.ivars().menu_open.set(true);
            self.ivars().opened_at.set(Some(Instant::now()));
            println!("menu bar: menu open");
        }

        #[unsafe(method(menuDidClose:))]
        fn menu_did_close(&self, _menu: &NSMenu) {
            self.ivars().menu_open.set(false);
            let open_for = self.ivars().opened_at.take().map_or(0.0, |at| at.elapsed().as_secs_f64());
            println!("menu bar: menu closed (open {open_for:.2} s)");
        }
    }
);

impl MenuTarget {
    fn new(mtm: MainThreadMarker, commands: Sender<ResidentCommand>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TargetIvars { commands, menu_open: Cell::new(false), opened_at: Cell::new(None), pop_up_at: Cell::new(NSPoint::new(0.0, 0.0)) });
        // SAFETY: NSObject's designated initializer.
        unsafe { msg_send![super(this), init] }
    }

    /// AppKit sends the action after the menu closed (`menuDidClose:` first), so whatever the
    /// command does, no menu is tracking by then.
    fn send(&self, command: ResidentCommand) {
        // unbounded: fails only once the app's task is gone, and then nobody is left to tell
        let _ = self.ivars().commands.try_send(command);
    }
}

/// The menu bar item and its menu, for the life of the app (a GPUI global, like `Updater`).
pub struct Resident {
    /// Released, the item leaves the menu bar.
    item: Retained<NSStatusItem>,
    menu: Retained<NSMenu>,
    today: Retained<NSMenuItem>,
    tokens: Retained<NSMenuItem>,
    scanned: Retained<NSMenuItem>,
    scan_now: Retained<NSMenuItem>,
    /// Menu items hold their target and the menu its delegate weakly: this keeps it alive.
    target: Retained<MenuTarget>,
}

impl Global for Resident {}

/// Menu items by index (`Resident::perform`), in the order `install` adds them.
const OPEN_SESSIONS_ITEM: isize = 4;
const SCAN_NOW_ITEM: isize = 5;
const QUIT_ITEM: isize = 7;

/// Puts the item into the menu bar and stores the `Resident` global. Menu items send their
/// commands to `commands`.
pub fn install(cx: &mut App, commands: Sender<ResidentCommand>) {
    let mtm = MainThreadMarker::new().expect("resident::install runs on the main thread");
    let target = MenuTarget::new(mtm, commands);

    let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
    item.setAutosaveName(Some(&NSString::from_str(AUTOSAVE_NAME)));
    if let Some(button) = item.button(mtm) {
        match NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(SYMBOL), Some(&NSString::from_str("Agent Session Manager"))) {
            Some(image) => {
                image.setTemplate(true);
                button.setImage(Some(&image));
            }
            None => eprintln!("menu bar: no system symbol {SYMBOL}"),
        }
        button.setImagePosition(NSCellImagePosition::ImageLeading);
        button.setTitle(&NSString::from_str(""));
    }

    let menu = NSMenu::new(mtm);
    // otherwise AppKit enables every item with a target and `setEnabled` does nothing
    menu.setAutoenablesItems(false);
    let view = ResidentView::new(None, &ScanRecord::default(), true);
    let today = info_item(mtm, &view.today);
    let tokens = info_item(mtm, &view.tokens);
    let scanned = info_item(mtm, &view.scanned);
    let scan_now = action_item(mtm, "Scan Now", sel!(scanNow:), &target);
    for entry in [
        today.clone(),
        tokens.clone(),
        scanned.clone(),
        NSMenuItem::separatorItem(mtm),
        action_item(mtm, "Open Sessions", sel!(openSessions:), &target),
        scan_now.clone(),
        NSMenuItem::separatorItem(mtm),
        action_item(mtm, "Quit Agent Session Manager", sel!(quit:), &target),
    ] {
        menu.addItem(&entry);
    }
    menu.setDelegate(Some(ProtocolObject::from_ref(&*target)));
    item.setMenu(Some(&menu));
    cx.set_global(Resident { item, menu, today, tokens, scanned, scan_now, target });
}

/// A read-only line: greyed out, does nothing.
fn info_item(mtm: MainThreadMarker, title: &str) -> Retained<NSMenuItem> {
    // SAFETY: no action selector.
    let item = unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &NSString::from_str(title), None, &NSString::from_str("")) };
    item.setEnabled(false);
    item
}

fn action_item(mtm: MainThreadMarker, title: &str, action: Sel, target: &MenuTarget) -> Retained<NSMenuItem> {
    // SAFETY: `action` is one of MenuTarget's methods, which take the sender as their argument.
    let item = unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &NSString::from_str(title), Some(action), &NSString::from_str("")) };
    let target: &AnyObject = target;
    // SAFETY: the target responds to `action`; `Resident` keeps it alive as long as the item.
    unsafe { item.setTarget(Some(target)) };
    item
}

impl Resident {
    /// Takes effect at once, also while the menu is open.
    pub fn update(&self, view: &ResidentView) {
        let mtm = MainThreadMarker::from(&*self.target);
        if let Some(button) = self.item.button(mtm) {
            button.setTitle(&NSString::from_str(&view.title));
        }
        self.today.setTitle(&NSString::from_str(&view.today));
        self.tokens.setTitle(&NSString::from_str(&view.tokens));
        self.scanned.setTitle(&NSString::from_str(&view.scanned));
        self.scan_now.setEnabled(!view.scanning);
    }

    /// Takes this one window off screen (`NSApp hide:` would hide the whole app, Sparkle's
    /// windows too) and the app out of the Dock and ⌘Tab.
    pub fn hide(&self, window: &Window) {
        match ns_window(window) {
            Some(ns_window) => ns_window.orderOut(None),
            None => eprintln!("menu bar: no NSWindow behind the window"),
        }
        self.set_dock_icon(false);
    }

    /// The Dock icon (and the app's menu bar, ⌘Tab) back; the caller then activates the app and
    /// the window. Before the activation, or the menu bar may not come back.
    pub fn show(&self) {
        self.set_dock_icon(true);
    }

    fn set_dock_icon(&self, visible: bool) {
        let mtm = MainThreadMarker::from(&*self.target);
        let policy = if visible { NSApplicationActivationPolicy::Regular } else { NSApplicationActivationPolicy::Accessory };
        set_policy(&NSApplication::sharedApplication(mtm), policy);
    }

    pub fn menu_open(&self) -> bool {
        self.target.ivars().menu_open.get()
    }

    /// Autoplay: open the menu with its top-left corner at `at`, in points from the top-left of
    /// the main screen (cliclick's coordinates), as a click on the item would; by default near
    /// the right end of the menu bar. A menu bar manager may keep the item itself off screen
    /// (Thaw does), so the menu does not open at the item. Runs once the current GPUI update is
    /// over (`performSelector:withObject:afterDelay:`). Returns where it opens.
    pub fn pop_up_later(&self, at: Option<(f64, f64)>) -> (f64, f64) {
        let mtm = MainThreadMarker::from(&*self.target);
        let screen = NSScreen::screens(mtm).firstObject().map(|screen| screen.frame().size);
        let (width, height) = screen.map_or((1440.0, 900.0), |size| (size.width, size.height));
        let (x, y) = at.unwrap_or((width - 420.0, 30.0));
        self.target.ivars().pop_up_at.set(NSPoint::new(x, height - y));
        let menu: &AnyObject = &self.menu;
        // SAFETY: NSObject's performSelector:withObject:afterDelay:; popUp: takes the menu.
        unsafe {
            let _: () = msg_send![&*self.target, performSelector: sel!(popUp:), withObject: menu, afterDelay: 0.0f64];
        }
        (x, y)
    }

    /// Autoplay: close the open menu without choosing anything.
    pub fn close_menu(&self) {
        self.menu.cancelTracking();
    }

    /// Autoplay: what a click on a menu item does (the target sends the command).
    pub fn perform(&self, command: ResidentCommand) {
        self.menu.performActionForItemAtIndex(match command {
            ResidentCommand::OpenSessions => OPEN_SESSIONS_ITEM,
            ResidentCommand::ScanNow => SCAN_NOW_ITEM,
            ResidentCommand::Quit => QUIT_ITEM,
        });
    }
}

/// The app to the front for something the user just asked for: a click on Open Sessions, or
/// opening the app again from Finder (`main()`'s `on_reopen`). Runs right there in the AppKit
/// callback, while the request is fresh: since macOS 14 activation is cooperative, and
/// `NSApp.activate()` is honoured after the user's own input (usage-p2-probe: 1/1 after a real
/// click, 0/4 from a timer). GPUI's `cx.activate(true)` (`activateIgnoringOtherApps:`), which
/// `AppRoot::show_window` still calls next, works without a click while a terminal is in front
/// but was refused while a game was (usage-p2-accept, 3 of 3). The Dock icon comes back first,
/// as in `Resident::show`, so that the activation brings the app's menu bar along.
pub fn bring_forward() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    let was_active = app.isActive();
    set_policy(&app, NSApplicationActivationPolicy::Regular);
    app.activate();
    println!("window: bring forward (active before: {was_active})");
}

fn set_policy(app: &NSApplication, policy: NSApplicationActivationPolicy) {
    // AppKit answers NO when asked for the policy it already has
    if app.activationPolicy() != policy && !app.setActivationPolicy(policy) {
        eprintln!("menu bar: activation policy {policy:?} refused");
    }
}

/// The NSWindow behind a GPUI window: GPUI's AppKit handle is its content view.
fn ns_window(window: &Window) -> Option<Retained<NSWindow>> {
    // fully qualified: GPUI's own `Window::window_handle()` (an AnyWindowHandle) shadows it
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else { return None };
    // SAFETY: the handle's view is GPUI's content view, alive as long as `window` is.
    let view: &NSView = unsafe { appkit.ns_view.cast::<NSView>().as_ref() };
    view.window()
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use agent_session_core::model::Tokens;
    use agent_session_core::pricing::CostLines;

    use super::*;

    fn at(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    fn today(cost: f64, unpriced: u64) -> Totals {
        Totals {
            responses: 812,
            tokens: Tokens { input: 1_200_000, cache_write_5m: 0, cache_write_1h: 7_500_000, cache_read: 484_000_000, output: 2_400_000, web_search: 0 },
            cost: CostLines { input: cost, ..Default::default() },
            unpriced_responses: unpriced,
        }
    }

    #[test]
    fn the_title_is_todays_cost_rounded_to_dollars() {
        assert_eq!(title(Some(203.69)), "$204");
        assert_eq!(title(Some(0.4)), "$0");
        assert_eq!(title(Some(0.0)), "$0");
        assert_eq!(title(Some(999.5)), "$1,000");
        assert_eq!(title(Some(1203.5)), "$1,204");
        assert_eq!(title(Some(1_234_567.2)), "$1,234,567");
        assert_eq!(title(None), "", "without a ledger only the icon shows");
    }

    #[test]
    fn the_menu_lines_show_cost_tokens_and_the_last_scan() {
        let paid = today(203.69, 0);
        assert_eq!(menu_today(Some(&paid)), "Today  $203.69");
        assert_eq!(menu_today(Some(&today(203.69, 2))), "Today  $203.69 (2 unpriced)", "like the status bar");
        assert_eq!(menu_tokens(Some(&paid)), "In 492.7M · Out 2.4M");
        assert_eq!(menu_today(None), "Today  —");
        assert_eq!(menu_tokens(None), "In — · Out —");

        let mut scans = ScanRecord::default();
        assert_eq!(menu_scanned(true, &scans), "Scanning…", "the start-up scan");
        scans.done(at(15, 53));
        assert_eq!(menu_scanned(false, &scans), "Scanned 15:53");
        assert_eq!(menu_scanned(true, &scans), "Scanning…");
        // asm held the index once: the next scan tries again, nothing to tell
        scans.failed(at(16, 3));
        assert_eq!(menu_scanned(false, &scans), "Scanned 15:53");
        scans.failed(at(16, 13));
        assert_eq!(menu_scanned(false, &scans), "Scan failed 16:13", "two in a row");
        assert_eq!(menu_scanned(true, &scans), "Scanning…");
        scans.done(at(16, 23));
        assert_eq!(menu_scanned(false, &scans), "Scanned 16:23", "one that goes through clears the failures");
    }

    #[test]
    fn a_first_scan_that_fails_says_so_at_once() {
        let mut scans = ScanRecord::default();
        assert_eq!(menu_scanned(false, &scans), "Scanned —");
        scans.failed(at(9, 5));
        assert_eq!(menu_scanned(false, &scans), "Scan failed 09:05", "nothing ever went through");
    }

    #[test]
    fn the_view_greys_scan_now_while_scanning_and_shows_only_the_icon_without_a_ledger() {
        let mut scans = ScanRecord::default();
        scans.done(at(15, 53));
        let view = ResidentView::new(Some(&today(203.69, 0)), &scans, false);
        assert_eq!(view, ResidentView { title: "$204".into(), today: "Today  $203.69".into(), tokens: "In 492.7M · Out 2.4M".into(), scanned: "Scanned 15:53".into(), scanning: false });
        let scanning = ResidentView::new(Some(&today(203.69, 0)), &scans, true);
        assert!(scanning.scanning);
        assert_eq!(scanning.scanned, "Scanning…");
        let no_ledger = ResidentView::new(None, &scans, false);
        assert_eq!((no_ledger.title.as_str(), no_ledger.today.as_str()), ("", "Today  —"));
    }
}
