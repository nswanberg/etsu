use crate::error::{AppError, Result};
use once_cell::sync::Lazy;
use std::sync::Mutex;
use tracing::{info, warn};
use twox_hash::XxHash64;

use glfw::Monitor as GlfwMonitor;
#[cfg(target_os = "macos")]
use core_foundation::base::TCFType;
#[cfg(target_os = "macos")]
use core_foundation::boolean::CFBoolean;
#[cfg(target_os = "macos")]
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
#[cfg(target_os = "macos")]
use core_foundation::string::{CFString, CFStringRef};

#[derive(Debug, Clone, PartialEq)]
pub struct MonitorInfo {
    pub id_hash: u64,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width_px: u32,
    pub height_px: u32,
    pub width_mm: u32,
    pub height_mm: u32,
    pub ppi: f64,
}

static MONITOR_INFO_CACHE: Lazy<Mutex<Option<Vec<MonitorInfo>>>> = Lazy::new(|| Mutex::new(None));

#[derive(Debug, Clone, Copy, Default)]
pub struct InputCapturePermissions {
    pub accessibility_granted: Option<bool>,
    pub input_monitoring_granted: Option<bool>,
}

impl InputCapturePermissions {
    pub fn missing_accessibility(self) -> bool {
        matches!(self.accessibility_granted, Some(false))
    }

    pub fn missing_input_monitoring(self) -> bool {
        matches!(self.input_monitoring_granted, Some(false))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("GLFW initialization failed: {0:?}")]
    GlfwInit(glfw::InitError),
    #[error("Failed to lock monitor cache")]
    CacheLock,
    #[error("Monitor cache init error")]
    CacheInit,
    #[error("No monitors available")]
    MonitorNotFound,
}

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
    static kAXTrustedCheckOptionPrompt: CFStringRef;
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
}

pub fn detect_input_capture_permissions() -> InputCapturePermissions {
    #[cfg(target_os = "macos")]
    unsafe {
        return InputCapturePermissions {
            accessibility_granted: Some(AXIsProcessTrusted()),
            input_monitoring_granted: Some(CGPreflightListenEventAccess()),
        };
    }

    #[cfg(not(target_os = "macos"))]
    {
        InputCapturePermissions::default()
    }
}

pub fn log_input_capture_permissions(executable_path: &str) -> InputCapturePermissions {
    let permissions = detect_input_capture_permissions();

    match permissions.input_monitoring_granted {
        Some(true) => info!("Input Monitoring permission is granted for {}.", executable_path),
        Some(false) => warn!(
            "Input Monitoring permission is not granted for {}. Enable System Settings > Privacy & Security > Input Monitoring.",
            executable_path
        ),
        None => {}
    }

    match permissions.accessibility_granted {
        Some(true) => info!("Accessibility permission is granted for {}.", executable_path),
        Some(false) => warn!(
            "Accessibility permission is not granted for {}. Enable System Settings > Privacy & Security > Accessibility.",
            executable_path
        ),
        None => {}
    }

    permissions
}

pub fn request_input_capture_permissions(executable_path: &str) {
    #[cfg(target_os = "macos")]
    unsafe {
        let permissions = detect_input_capture_permissions();

        if permissions.missing_input_monitoring() {
            warn!(
                "Input Monitoring permission is not granted for {}. Requesting the macOS permission prompt now.",
                executable_path
            );
            let _ = CGRequestListenEventAccess();
        }

        if permissions.missing_accessibility() {
            warn!(
                "Accessibility permission is not granted for {}. Requesting the macOS permission prompt now.",
                executable_path
            );
            let prompt_key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
            let options = CFDictionary::from_CFType_pairs(&[
                (prompt_key, CFBoolean::true_value()),
            ]);
            let _ = AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef());
        }
    }
}

/// Initializes GLFW, fetches monitor information, calculates PPI, caches it, and terminates GLFW.
/// Must be called once at startup from the main thread.
pub fn initialize_monitor_info() -> std::result::Result<(), PlatformError> {
    info!("Initializing GLFW for monitor detection...");

    let mut glfw = glfw::init(glfw::fail_on_errors).map_err(PlatformError::GlfwInit)?;

    let monitors = glfw.with_connected_monitors(|_glfw, monitors| {
        let mut info_list = Vec::new();
        for (index, monitor) in monitors.iter().enumerate() {
            if let Some(info) = get_info_for_monitor(monitor, index) {
                info_list.push(info);
            }
        }
        info_list
    });

    info!("Detected {} monitors.", monitors.len());
    for monitor in &monitors {
        info!(
            "  Monitor '{}' [{}x{} @ ({},{}) - {:.1} PPI]",
            monitor.name, monitor.width_px, monitor.height_px, monitor.x, monitor.y, monitor.ppi
        );
    }

    let mut cache = MONITOR_INFO_CACHE
        .lock()
        .map_err(|_| PlatformError::CacheLock)?;
    *cache = Some(monitors);

    info!("Monitor information cached successfully. GLFW terminated.");
    Ok(())
}

fn get_info_for_monitor(monitor: &GlfwMonitor, index: usize) -> Option<MonitorInfo> {
    monitor.get_video_mode().map(|mode| {
        let (width_mm, height_mm) = monitor.get_physical_size();
        let (x, y) = monitor.get_pos();
        let name = monitor
            .get_name()
            .unwrap_or_else(|| format!("Monitor {}", index));

        let ppi = if width_mm > 0 && height_mm > 0 {
            let width_in = width_mm as f64 / 25.4;
            let height_in = height_mm as f64 / 25.4;
            let ppi_x = mode.width as f64 / width_in;
            let ppi_y = mode.height as f64 / height_in;
            (ppi_x + ppi_y) / 2.0
        } else {
            warn!(
                "Monitor '{}' reported 0 physical size, assuming default PPI.",
                name
            );
            96.0 // default ppi
        };

        MonitorInfo {
            id_hash: hash_name_xxhash64(&name),
            name,
            x,
            y,
            width_px: mode.width,
            height_px: mode.height,
            width_mm: width_mm as u32,
            height_mm: height_mm as u32,
            ppi,
        }
    })
}

pub fn get_cached_monitor_info() -> Result<Vec<MonitorInfo>> {
    let cache = MONITOR_INFO_CACHE
        .lock()
        .map_err(|_| AppError::Platform(PlatformError::CacheLock))?;

    cache
        .as_ref()
        .cloned()
        .ok_or_else(|| AppError::Platform(PlatformError::CacheInit))
}

/// Finds the monitor containing the given screen coordinates using the cached monitor info.
/// Defaults to the primary/first monitor if coordinates are outside known bounds.
pub fn get_monitor_for_point(x: i32, y: i32) -> Result<MonitorInfo> {
    let monitors = get_cached_monitor_info()?;
    if monitors.is_empty() {
        return Err(AppError::Platform(PlatformError::MonitorNotFound));
    }

    let found_monitor = monitors.iter().find(|m| {
        x >= m.x && x < (m.x + m.width_px as i32) && y >= m.y && y < (m.y + m.height_px as i32)
    });

    match found_monitor {
        Some(monitor) => Ok(monitor.clone()),
        None => {
            warn!(
                "Coordinates ({}, {}) outside known monitor bounds, using first monitor as default.",
                x, y
            );
            monitors.first().cloned().ok_or_else(|| AppError::Platform(PlatformError::MonitorNotFound))
        }
    }
}

fn hash_name_xxhash64(name: &str) -> u64 {
    XxHash64::oneshot(42, name.as_bytes())
}
