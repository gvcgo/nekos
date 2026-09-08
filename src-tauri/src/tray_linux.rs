//! Linux-only glue: relay tray-icon `activate` signals to the shared tray
//! activation handler.
//!
//! `tray-icon` documents that icon-click events are unsupported on Linux and
//! never emits [`tauri::tray::TrayIconEvent`] there. The underlying
//! libappindicator (used by tray-icon's GTK backend) still surfaces the SNI
//! host's `Activate` DBus call as a GObject `activate` signal, so we connect
//! to it directly and let the shared handler time repeated activations into
//! single/double clicks. StatusNotifierItem has no double-click concept: the
//! host (e.g. the Quickshell-based "noctalia" shell) sends one `Activate` per
//! physical click, and [`crate::on_tray_primary_activation`] collapses a pair
//! within [`crate::TRAY_DOUBLE_CLICK`] into a toggle.

use std::sync::Arc;

use glib::prelude::*;
use glib::translate::FromGlibPtrNone;
use glib::Object;
use tauri::AppHandle;

use crate::{on_tray_primary_activation, TrayClicks};

/// Connects the tray icon's `activate` signal. Best effort: returns `false`
/// when the appindicator instance cannot be reached. Must be called from the
/// GTK main thread (the app `setup` hook runs there).
pub fn connect(app: &AppHandle, tray: &tauri::tray::TrayIcon) -> bool {
    // with_inner_tray_icon runs the closure on the main thread but requires
    // the return value to be `Send`; raw pointers are not, so pass a usize.
    let Ok(wrapper_ptr) = tray.with_inner_tray_icon(|inner| {
        // Safety: the returned pointer stays valid for as long as the tray
        // icon lives, and tauri keeps the icon alive for the app's lifetime.
        unsafe { inner.app_indicator() as usize }
    }) else {
        return false;
    };
    if wrapper_ptr == 0 {
        return false;
    }

    // libappindicator's Rust wrapper is a pointer newtype: its only field is
    // the raw C GObject pointer. Read that field to reach the instance.
    // Safety: single-field structs share the field's layout/ABI in practice
    // (libappindicator 0.9), and we only read through the wrapper, never drop
    // it.
    let obj_ptr = unsafe { *(wrapper_ptr as *const *mut glib::gobject_ffi::GObject) };
    if obj_ptr.is_null() {
        return false;
    }

    // Take an extra reference so the object outlives this function regardless
    // of what the tray keeps. connect_local detaches its closure only when
    // the object is disposed, which for the tray icon is at process exit.
    let obj: Object = unsafe { FromGlibPtrNone::from_glib_none(obj_ptr) };

    let state = Arc::new((app.clone(), Arc::new(TrayClicks::default())));
    obj.connect_local("activate", false, {
        let state = state.clone();
        move |_| {
            let (app, clicks) = &*state;
            on_tray_primary_activation(app, clicks);
            None
        }
    });

    // The signal connection keeps the closure (and its clone of `state`)
    // alive until the appindicator is disposed at process exit. `obj` is a
    // ref-counted wrapper; dropping it here only releases our extra ref.
    true
}
