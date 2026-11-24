//! macOS hands files to an application (double-click in Finder, "Open With", dropping a
//! file on the Dock icon) by calling `application:openURLs:` on the application's delegate,
//! not on the command line.
//!
//! The delegate belongs to winit, which must stay in charge of it, so nothing here touches
//! the delegate or the event loop. Instead the method is added to `NSObject`, the root class
//! every delegate inherits from; the application then finds it on whatever delegate it has.

use objc2::ffi;
use objc2::runtime::{AnyClass, AnyObject, Sel};
use objc2::sel;
use objc2_foundation::{NSArray, NSURL};

/// `- (void)application:(NSApplication *)application openURLs:(NSArray<NSURL *> *)urls`
extern "C" fn open_urls(_this: &AnyObject, _cmd: Sel, _application: &AnyObject, urls: &NSArray<NSURL>) {
    for i in 0..urls.count() {
        let url = unsafe { urls.objectAtIndex(i) };
        if let Some(path) = unsafe { url.path() } {
            super::push(std::path::PathBuf::from(path.to_string()));
        }
    }
}

/// Make the method available. Call once at start-up, before the window is created.
pub fn install() {
    let Some(class) = AnyClass::get("NSObject") else { return };
    let imp: unsafe extern "C" fn() = unsafe {
        std::mem::transmute(open_urls as extern "C" fn(&AnyObject, Sel, &AnyObject, &NSArray<NSURL>))
    };
    // "v@:@@": returns nothing; takes self, the selector and two objects.
    // Does nothing if the method already exists.
    unsafe {
        ffi::class_addMethod(
            class as *const AnyClass as *mut ffi::objc_class,
            sel!(application:openURLs:).as_ptr(),
            Some(imp),
            c"v@:@@".as_ptr(),
        );
    }
}
