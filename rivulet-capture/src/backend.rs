//! Platform-independent capture backend status and error classification.
//!
//! The G1 strategy ([docs/game-capture-strategy.md]) requires every capture
//! run to report *which* backend is active and whether a fallback occurred,
//! and to classify DXGI failure codes into actionable states instead of
//! surfacing raw HRESULTs. This module contains that logic so it can be unit
//! tested on every CI platform (Linux/macOS/Windows), while the actual DXGI
//! calls live in the Windows-only [`crate::dxgi`] module.

/// The capture backend currently responsible for producing frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    /// DXGI Desktop Duplication (G2) — primary zero-overhead fullscreen path.
    DesktopDuplication,
    /// Windows Graphics Capture (existing windowed path, fallback).
    WindowsGraphicsCapture,
    /// No backend is active (not started, or everything failed).
    None,
}

impl BackendKind {
    /// Stable identifier used for logs and GUI status strings.
    pub fn as_str(self) -> &'static str {
        match self {
            BackendKind::DesktopDuplication => "desktop-duplication",
            BackendKind::WindowsGraphicsCapture => "windows-graphics-capture",
            BackendKind::None => "none",
        }
    }
}

/// Live status of a capture backend run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendStatus {
    /// The backend currently active (or `None` if not capturing).
    pub active: BackendKind,
    /// Whether a fallback happened since the run started.
    pub fallback_occurred: bool,
    /// Human-readable reason of the last failure, if any.
    pub last_error: Option<String>,
}

impl BackendStatus {
    /// A fresh status with no backend active.
    pub fn idle() -> Self {
        Self {
            active: BackendKind::None,
            fallback_occurred: false,
            last_error: None,
        }
    }

    /// Mark `next` as the active backend, remembering whether that is a
    /// downgrade from the previous backend (i.e. a fallback occurred).
    pub fn switch_to(&mut self, next: BackendKind) {
        if self.active != BackendKind::None && self.active != next {
            self.fallback_occurred = true;
        }
        self.active = next;
    }

    /// Record a failure; the message is kept for GUI/log reporting.
    pub fn fail(&mut self, message: impl Into<String>) {
        self.last_error = Some(message.into());
    }

    /// True when the run is healthy (a backend is active and no error is
    /// pending).
    pub fn is_healthy(&self) -> bool {
        self.active != BackendKind::None && self.last_error.is_none()
    }

    /// The i18n key (and optional fallback reason) the GUI should render for
    /// this status. The key is resolved through the app's locale table;
    /// keeping the mapping here makes it unit-testable on every platform
    /// without a GUI.
    pub fn ui_key(&self) -> (&'static str, Option<&str>) {
        match self.active {
            BackendKind::DesktopDuplication => ("backend_desktop_duplication", None),
            // Only use the parameterised fallback key when there is a reason
            // to show; otherwise the plain WGC key avoids a literal `{0}`.
            BackendKind::WindowsGraphicsCapture => match self.last_error.as_deref() {
                Some(r) if !r.is_empty() => ("backend_wgc_fallback", Some(r)),
                _ => ("backend_wgc", None),
            },
            BackendKind::None => ("backend_none", None),
        }
    }
}

/// Classified DXGI failure codes (Windows-only at runtime; the numeric
/// mapping is stable and therefore unit-testable everywhere).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DxgiFailure {
    /// `DXGI_ERROR_WAIT_TIMEOUT` — no new frame within the acquire timeout.
    /// Not an error: the caller should try again.
    Timeout,
    /// `DXGI_ERROR_ACCESS_LOST` — the desktop session changed (mode switch,
    /// lock, RDP disconnect). The duplication must be re-created.
    AccessLost,
    /// `DXGI_ERROR_ACCESS_DENIED` — protected content or a desktop that
    /// cannot be duplicated (e.g. secure desktop / UAC). Never a black
    /// frame; surface "not capturable".
    AccessDenied,
    /// `DXGI_ERROR_UNSUPPORTED` — Desktop Duplication unavailable on this
    /// target (e.g. Windows 7 or WARP-only).
    Unsupported,
    /// Any other failure — reported as-is.
    Other(i32),
}

