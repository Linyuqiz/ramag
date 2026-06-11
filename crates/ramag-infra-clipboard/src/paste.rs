//! 粘贴模拟：辅助功能权限检测 + 激活目标应用 + CGEvent 发 cmd-V

use cocoa::base::{id, nil};
use cocoa::foundation::NSArray;
use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use objc::{class, msg_send, sel, sel_impl};
use tracing::warn;

use crate::pasteboard::ns_string;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
    static kAXTrustedCheckOptionPrompt: CFStringRef;
}

/// prompt=true 时系统会弹「辅助功能」授权引导窗
pub(crate) fn accessibility_trusted(prompt: bool) -> bool {
    unsafe {
        if !prompt {
            return AXIsProcessTrusted();
        }
        let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
        let dict = CFDictionary::from_CFType_pairs(&[(
            key.as_CFType(),
            CFBoolean::true_value().as_CFType(),
        )]);
        AXIsProcessTrustedWithOptions(dict.as_concrete_TypeRef())
    }
}

/// 激活 bundle_id 对应的运行中应用；未运行返回 false
pub(crate) fn activate_app(bundle_id: &str) -> bool {
    unsafe {
        let arr: id = msg_send![class!(NSRunningApplication),
            runningApplicationsWithBundleIdentifier: ns_string(bundle_id)];
        if arr == nil || NSArray::count(arr) == 0 {
            return false;
        }
        let app: id = NSArray::objectAtIndex(arr, 0);
        // NSApplicationActivateIgnoringOtherApps（已软废弃但行为可靠）
        msg_send![app, activateWithOptions: 1u64 << 1]
    }
}

/// 后台线程延迟发 cmd-V：等待激活切换到位，且不阻塞主线程
pub(crate) fn post_cmd_v_delayed(delay_ms: u64) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        post_cmd_v();
    });
}

fn post_cmd_v() {
    // kVK_ANSI_V = 9
    let Ok(src) = CGEventSource::new(CGEventSourceStateID::CombinedSessionState) else {
        warn!("create CGEventSource failed");
        return;
    };
    for down in [true, false] {
        match CGEvent::new_keyboard_event(src.clone(), 9, down) {
            Ok(ev) => {
                ev.set_flags(CGEventFlags::CGEventFlagCommand);
                ev.post(CGEventTapLocation::HID);
            }
            Err(()) => warn!(down, "create cmd-v keyboard event failed"),
        }
    }
}
