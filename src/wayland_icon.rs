//! The window's icon on Wayland. winit 0.30 drops the icon there (its
//! `set_window_icon` does nothing), so the compositor looks for a desktop file
//! named after the app id. An AppImage installs none, and KDE shows its
//! generic Wayland "W" in the title bar and task switcher instead.
//!
//! This sends the same picture through the xdg-toplevel-icon protocol (recent
//! KDE Plasma and other compositors that offer it). Where it isn't offered,
//! nothing happens and the compositor keeps its own choice. X11 and XWayland
//! windows already get the icon from `with_icon`.

use std::fs::File;
use std::io::Write;
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use wayland_backend::client::{Backend, ObjectId};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, delegate_noop};
use wayland_protocols::xdg::shell::client::xdg_toplevel::XdgToplevel;
use wayland_protocols::xdg::toplevel_icon::v1::client::xdg_toplevel_icon_manager_v1::XdgToplevelIconManagerV1;
use wayland_protocols::xdg::toplevel_icon::v1::client::xdg_toplevel_icon_v1::XdgToplevelIconV1;
use winit::platform::wayland::WindowExtWayland;
use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};

/// Gives the window its icon when it's a Wayland window and the compositor
/// takes icons this way. Quiet otherwise: the icon is cosmetic.
pub fn set(cc: &eframe::CreationContext<'_>, icon: &eframe::egui::IconData) {
    let _ = try_set(cc, icon);
}

fn try_set(cc: &eframe::CreationContext<'_>, icon: &eframe::egui::IconData) -> Option<()> {
    let side = i32::try_from(icon.width).ok()?;
    // The protocol takes square pictures only; anything else is a protocol
    // error, which would close the window's connection.
    let len = icon.width as usize * icon.height as usize * 4;
    if side == 0 || icon.width != icon.height || icon.rgba.len() != len {
        return None;
    }
    let window = cc.winit_window()?;
    let toplevel = window.xdg_toplevel()?;
    let RawDisplayHandle::Wayland(display) = window.display_handle().ok()?.as_raw() else {
        return None;
    };
    // A second handle on winit's connection, with its own event queue.
    // SAFETY: winit's display stays open for as long as the window, which
    // outlives this call, and a foreign backend never closes it.
    let backend = unsafe { Backend::from_foreign_display(display.display.as_ptr().cast()) };
    let conn = Connection::from_backend(backend);
    // SAFETY: winit's xdg_toplevel proxy, alive as long as the window, which
    // outlives this call; the interface is checked against the proxy's.
    let id = unsafe { ObjectId::from_ptr(XdgToplevel::interface(), toplevel.as_ptr().cast()) };
    let toplevel = XdgToplevel::from_id(&conn, id.ok()?).ok()?;

    let (globals, mut queue) = registry_queue_init::<Icon>(&conn).ok()?;
    let qh = queue.handle();
    let manager: XdgToplevelIconManagerV1 = globals.bind(&qh, 1..=1, ()).ok()?;
    let shm: WlShm = globals.bind(&qh, 1..=1, ()).ok()?;
    let pixels = argb8888(&icon.rgba);
    let file = shared_memory(&pixels)?;
    let pool = shm.create_pool(file.as_fd(), pixels.len().try_into().ok()?, &qh, ());
    let stride = side.checked_mul(4)?;
    let buffer = pool.create_buffer(0, side, side, stride, wl_shm::Format::Argb8888, &qh, ());
    let xdg_icon = manager.create_icon(&qh, ());
    xdg_icon.add_buffer(&buffer, 1);
    // Applied with the window's next frame.
    manager.set_icon(&toplevel, Some(&xdg_icon));
    queue.roundtrip(&mut Icon).ok()?;
    // Dropping the connection here frees only this side's handles. No destroy
    // request is ever sent, so the icon and its buffer (which must outlive
    // it) stay with the compositor until the window closes.
    Some(())
}

/// RGBA as wl_shm's ARGB8888: alpha premultiplied, and the 32-bit pixel
/// little-endian, so B, G, R, A in memory.
fn argb8888(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4)
        .flat_map(|p| {
            let a = u16::from(p[3]);
            let pre = |c: u8| ((u16::from(c) * a + 127) / 255) as u8;
            [pre(p[2]), pre(p[1]), pre(p[0]), p[3]]
        })
        .collect()
}

/// The pixels in an anonymous file the compositor maps.
fn shared_memory(bytes: &[u8]) -> Option<File> {
    // SAFETY: plain syscall; a valid descriptor is owned below.
    let fd = unsafe { libc::memfd_create(c"is-alive-icon".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return None;
    }
    // SAFETY: fd was just created and nothing else owns it.
    let mut file = File::from(unsafe { OwnedFd::from_raw_fd(fd) });
    file.write_all(bytes).ok()?;
    Some(file)
}

/// The queue's state. None of these objects' events are needed: the
/// compositor's preferred sizes are only a hint, and wl_buffer.release isn't
/// used for icons.
struct Icon;

impl Dispatch<WlRegistry, GlobalListContents> for Icon {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
delegate_noop!(Icon: ignore WlShm);
delegate_noop!(Icon: ignore WlShmPool);
delegate_noop!(Icon: ignore WlBuffer);
delegate_noop!(Icon: ignore XdgToplevelIconManagerV1);
delegate_noop!(Icon: ignore XdgToplevelIconV1);

#[cfg(test)]
mod tests {
    #[test]
    fn pixels_are_premultiplied_bgra() {
        let rgba = [
            255, 128, 0, 255, // opaque orange stays as is, reordered
            200, 100, 50, 0, // fully transparent goes to zero
            255, 255, 255, 128, // half-transparent white halves
        ];
        assert_eq!(
            super::argb8888(&rgba),
            [0, 128, 255, 255, 0, 0, 0, 0, 128, 128, 128, 128]
        );
    }

    #[test]
    fn the_app_icon_is_square() {
        let i = crate::icon();
        assert_eq!(i.width, i.height);
        assert_eq!(super::argb8888(&i.rgba).len(), i.rgba.len());
    }
}
