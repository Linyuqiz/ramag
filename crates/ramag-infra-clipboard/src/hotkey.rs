//! 全局热键：Carbon `RegisterEventHotKey` 注册系统级快捷键（cmd-shift-V）。
//! 事件回调在主线程触发，经 mpsc channel 转出，由 main.rs 的计时器轮询消费——
//! 与采集循环同款模式，不引入第三方 global-hotkey 依赖

use std::ffi::c_void;
use std::sync::mpsc::{Receiver, Sender, channel};

use tracing::{info, warn};

// —— Carbon 类型（HIToolbox）——
type OsStatus = i32;
type EventTargetRef = *mut c_void;
type EventHandlerRef = *mut c_void;
type EventHandlerCallRef = *mut c_void;
type EventRef = *mut c_void;
type EventHotKeyRef = *mut c_void;

#[repr(C)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

#[repr(C)]
struct EventHotKeyId {
    signature: u32,
    id: u32,
}

type EventHandlerProc = extern "C" fn(EventHandlerCallRef, EventRef, *mut c_void) -> OsStatus;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetApplicationEventTarget() -> EventTargetRef;
    fn InstallEventHandler(
        target: EventTargetRef,
        handler: EventHandlerProc,
        num_types: u32,
        type_list: *const EventTypeSpec,
        user_data: *mut c_void,
        out_ref: *mut EventHandlerRef,
    ) -> OsStatus;
    fn RegisterEventHotKey(
        key_code: u32,
        modifiers: u32,
        hot_key_id: EventHotKeyId,
        target: EventTargetRef,
        options: u32,
        out_ref: *mut EventHotKeyRef,
    ) -> OsStatus;
    fn UnregisterEventHotKey(hot_key: EventHotKeyRef) -> OsStatus;
    fn RemoveEventHandler(handler: EventHandlerRef) -> OsStatus;
}

// kEventClassKeyboard = 'keyb'，kEventHotKeyPressed = 5
const EVENT_CLASS_KEYBOARD: u32 = u32::from_be_bytes(*b"keyb");
const EVENT_HOTKEY_PRESSED: u32 = 5;
// Carbon 修饰键掩码
const CMD_KEY: u32 = 0x0100;
const SHIFT_KEY: u32 = 0x0200;
// kVK_ANSI_V = 9
const KEY_V: u32 = 9;

/// 热键事件回调：经 user_data 还原 Sender 并发信号。回调全程不 panic（跨 FFI 边界）
extern "C" fn hotkey_handler(
    _next: EventHandlerCallRef,
    _event: EventRef,
    user_data: *mut c_void,
) -> OsStatus {
    if !user_data.is_null() {
        // user_data 指向 Sender 裸指针，仅借用不接管（注销时由 Drop 回收）
        let tx = unsafe { &*(user_data as *const Sender<()>) };
        let _ = tx.send(());
    }
    0
}

/// 热键句柄：持有 Receiver 与 Carbon ref；Drop 时注销热键、移除 handler、回收 Sender。
/// ref 以 usize 存（裸指针非 Send，须能随句柄移入异步轮询任务）
pub struct HotkeyListener {
    rx: Receiver<()>,
    handler_ref: usize,
    hotkey_ref: usize,
    tx_ptr: usize,
}

impl HotkeyListener {
    /// 注册 cmd-shift-V。须在主线程、NSApplication 事件循环就绪后调用
    pub fn register_cmd_shift_v() -> Option<Self> {
        let (tx, rx) = channel::<()>();
        // Sender 转裸指针交给 Carbon 回调；句柄存活期间常驻，注销时由 Drop 回收
        let tx_ptr = Box::into_raw(Box::new(tx)) as *mut c_void;

        unsafe {
            let target = GetApplicationEventTarget();
            let spec = EventTypeSpec {
                event_class: EVENT_CLASS_KEYBOARD,
                event_kind: EVENT_HOTKEY_PRESSED,
            };
            let mut handler_ref: EventHandlerRef = std::ptr::null_mut();
            let status =
                InstallEventHandler(target, hotkey_handler, 1, &spec, tx_ptr, &mut handler_ref);
            if status != 0 {
                warn!(status, "InstallEventHandler failed");
                drop(Box::from_raw(tx_ptr as *mut Sender<()>));
                return None;
            }

            let hot_id = EventHotKeyId {
                signature: u32::from_be_bytes(*b"rmag"),
                id: 1,
            };
            let mut hotkey_ref: EventHotKeyRef = std::ptr::null_mut();
            let status = RegisterEventHotKey(
                KEY_V,
                CMD_KEY | SHIFT_KEY,
                hot_id,
                target,
                0,
                &mut hotkey_ref,
            );
            if status != 0 {
                warn!(status, "RegisterEventHotKey failed");
                // 注册失败：回收已装的 handler 与 Sender，避免悬挂 handler 与内存泄漏
                RemoveEventHandler(handler_ref);
                drop(Box::from_raw(tx_ptr as *mut Sender<()>));
                return None;
            }
            info!("global hotkey cmd-shift-v registered");
            Some(Self {
                rx,
                handler_ref: handler_ref as usize,
                hotkey_ref: hotkey_ref as usize,
                tx_ptr: tx_ptr as usize,
            })
        }
    }

    /// 非阻塞取一次热键事件（多次触发只需知道是否发生过，故 drain 后返回是否有）
    pub fn poll(&self) -> bool {
        let mut fired = false;
        while self.rx.try_recv().is_ok() {
            fired = true;
        }
        fired
    }
}

impl Drop for HotkeyListener {
    /// 注销热键 → 移除 handler → 回收 Sender。须与注册同在主线程，避免与事件分发竞争。
    /// 先移除 handler 阻断后续回调，再释放其借用的 Sender
    fn drop(&mut self) {
        unsafe {
            UnregisterEventHotKey(self.hotkey_ref as EventHotKeyRef);
            RemoveEventHandler(self.handler_ref as EventHandlerRef);
            drop(Box::from_raw(self.tx_ptr as *mut Sender<()>));
        }
        info!("global hotkey cmd-shift-v unregistered");
    }
}
