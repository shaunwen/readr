use std::ffi::c_void;

use core_foundation::base::{CFGetTypeID, CFRelease, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::string::{CFString, CFStringRef};

type AXUIElementRef = *const c_void;
const AX_SUCCESS: i32 = 0;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    static kAXTrustedCheckOptionPrompt: CFStringRef;
    fn AXIsProcessTrusted() -> u8;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> u8;
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> i32;
}

pub fn trusted() -> bool {
    unsafe { AXIsProcessTrusted() != 0 }
}

pub fn request_trust() -> bool {
    let key = unsafe { CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt) };
    let options = CFDictionary::from_CFType_pairs(&[(key, CFBoolean::true_value())]);
    unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef()) != 0 }
}

pub fn focused_window_title(pid: i32) -> Option<String> {
    let app = unsafe { AXUIElementCreateApplication(pid) };
    if app.is_null() {
        return None;
    }
    let title = copy_attribute(app, "AXFocusedWindow").and_then(|window| {
        let title = copy_attribute(window, "AXTitle").and_then(|value| {
            if unsafe { CFGetTypeID(value) } != CFString::type_id() {
                unsafe { CFRelease(value) };
                return None;
            }
            let text = unsafe { CFString::wrap_under_create_rule(value as CFStringRef) };
            Some(text.to_string())
        });
        unsafe { CFRelease(window) };
        title
    });
    unsafe { CFRelease(app) };
    title.filter(|title| !title.is_empty())
}

fn copy_attribute(element: AXUIElementRef, name: &str) -> Option<CFTypeRef> {
    let attribute = CFString::new(name);
    let mut value: CFTypeRef = std::ptr::null();
    let status = unsafe {
        AXUIElementCopyAttributeValue(element, attribute.as_concrete_TypeRef(), &mut value)
    };
    (status == AX_SUCCESS && !value.is_null()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accessibility_queries_fail_closed_without_crashing() {
        // given
        let pid = std::process::id() as i32;

        // when
        let trusted = trusted();
        let title = focused_window_title(pid);

        // then
        if !trusted {
            assert_eq!(title, None);
        }
    }
}
