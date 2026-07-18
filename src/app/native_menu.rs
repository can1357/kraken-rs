//! Native macOS presenter for right-click context menus.
//!
//! Builds an `NSMenu` from the state-derived [`MenuSpec`] and pops it up at
//! the cursor. The call blocks inside `AppKit`'s menu-tracking loop; the chosen
//! item is drained from muda's event channel once tracking ends and mapped
//! back to the [`UiAction`] it was built from. Headless automation never
//! reaches this module — it drives the drawn fallback renderer instead.

use muda::{ContextMenu, Menu, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use winit::{
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
    window::Window,
};

use crate::ui::{
    action::UiAction,
    menu::{MenuEntry, MenuSpec},
};

/// Shows `spec` as a native context menu at the cursor and returns the chosen
/// action, or `None` when the menu was cancelled or could not be presented.
pub(crate) fn show(window: &Window, spec: &MenuSpec) -> Option<UiAction> {
    let menu = Menu::new();
    let mut bindings: Vec<(MenuId, UiAction)> = Vec::new();

    let title = MenuItem::new(&spec.title, false, None);
    menu.append(&title).ok()?;
    menu.append(&PredefinedMenuItem::separator()).ok()?;

    for entry in &spec.entries {
        match entry {
            MenuEntry::Item {
                label,
                action,
                enabled,
            } => {
                let item = MenuItem::new(label, *enabled, None);
                bindings.push((item.id().clone(), action.clone()));
                menu.append(&item).ok()?;
            }
            MenuEntry::Separator => {
                menu.append(&PredefinedMenuItem::separator()).ok()?;
            }
            MenuEntry::Submenu { label, entries } => {
                let submenu = Submenu::new(label, true);
                for (label, action) in entries {
                    let item = MenuItem::new(label, true, None);
                    bindings.push((item.id().clone(), action.clone()));
                    submenu.append(&item).ok()?;
                }
                menu.append(&submenu).ok()?;
            }
        }
    }

    let RawWindowHandle::AppKit(handle) = window.window_handle().ok()?.as_raw() else {
        return None;
    };
    // Drain anything stale so the pick we read below belongs to this popup.
    while muda::MenuEvent::receiver().try_recv().is_ok() {}
    if !popup(&menu, handle.ns_view.as_ptr()) {
        return None;
    }
    let picked = muda::MenuEvent::receiver().try_recv().ok()?;
    bindings
        .into_iter()
        .find_map(|(id, action)| (id == picked.id).then_some(action))
}

/// Runs the blocking `AppKit` tracking loop for `menu` at the cursor position.
#[allow(unsafe_code)]
fn popup(menu: &Menu, ns_view: *mut std::ffi::c_void) -> bool {
    // SAFETY: `ns_view` comes from winit's AppKit handle for a live window and
    // stays valid for this synchronous call, which winit delivers on the macOS
    // main thread as muda requires.
    unsafe { menu.show_context_menu_for_nsview(ns_view.cast_const(), None) }
}