impl DxgiFailure {
    /// Classify a raw Win32 HRESULT into a [`DxgiFailure`].
    ///
    /// The numeric constants are taken from the DXGI error table; they are
    /// stable across Windows versions, so this function is unit-tested on
    /// every platform without needing a real DXGI device.
    pub fn classify(hr: i32) -> DxgiFailure {
        // DXGI error codes are negative i32 values; compare bit patterns as
        // u32 so the hex literals read naturally (e.g. 0x887A0027).
        let bits = hr as u32;
        // 0x887A0027 — WAIT_TIMEOUT
        if bits == 0x887A0027 {
            return DxgiFailure::Timeout;
        }
        // 0x887A0026 — ACCESS_LOST
        if bits == 0x887A0026 {
            return DxgiFailure::AccessLost;
        }
        // 0x887A002B — ACCESS_DENIED
        if bits == 0x887A002B {
            return DxgiFailure::AccessDenied;
        }
        // 0x887A0004 — UNSUPPORTED
        if bits == 0x887A0004 {
            return DxgiFailure::Unsupported;
        }
        DxgiFailure::Other(hr)
    }

    /// Whether this failure is fatal for the current duplication object
    /// (the capture must be re-created or abandoned).
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            DxgiFailure::AccessLost | DxgiFailure::AccessDenied | DxgiFailure::Unsupported
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_kind_as_str_is_stable() {
        assert_eq!(
            BackendKind::DesktopDuplication.as_str(),
            "desktop-duplication"
        );
        assert_eq!(
            BackendKind::WindowsGraphicsCapture.as_str(),
            "windows-graphics-capture"
        );
        assert_eq!(BackendKind::None.as_str(), "none");
    }

    #[test]
    fn status_starts_idle_and_healthy_after_start() {
        let mut s = BackendStatus::idle();
        assert_eq!(s.active, BackendKind::None);
        assert!(!s.is_healthy());

        s.switch_to(BackendKind::DesktopDuplication);
        assert!(s.is_healthy());
        assert!(!s.fallback_occurred);
    }

    #[test]
    fn switching_backend_marks_fallback() {
        let mut s = BackendStatus::idle();
        s.switch_to(BackendKind::DesktopDuplication);
        s.switch_to(BackendKind::WindowsGraphicsCapture);
        assert!(s.fallback_occurred);
        assert_eq!(s.active, BackendKind::WindowsGraphicsCapture);

        // Switching to the same backend is not a fallback.
        let mut s2 = BackendStatus::idle();
        s2.switch_to(BackendKind::DesktopDuplication);
        s2.switch_to(BackendKind::DesktopDuplication);
        assert!(!s2.fallback_occurred);
    }

    #[test]
    fn fail_records_error_and_breaks_health() {
        let mut s = BackendStatus::idle();
        s.switch_to(BackendKind::DesktopDuplication);
        s.fail("access lost");
        assert!(!s.is_healthy());
        assert_eq!(s.last_error.as_deref(), Some("access lost"));
    }

    #[test]
    fn classify_known_dxgi_codes() {
        // DXGI errors are negative i32; the literals are written as their
        // u32 bit patterns cast to i32 (identical at the bit level).
        assert_eq!(
            DxgiFailure::classify(0x887A0027u32 as i32),
            DxgiFailure::Timeout
        );
        assert_eq!(
            DxgiFailure::classify(0x887A0026u32 as i32),
            DxgiFailure::AccessLost
        );
        assert_eq!(
            DxgiFailure::classify(0x887A002Bu32 as i32),
            DxgiFailure::AccessDenied
        );
        assert_eq!(
            DxgiFailure::classify(0x887A0004u32 as i32),
            DxgiFailure::Unsupported
        );
    }

    #[test]
    fn classify_unknown_code_is_other() {
        let hr = 0x80004005u32 as i32; // E_FAIL
        assert_eq!(DxgiFailure::classify(hr), DxgiFailure::Other(hr));
    }

    #[test]
    fn timeout_is_not_fatal_but_access_lost_is() {
        assert!(!DxgiFailure::Timeout.is_fatal());
        assert!(DxgiFailure::AccessLost.is_fatal());
        assert!(DxgiFailure::AccessDenied.is_fatal());
        assert!(DxgiFailure::Unsupported.is_fatal());
    }

    #[test]
    fn ui_key_maps_backend_to_i18n_key() {
        let mut s = BackendStatus::idle();
        s.switch_to(BackendKind::DesktopDuplication);
        assert_eq!(s.ui_key(), ("backend_desktop_duplication", None));

        let mut s2 = BackendStatus::idle();
        s2.switch_to(BackendKind::WindowsGraphicsCapture);
        // No reason → plain fallback key, no literal `{0}`.
        assert_eq!(s2.ui_key(), ("backend_wgc", None));

        s2.fail("access denied");
        assert_eq!(s2.ui_key(), ("backend_wgc_fallback", Some("access denied")));

        assert_eq!(BackendStatus::idle().ui_key(), ("backend_none", None));
    }
}
