//! Wayland MITM proxy: `KWin` <-> bigcedilla <-> plasma-keyboard/Maliit.
//!
//! `KWin` spawns us as the registered IM (kwinrc `[Wayland] InputMethod`).
//! Upstream we own the singleton `zwp_input_method_v1`. Downstream we run a
//! private Wayland display where the real virtual keyboard (plasma-keyboard
//! by default, Maliit when `BIGCEDILLA_CHILD_IM=maliit-keyboard`) connects,
//! sees its usual globals, and behaves normally (panel surfaces +
//! `commit_string` for clicked keys).
//!
//! On top of that pass-through we transparently grab a private `wl_keyboard`
//! upstream (not exposed to the child) and run hardware keys through our
//! compose state machine. `dead_acute + c` -> `commit_string("ç")` to `KWin`;
//! everything else replayed via `context.key()` so the focused app processes
//! it normally (including its own `XCompose`).

#![allow(clippy::similar_names)] // wl-proxy trait methods name the receiver `slf`

use std::cell::RefCell;
use std::os::fd::{AsRawFd, OwnedFd};
use std::rc::Rc;

use wl_proxy::object::{Object, ObjectCoreApi, ObjectRcUtils};
use wl_proxy::protocols::ObjectInterface;
use wl_proxy::protocols::input_method_unstable_v1::zwp_input_method_context_v1::{
    ZwpInputMethodContextV1, ZwpInputMethodContextV1Handler,
};
use wl_proxy::protocols::input_method_unstable_v1::zwp_input_method_v1::{
    ZwpInputMethodV1, ZwpInputMethodV1Handler,
};
use wl_proxy::protocols::wayland::wl_display::{WlDisplay, WlDisplayHandler};
use wl_proxy::protocols::wayland::wl_keyboard::{
    WlKeyboard, WlKeyboardHandler, WlKeyboardKeyState, WlKeyboardKeymapFormat,
};
use wl_proxy::protocols::wayland::wl_registry::{WlRegistry, WlRegistryHandler};
use xkbcommon::xkb;

use crate::compose::{Action, ComposeState, PendingKey};

const KEYCODE_OFFSET: u32 = 8;

type Shared = Rc<RefCell<ClientCtx>>;

struct ClientCtx {
    xkb_ctx: xkb::Context,
    xkb_state: Option<xkb::State>,
    compose: ComposeState,
    /// Latest serial from `commit_state`. Used when synthesizing `commit_string`.
    text_serial: u32,
    /// Latest serial from `wl_keyboard.key`. Used when forwarding via `context.key()`.
    last_kbd_serial: u32,
}

impl ClientCtx {
    fn new() -> Shared {
        Rc::new(RefCell::new(Self {
            xkb_ctx: xkb::Context::new(xkb::CONTEXT_NO_FLAGS),
            xkb_state: None,
            compose: ComposeState::default(),
            text_serial: 0,
            last_kbd_serial: 0,
        }))
    }
}

pub struct DisplayHandler {
    state: Shared,
}

impl DisplayHandler {
    pub fn new() -> Self {
        Self {
            state: ClientCtx::new(),
        }
    }
}

impl WlDisplayHandler for DisplayHandler {
    fn handle_get_registry(&mut self, slf: &Rc<WlDisplay>, registry: &Rc<WlRegistry>) {
        registry.set_handler(RegistryHandler {
            state: self.state.clone(),
        });
        slf.send_get_registry(registry);
    }
}

struct RegistryHandler {
    state: Shared,
}

impl WlRegistryHandler for RegistryHandler {
    fn handle_bind(&mut self, slf: &Rc<WlRegistry>, name: u32, id: Rc<dyn Object>) {
        if id.interface() == ObjectInterface::ZwpInputMethodV1
            && let Some(im) = id.clone().try_downcast::<ZwpInputMethodV1>()
        {
            log::debug!("intercept bind zwp_input_method_v1 name={name}");
            im.set_handler(ImHandler {
                state: self.state.clone(),
            });
        }
        slf.send_bind(name, id);
    }
}

struct ImHandler {
    state: Shared,
}

impl ZwpInputMethodV1Handler for ImHandler {
    fn handle_activate(&mut self, slf: &Rc<ZwpInputMethodV1>, id: &Rc<ZwpInputMethodContextV1>) {
        log::debug!("activate: text input focused");
        id.set_handler(CtxHandler {
            state: self.state.clone(),
        });
        slf.send_activate(id);

        // Private upstream-only wl_keyboard for compose interception.
        let kbd = id.new_send_grab_keyboard();
        kbd.set_handler(KeyboardHandler {
            state: self.state.clone(),
            ctx: id.clone(),
        });

        self.state.borrow_mut().compose = ComposeState::default();
    }

    fn handle_deactivate(
        &mut self,
        slf: &Rc<ZwpInputMethodV1>,
        context: &Rc<ZwpInputMethodContextV1>,
    ) {
        log::debug!("deactivate: text input lost focus");
        slf.send_deactivate(context);

        let mut st = self.state.borrow_mut();
        st.compose = ComposeState::default();
        st.xkb_state = None;
    }
}

