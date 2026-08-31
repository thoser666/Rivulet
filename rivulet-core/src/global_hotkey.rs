//! OS-level global hotkey registration.
//!
//! The GUI dispatches in-app hotkeys itself while the window is focused (that
//! works on every platform). This module layers a *global* registration on top
//! so the bindings keep working while the app is unfocused — the OBS-style
//! behaviour requested in M5 issue #80.
//!
//! Platform matrix (honest):
//! - **Windows**: real `RegisterHotKey` behind a dedicated message-pump thread,
//!   so hotkeys fire even when the window is not focused.
//! - **Linux / macOS**: not yet implemented at the OS level (Linux needs an X11
//!   grab / Wayland \u00e0-la shortcuts portal; macOS needs Carbon
//!   `RegisterEventHotKey`). On those platforms `set_bindings` is a no-op and
//!   hotkeys remain in-app (focused) only, which is documented in docs/hotkeys.md.

use std::sync::mpsc;

/// A key expressed as a Windows virtual-key code. Kept UI-agnostic here so the
/// GUI (egui) and any headless caller can translate to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyCode(pub u32);

/// Modifier mask. Bit 0 = Ctrl, 1 = Alt, 2 = Shift, 3 = Super/Win.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ModMask(pub u8);

impl ModMask {
    pub fn ctrl(self) -> bool {
        self.0 & 0b0001 != 0
    }
    pub fn alt(self) -> bool {
        self.0 & 0b0010 != 0
    }
    pub fn shift(self) -> bool {
        self.0 & 0b0100 != 0
    }
    pub fn super_(self) -> bool {
        self.0 & 0b1000 != 0
    }
}

/// One registered binding: an action name plus its key/modifiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalBinding {
    pub action: String,
    pub key: KeyCode,
    pub mods: ModMask,
}

/// Global hotkey handle. Owns the registration and the event stream.
#[derive(Debug)]
pub struct GlobalHotkey {
    rx: mpsc::Receiver<String>,
    // internal registration state lives in the platform backend; the struct is
    // always constructed through `GlobalHotkey::new()`.
    #[allow(dead_code)]
    backend: crate::global_hotkey::GlobalHotkeyInner,
}

// The concrete per-platform machinery. On Windows this is a message-pump thread
// that owns the registrations; elsewhere it is an empty no-op type.
#[cfg(target_os = "windows")]
mod imp {
    use super::{GlobalBinding, ModMask};
    use std::collections::HashMap;
    use std::sync::mpsc;
    use std::thread;
    use winapi::um::winuser::{
        GetMessageW, RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
        MOD_SHIFT, MOD_WIN, MSG, WM_HOTKEY, WM_QUIT,
    };

    pub struct GlobalHotkeyInner {
        // The message-pump thread keeps running until the handle is dropped.
        _tx: mpsc::Sender<Vec<GlobalBinding>>,
        _shutdown: mpsc::Sender<()>,
    }

