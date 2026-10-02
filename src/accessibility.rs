//! Native semantic peers for GPUI controls. Layout and behavior remain owned by the UI.
use gpui::{Bounds, Pixels, Window};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessibilityRole {
    Button,
    TextField,
    StaticText,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AccessibilityNode {
    pub id: String,
    pub label: String,
    pub role: AccessibilityRole,
    pub value: String,
    pub enabled: bool,
    pub selected: bool,
    pub focused: bool,
    /// UTF-16 start and length for the editable search field.
    pub selected_range: Option<(usize, usize)>,
    pub bounds: Bounds<Pixels>,
}

// Actions are constructed by the macOS native accessibility bridge.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Clone, Debug)]
pub enum AccessibilityAction {
    Press(String),
    SetValue(String, String),
    Focus(String),
    SetSelection(String, usize, usize),
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, Sel};
    use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
    use objc2_app_kit::*;
    use objc2_foundation::{
        MainThreadMarker, NSArray, NSObjectProtocol, NSPoint, NSRange, NSRect, NSSize, NSString,
    };
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;

    struct PeerState {
        id: String,
        sender: async_channel::Sender<AccessibilityAction>,
        role: AccessibilityRole,
        value: RefCell<Retained<NSString>>,
        enabled: Cell<bool>,
        focused: Cell<bool>,
        selected_range: Cell<NSRange>,
    }

    define_class!(
        // NSAccessibilityElement is designed for custom accessibility peers.
        #[unsafe(super = NSAccessibilityElement)]
        #[thread_kind = MainThreadOnly]
        #[ivars = PeerState]
        struct Peer;
        unsafe impl NSObjectProtocol for Peer {}
        unsafe impl NSAccessibility for Peer {
            #[unsafe(method(isAccessibilitySelectorAllowed:))]
            fn selector_allowed(&self, selector: Sel) -> bool {
                if selector == objc2::sel!(accessibilityPerformPress) {
                    self.ivars().enabled.get() && self.ivars().role == AccessibilityRole::Button
                } else if selector == objc2::sel!(setAccessibilityValue:)
                    || selector == objc2::sel!(setAccessibilitySelectedTextRange:)
                {
                    self.ivars().enabled.get() && self.ivars().role == AccessibilityRole::TextField
                } else {
                    unsafe { msg_send![super(self), isAccessibilitySelectorAllowed: selector] }
                }
            }
            #[unsafe(method(accessibilityPerformPress))]
            fn press(&self) -> bool {
                self.ivars().enabled.get()
                    && self.ivars().role == AccessibilityRole::Button
                    && self
                        .ivars()
                        .sender
                        .try_send(AccessibilityAction::Press(self.ivars().id.clone()))
                        .is_ok()
            }
            #[unsafe(method_id(accessibilityValue))]
            fn value(&self) -> Retained<AnyObject> {
                self.ivars()
                    .value
                    .borrow()
                    .clone()
                    .into_super()
                    .into_super()
            }
            #[unsafe(method(setAccessibilityValue:))]
            fn set_value(&self, value: Option<&AnyObject>) {
                if self.ivars().role != AccessibilityRole::TextField || !self.ivars().enabled.get()
                {
                    return;
                }
                if let Some(value) = value.and_then(|v| v.downcast_ref::<NSString>()) {
                    let _ = self.ivars().sender.try_send(AccessibilityAction::SetValue(
                        self.ivars().id.clone(),
                        value.to_string(),
                    ));
                }
            }
            #[unsafe(method(accessibilitySelectedTextRange))]
            fn selection(&self) -> NSRange {
                self.ivars().selected_range.get()
            }
            #[unsafe(method(setAccessibilitySelectedTextRange:))]
            fn set_selection(&self, range: NSRange) {
                if self.ivars().role == AccessibilityRole::TextField && self.ivars().enabled.get() {
                    let _ = self
                        .ivars()
                        .sender
                        .try_send(AccessibilityAction::SetSelection(
                            self.ivars().id.clone(),
                            range.location,
                            range.length,
                        ));
                }
            }
            #[unsafe(method_id(accessibilitySelectedText))]
            fn selected_text(&self) -> Retained<NSString> {
                self.range_text(self.ivars().selected_range.get())
            }
            #[unsafe(method_id(accessibilityStringForRange:))]
            fn string_for_range(&self, range: NSRange) -> Retained<NSString> {
                let text = self.ivars().value.borrow().to_string();
                let units: Vec<u16> = text
                    .encode_utf16()
                    .skip(range.location)
                    .take(range.length)
                    .collect();
                NSString::from_str(&String::from_utf16_lossy(&units))
            }
            #[unsafe(method(accessibilityNumberOfCharacters))]
            fn character_count(&self) -> usize {
                self.ivars().value.borrow().length()
            }
            #[unsafe(method(isAccessibilityFocused))]
            fn focused(&self) -> bool {
                self.ivars().focused.get()
            }
            #[unsafe(method(setAccessibilityFocused:))]
            fn set_focused(&self, focused: bool) {
                if focused && self.ivars().enabled.get() {
                    let _ = self
                        .ivars()
                        .sender
                        .try_send(AccessibilityAction::Focus(self.ivars().id.clone()));
                }
            }
        }
    );

    impl Peer {
        fn range_text(&self, range: NSRange) -> Retained<NSString> {
            let text = self.ivars().value.borrow().to_string();
            let units: Vec<u16> = text
                .encode_utf16()
                .skip(range.location)
                .take(range.length)
                .collect();
            NSString::from_str(&String::from_utf16_lossy(&units))
        }
    }

    pub struct AccessibilityBridge {
        view: Retained<NSView>,
        peers: HashMap<String, Retained<Peer>>,
        sender: async_channel::Sender<AccessibilityAction>,
        previous: HashMap<String, AccessibilityNode>,
        focused_peer: Option<Retained<Peer>>,
    }
    impl AccessibilityBridge {
        pub fn new(
            window: &Window,
            sender: async_channel::Sender<AccessibilityAction>,
        ) -> Option<Self> {
            MainThreadMarker::new()?;
            let RawWindowHandle::AppKit(handle) =
                HasWindowHandle::window_handle(window).ok()?.as_raw()
            else {
                return None;
            };
            // GPUI's window handle supplies its live NSView; retain it for the bridge lifetime.
            let view = unsafe { Retained::retain(handle.ns_view.as_ptr().cast::<NSView>()) }?;
            view.setAccessibilityElement(false);
            Some(Self {
                view,
                peers: HashMap::new(),
                sender,
                previous: HashMap::new(),
                focused_peer: None,
            })
        }
        pub fn update(&mut self, nodes: Vec<AccessibilityNode>) {
            let Some(mtm) = MainThreadMarker::new() else {
                return;
            };
            let Some(window) = self.view.window() else {
                return;
            };
            let mut children = Vec::with_capacity(nodes.len());
            let mut ids = std::collections::HashSet::new();
            let mut current = HashMap::new();
            let mut layout_changed = false;
            let mut focused_peer = None;
            for mut node in nodes {
                // AXStaticText conveys its spoken/displayed text through AXValue.
                // A label identifies a control; it cannot replace a static text value.
                if node.role == AccessibilityRole::StaticText && node.value.is_empty() {
                    node.value = node.label.clone();
                }
                if node.bounds.size.width <= gpui::px(0.) || node.bounds.size.height <= gpui::px(0.)
                {
                    continue;
                }
                ids.insert(node.id.clone());
                let previous = self.previous.get(&node.id);
                layout_changed |= previous.is_none_or(|p| {
                    p.bounds != node.bounds || p.label != node.label || p.role != node.role
                });
                if self
                    .peers
                    .get(&node.id)
                    .is_some_and(|p| p.ivars().role != node.role)
                    && let Some(peer) = self.peers.remove(&node.id)
                {
                    unsafe {
                        peer.setAccessibilityParent(None);
                    }
                }
                let peer = self.peers.entry(node.id.clone()).or_insert_with(|| {
                    let allocated = Peer::alloc(mtm).set_ivars(PeerState {
                        id: node.id.clone(),
                        sender: self.sender.clone(),
                        role: node.role,
                        value: RefCell::new(NSString::from_str("")),
                        enabled: Cell::new(true),
                        focused: Cell::new(false),
                        selected_range: Cell::new(NSRange::new(0, 0)),
                    });
                    let peer: Retained<Peer> = unsafe { msg_send![super(allocated), init] };
                    peer.setAccessibilityElement(true);
                    unsafe { peer.setAccessibilityParent(Some(&self.view)) };
                    peer
                });
                let role = unsafe {
                    match node.role {
                        AccessibilityRole::Button => NSAccessibilityButtonRole,
                        AccessibilityRole::TextField => NSAccessibilityTextFieldRole,
                        AccessibilityRole::StaticText => NSAccessibilityStaticTextRole,
                    }
                };
                peer.setAccessibilityRole(Some(role));
                peer.setAccessibilityLabel(Some(&NSString::from_str(&node.label)));
                peer.setAccessibilityIdentifier(Some(&NSString::from_str(&node.id)));
                *peer.ivars().value.borrow_mut() = NSString::from_str(&node.value);
                peer.ivars().enabled.set(node.enabled);
                peer.ivars().focused.set(node.focused);
                let (start, length) = node.selected_range.unwrap_or((0, 0));
                peer.ivars().selected_range.set(NSRange::new(start, length));
                peer.setAccessibilityEnabled(node.enabled);
                peer.setAccessibilitySelected(node.selected);
                let height = f64::from(f32::from(node.bounds.size.height));
                let y = f64::from(f32::from(node.bounds.origin.y));
                let y = if self.view.isFlipped() {
                    y
                } else {
                    self.view.bounds().size.height - y - height
                };
                let local = NSRect::new(
                    NSPoint::new(f64::from(f32::from(node.bounds.origin.x)), y),
                    NSSize::new(f64::from(f32::from(node.bounds.size.width)), height),
                );
                let screen = window.convertRectToScreen(self.view.convertRect_toView(local, None));
                peer.setAccessibilityFrame(screen);
                unsafe {
                    if previous.is_some_and(|p| p.value != node.value) {
                        NSAccessibilityPostNotification(
                            peer,
                            NSAccessibilityValueChangedNotification,
                        );
                    }
                    if previous.is_some_and(|p| p.selected != node.selected) {
                        NSAccessibilityPostNotification(
                            &self.view,
                            NSAccessibilitySelectedChildrenChangedNotification,
                        );
                    }
                    if previous.is_some_and(|p| p.selected_range != node.selected_range) {
                        NSAccessibilityPostNotification(
                            peer,
                            NSAccessibilitySelectedTextChangedNotification,
                        );
                    }
                }
                if node.focused && node.enabled {
                    focused_peer = Some(peer.clone());
                }
                children.push(peer.clone().into_super().into_super().into_super());
                current.insert(node.id.clone(), node);
            }
            self.peers.retain(|id, peer| {
                if ids.contains(id) {
                    true
                } else {
                    unsafe {
                        peer.setAccessibilityParent(None);
                    }
                    false
                }
            });
            let children = NSArray::from_retained_slice(&children);
            unsafe {
                self.view.setAccessibilityChildren(Some(&children));
                if layout_changed || self.previous.len() != current.len() {
                    NSAccessibilityPostNotification(
                        &self.view,
                        NSAccessibilityLayoutChangedNotification,
                    );
                }
            }
            let focused_peer = if window.isKeyWindow() {
                focused_peer
            } else {
                None
            };
            let focus_changed = match (&self.focused_peer, &focused_peer) {
                (Some(old), Some(new)) => !std::ptr::eq(&**old, &**new),
                (None, None) => false,
                _ => true,
            };
            if focus_changed {
                self.clear_application_focus(mtm);
                if let Some(peer) = &focused_peer {
                    let app = NSApplication::sharedApplication(mtm);
                    // AppKit's application AXFocusedUIElement is distinct from each peer's
                    // AXFocused state. Publish only a live peer belonging to this key window.
                    unsafe {
                        app.setAccessibilityApplicationFocusedUIElement(Some(peer));
                        NSAccessibilityPostNotification(
                            &app,
                            NSAccessibilityFocusedUIElementChangedNotification,
                        );
                    }
                }
                self.focused_peer = focused_peer;
            }
            self.previous = current;
        }
    }
    impl AccessibilityBridge {
        fn clear_application_focus(&mut self, mtm: MainThreadMarker) {
            if let Some(peer) = self.focused_peer.take() {
                let app = NSApplication::sharedApplication(mtm);
                // Another window may have since established its own focus; never clear it.
                if app
                    .accessibilityApplicationFocusedUIElement()
                    .is_some_and(|focused| std::ptr::eq(&*focused, &**peer as &AnyObject))
                {
                    unsafe {
                        app.setAccessibilityApplicationFocusedUIElement(None);
                        NSAccessibilityPostNotification(
                            &app,
                            NSAccessibilityFocusedUIElementChangedNotification,
                        );
                    }
                }
            }
        }
    }
    impl Drop for AccessibilityBridge {
        fn drop(&mut self) {
            if let Some(mtm) = MainThreadMarker::new() {
                self.clear_application_focus(mtm);
            }
            // Sever both directions before releasing peers; AppKit parent references can retain.
            unsafe {
                self.view.setAccessibilityChildren(None);
            }
            for peer in self.peers.values() {
                unsafe {
                    peer.setAccessibilityParent(None);
                }
            }
        }
    }
}
#[cfg(target_os = "macos")]
pub use native::AccessibilityBridge;

#[cfg(not(target_os = "macos"))]
pub struct AccessibilityBridge;
#[cfg(not(target_os = "macos"))]
impl AccessibilityBridge {
    pub fn new(_: &Window, _: async_channel::Sender<AccessibilityAction>) -> Option<Self> {
        None
    }
    pub fn update(&mut self, _: Vec<AccessibilityNode>) {}
}