struct CtxHandler {
    state: Shared,
}

impl ZwpInputMethodContextV1Handler for CtxHandler {
    fn handle_commit_state(&mut self, slf: &Rc<ZwpInputMethodContextV1>, serial: u32) {
        self.state.borrow_mut().text_serial = serial;
        slf.send_commit_state(serial);
    }

    fn handle_reset(&mut self, slf: &Rc<ZwpInputMethodContextV1>) {
        self.state.borrow_mut().compose = ComposeState::default();
        slf.send_reset();
    }
}

struct KeyboardHandler {
    state: Shared,
    ctx: Rc<ZwpInputMethodContextV1>,
}

impl KeyboardHandler {
    fn keysym_for(&self, keycode: u32) -> Option<u32> {
        let st = self.state.borrow();
        Some(st.xkb_state.as_ref()?.key_get_one_sym(keycode.into()).raw())
    }

    fn forward_key(&self, time: u32, key: u32, state: WlKeyboardKeyState) {
        let serial = self.state.borrow().last_kbd_serial;
        self.ctx.send_key(serial, time, key, state.0);
    }

    fn replay(&self, prev: PendingKey) {
        self.forward_key(prev.time, prev.keycode, WlKeyboardKeyState::PRESSED);
        self.forward_key(prev.time, prev.keycode, WlKeyboardKeyState::RELEASED);
    }

    fn commit_text(&self, text: &str) {
        let serial = self.state.borrow().text_serial;
        self.ctx.send_commit_string(serial, text);
    }

    fn handle_press(&self, time: u32, key: u32) {
        let Some(sym) = self.keysym_for(key + KEYCODE_OFFSET) else {
            self.forward_key(time, key, WlKeyboardKeyState::PRESSED);
            return;
        };
        match self.state.borrow_mut().compose.on_press(sym, key, time) {
            Action::Forward => self.forward_key(time, key, WlKeyboardKeyState::PRESSED),
            Action::Swallow => {}
            Action::Commit(text) => self.commit_text(text),
            Action::ReplayThenForward(prev) => {
                self.replay(prev);
                self.forward_key(time, key, WlKeyboardKeyState::PRESSED);
            }
            Action::ReplayThenBuffer(prev) => self.replay(prev),
        }
    }
}

impl WlKeyboardHandler for KeyboardHandler {
    fn handle_keymap(
        &mut self,
        _slf: &Rc<WlKeyboard>,
        _format: WlKeyboardKeymapFormat,
        fd: &Rc<OwnedFd>,
        size: u32,
    ) {
        let xkb_ctx = self.state.borrow().xkb_ctx.clone();
        match build_xkb_state(&xkb_ctx, fd, size) {
            Some(s) => self.state.borrow_mut().xkb_state = Some(s),
            None => log::warn!("failed to build xkb state from compositor keymap"),
        }
    }

    fn handle_key(
        &mut self,
        _slf: &Rc<WlKeyboard>,
        serial: u32,
        time: u32,
        key: u32,
        state: WlKeyboardKeyState,
    ) {
        self.state.borrow_mut().last_kbd_serial = serial;
        match state {
            WlKeyboardKeyState::PRESSED => self.handle_press(time, key),
            WlKeyboardKeyState::RELEASED => {
                let swallow = self.state.borrow_mut().compose.on_release(key);
                if !swallow {
                    self.forward_key(time, key, WlKeyboardKeyState::RELEASED);
                }
            }
            other => self.forward_key(time, key, other),
        }
    }

    fn handle_modifiers(
        &mut self,
        _slf: &Rc<WlKeyboard>,
        serial: u32,
        mods_depressed: u32,
        mods_latched: u32,
        mods_locked: u32,
        group: u32,
    ) {
        if let Some(xkb_state) = self.state.borrow_mut().xkb_state.as_mut() {
            xkb_state.update_mask(mods_depressed, mods_latched, mods_locked, 0, 0, group);
        }
        self.ctx
            .send_modifiers(serial, mods_depressed, mods_latched, mods_locked, group);
    }
}

fn build_xkb_state(ctx: &xkb::Context, fd: &OwnedFd, size: u32) -> Option<xkb::State> {
    use libc::{MAP_PRIVATE, PROT_READ, mmap, munmap};
    use std::ptr;
    let len = size as usize;
    let ptr = unsafe {
        mmap(
            ptr::null_mut(),
            len,
            PROT_READ,
            MAP_PRIVATE,
            fd.as_raw_fd(),
            0,
        )
    };
    if ptr == libc::MAP_FAILED {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), len) };
    let nul = bytes.iter().position(|&b| b == 0).unwrap_or(len);
    let keymap = std::str::from_utf8(&bytes[..nul]).ok().and_then(|s| {
        xkb::Keymap::new_from_string(
            ctx,
            s.to_owned(),
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
    });
    unsafe { munmap(ptr, len) };
    keymap.map(|k| xkb::State::new(&k))
}