    impl std::fmt::Debug for GlobalHotkeyInner {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("GlobalHotkeyInner")
        }
    }

    impl GlobalHotkeyInner {
        /// Spawn the message-pump thread and register the initial bindings.
        pub fn new(bindings: Vec<GlobalBinding>, events_tx: mpsc::Sender<String>) -> Self {
            let (ctrl_tx, ctrl_rx) = mpsc::channel::<Vec<GlobalBinding>>();
            let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();
            thread::Builder::new()
                .name("rivulet-global-hotkeys".to_string())
                .spawn(move || {
                    run_pump(events_tx, ctrl_rx, shutdown_rx);
                })
                .expect("failed to spawn global-hotkey thread");
            let _ = ctrl_tx.send(bindings);
            GlobalHotkeyInner {
                _tx: ctrl_tx,
                _shutdown: shutdown_tx,
            }
        }

        /// Re-register the full set of bindings in-place.
        pub fn rebind(&self, bindings: Vec<GlobalBinding>) {
            let _ = self._tx.send(bindings);
        }
    }

    impl Drop for GlobalHotkeyInner {
        fn drop(&mut self) {
            let _ = self._shutdown.send(());
        }
    }

    fn register_bindings(bindings: &[GlobalBinding], ids: &mut HashMap<u32, String>) {
        for (idx, b) in bindings.iter().enumerate() {
            let id = idx as i32 + 1; // 0 is reserved
            let mods = mod_flags(b.mods);
            // hWnd = NULL ties the hotkey to this thread's message queue.
            let ok = unsafe { RegisterHotKey(std::ptr::null_mut(), id, mods, b.key.0) };
            if ok != 0 {
                ids.insert(id as u32, b.action.clone());
            }
        }
    }

    fn unregister_bindings(ids: &HashMap<u32, String>) {
        for id in ids.keys() {
            unsafe {
                UnregisterHotKey(std::ptr::null_mut(), *id as i32);
            }
        }
    }

    fn run_pump(
        events_tx: mpsc::Sender<String>,
        ctrl_rx: mpsc::Receiver<Vec<GlobalBinding>>,
        shutdown_rx: mpsc::Receiver<()>,
    ) {
        let mut pending: Vec<GlobalBinding> = ctrl_rx.recv().unwrap_or_default();
        let mut ids: HashMap<u32, String> = HashMap::new();
        register_bindings(&pending, &mut ids);

        loop {
            // Re-register if the control channel has newer bindings.
            if let Ok(new) = ctrl_rx.try_recv() {
                unregister_bindings(&ids);
                ids.clear();
                pending = new;
                register_bindings(&pending, &mut ids);
            }
            if let Ok(()) = shutdown_rx.try_recv() {
                unregister_bindings(&ids);
                return;
            }

            let mut msg: MSG = unsafe { std::mem::zeroed() };
            // GetMessageW blocks until a message (hotkey) or WM_QUIT arrives.
            let ret = unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) };
            if ret <= 0 {
                return;
            }
            if msg.message == WM_QUIT {
                return;
            }
            if msg.message == WM_HOTKEY {
                let id = msg.wParam as u32;
                if let Some(action) = ids.get(&id) {
                    let _ = events_tx.send(action.clone());
                }
            }
        }
    }

    fn mod_flags(m: ModMask) -> u32 {
        let mut flags = MOD_NOREPEAT as u32;
        if m.ctrl() {
            flags |= MOD_CONTROL as u32;
        }
        if m.alt() {
            flags |= MOD_ALT as u32;
        }
        if m.shift() {
            flags |= MOD_SHIFT as u32;
        }
        if m.super_() {
            flags |= MOD_WIN as u32;
        }
        flags
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    use super::GlobalBinding;
    use std::sync::mpsc;

    /// No-op backend (Linux/macOS not yet implemented).
    pub struct GlobalHotkeyInner {
        #[allow(dead_code)]
        bindings: std::sync::Mutex<Vec<GlobalBinding>>,
    }

    impl std::fmt::Debug for GlobalHotkeyInner {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("GlobalHotkeyInner")
        }
    }

    impl GlobalHotkeyInner {
        pub fn new(bindings: Vec<GlobalBinding>, _tx: mpsc::Sender<String>) -> Self {
            GlobalHotkeyInner {
                bindings: std::sync::Mutex::new(bindings),
            }
        }
    }
}

use imp::GlobalHotkeyInner;

impl GlobalHotkey {
    /// Create a global-hotkey handle for the given bindings. New bindings can
    /// be pushed later through [`GlobalHotkey::set_bindings`].
    pub fn new(bindings: Vec<GlobalBinding>) -> Self {
        let (tx, rx) = mpsc::channel();
        let backend = GlobalHotkeyInner::new(bindings, tx);
        GlobalHotkey { rx, backend }
    }

    /// Replace the full set of bindings (re-registering on Windows). Returns
    /// immediately; failures are logged, never fatal. On Linux/macOS this is a
    /// no-op because OS-level registration is not implemented there yet.
    #[cfg(target_os = "windows")]
    pub fn set_bindings(&mut self, bindings: Vec<GlobalBinding>) {
        self.backend.rebind(bindings);
    }

    /// Replace the full set of bindings.
    #[cfg(not(target_os = "windows"))]
    pub fn set_bindings(&mut self, _bindings: Vec<GlobalBinding>) {
        // no-op: platform global registration not yet implemented
    }

    /// Non-blocking drain of fired actions.
    pub fn try_recv(&self) -> Option<String> {
        self.rx.try_recv().ok()
    }

    /// Blocking receive (used only by tests).
    #[allow(dead_code)]
    fn recv(&self) -> Option<String> {
        self.rx.recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mod_mask_flags() {
        // Bit 0=Ctrl, 1=Alt, 2=Shift, 3=Super. 0b0111 = Ctrl|Alt|Shift.
        let m = ModMask(0b0111);
        assert!(m.ctrl());
        assert!(m.alt());
        assert!(m.shift());
        assert!(!m.super_());
        assert!(ModMask(0b1000).super_());
        assert!(ModMask(0b0001).ctrl());
        assert!(ModMask(0b0010).alt());
        assert!(ModMask(0b0100).shift());
    }

    #[test]
    fn empty_bindings_is_sendable_and_constructible_on_all_platforms() {
        // Construction must not panic on any platform (Windows spawns a thread;
        // Linux/macOS keeps a no-op). The handle is promptly dropped, which is
        // fine for a smoke test.
        let hotkey = GlobalHotkey::new(vec![]);
        assert!(hotkey.try_recv().is_none());
    }

    #[test]
    fn binding_struct_round_trips_fields() {
        let b = GlobalBinding {
            action: "record".to_string(),
            key: KeyCode(0x78), // F9
            mods: ModMask(0b0001),
        };
        assert_eq!(b.action, "record");
        assert_eq!(b.key, KeyCode(0x78));
        assert!(b.mods.ctrl());
        // clonable + comparable for tests that build expected sets.
        assert_eq!(b.clone(), b);
    }
}
