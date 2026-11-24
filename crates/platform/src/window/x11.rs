//! Linux: windows on the X display (an X11 session, or XWayland under Wayland), listed and pictured
//! over the X protocol itself, so nothing native has to be built or loaded. Wayland lets a program
//! picture another's window only through the desktop's screenshot portal, which isn't built in.

use super::{Candidate, WindowShot, choose, no_window};
use craft_anything_evidence::{ErrorCode, ToolError};
use x11rb::connection::Connection;
use x11rb::errors::ReplyError;
use x11rb::image::{Image, PixelLayout};
use x11rb::protocol::ErrorKind;
use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt, GetPropertyReply, MapState, Window};
use x11rb::rust_connection::RustConnection;

/// Most windows looked at: a desktop has far fewer, and each costs a few round trips.
const MAX_WINDOWS: usize = 4096;

fn on_wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some_and(|d| !d.is_empty())
}

pub(super) fn connect() -> Result<(RustConnection, usize), ToolError> {
    RustConnection::connect(None).map_err(|e| {
        if on_wayland() {
            ToolError::new(
                ErrorCode::FeatureMissing,
                "This Wayland desktop has no X display (XWayland), and Wayland lets a program picture another's window only through the desktop's screenshot portal, which isn't built in yet.",
            )
            .with_remediation("Run it in an X11 session or with XWayland on, or add pictures of the original yourself (screenshots).")
        } else {
            ToolError::new(ErrorCode::FeatureMissing, format!("There's no X display to picture windows on: {e}."))
                .with_remediation("Run it from a desktop session (DISPLAY set), or add pictures of the original yourself (screenshots).")
        }
    })
}

fn x_error(e: impl std::fmt::Display) -> ToolError {
    ToolError::new(ErrorCode::ProgramFailed, format!("The X display didn't answer: {e}"))
}

/// A window can close between being listed and pictured. The server then refuses the request
/// for that window, which means it's gone (look again, or at another process's window), not that
/// the display failed; anything else is `failed`.
fn gone_or(e: ReplyError, failed: impl FnOnce(ReplyError) -> ToolError) -> ToolError {
    match &e {
        ReplyError::X11Error(x) if matches!(x.error_kind, ErrorKind::Window | ErrorKind::Drawable | ErrorKind::Match) => {
            ToolError::new(ErrorCode::NotFound, "The program's window closed before it could be pictured.")
        }
        _ => failed(e),
    }
}

/// A window property, or `None` when it isn't set or the window has gone.
fn property(conn: &RustConnection, window: Window, name: Atom, kind: impl Into<Atom>, words: u32) -> Option<GetPropertyReply> {
    conn.get_property(false, window, name, kind, 0, words).ok()?.reply().ok().filter(|p| p.format != 0)
}

/// `WM_CLASS` holds the instance and class names, each ending in a NUL; the class names the program.
fn class_name(value: &[u8]) -> String {
    let mut names = value.split(|b| *b == 0).filter(|n| !n.is_empty());
    let instance = names.next();
    names.next().or(instance).map(|n| String::from_utf8_lossy(n).into_owned()).unwrap_or_default()
}

/// The part of a window at (`x`, `y`) on the root that the screen shows, relative to the window:
/// the X server refuses to picture past the screen's edge.
fn on_screen(x: i16, y: i16, width: u32, height: u32, screen_width: u16, screen_height: u16) -> Option<(i16, i16, u16, u16)> {
    let clip = |at: i16, len: u32, screen: u16| {
        let start = i64::from(at).max(0);
        let end = (i64::from(at) + i64::from(len)).min(i64::from(screen));
        if end <= start {
            return None;
        }
        Some((i16::try_from(start - i64::from(at)).ok()?, u16::try_from(end - start).ok()?))
    };
    let (left, shown_width) = clip(x, width, screen_width)?;
    let (top, shown_height) = clip(y, height, screen_height)?;
    Some((left, top, shown_width, shown_height))
}

pub(super) fn capture(pid: u32, app_name: Option<&str>) -> Result<WindowShot, ToolError> {
    let (conn, screen) = connect()?;
    let screen = conn.setup().roots.get(screen).cloned().ok_or_else(|| x_error("it has no such screen"))?;
    let atom = |name: &str| conn.intern_atom(false, name.as_bytes()).map_err(x_error)?.reply().map(|r| r.atom).map_err(x_error);
    let (client_list, wm_pid, wm_name, utf8) = (atom("_NET_CLIENT_LIST")?, atom("_NET_WM_PID")?, atom("_NET_WM_NAME")?, atom("UTF8_STRING")?);
    let listed = property(&conn, screen.root, client_list, AtomEnum::WINDOW, u32::try_from(MAX_WINDOWS).unwrap_or(u32::MAX))
        .and_then(|p| p.value32().map(Iterator::collect::<Vec<_>>))
        .unwrap_or_default();
    // Without a window manager that lists them, programs' windows are the root's children.
    let ids = if listed.is_empty() { conn.query_tree(screen.root).map_err(x_error)?.reply().map_err(x_error)?.children } else { listed };
    let windows: Vec<(Window, Candidate)> = ids
        .into_iter()
        .take(MAX_WINDOWS)
        .filter_map(|id| {
            // A window closed while we look makes these fail; it's then left out.
            let attributes = conn.get_window_attributes(id).ok()?.reply().ok()?;
            let geometry = conn.get_geometry(id).ok()?.reply().ok()?;
            let pid = property(&conn, id, wm_pid, AtomEnum::CARDINAL, 1).and_then(|p| p.value32()?.next());
            let app = property(&conn, id, AtomEnum::WM_CLASS.into(), AtomEnum::STRING, 256).map(|p| class_name(&p.value)).unwrap_or_default();
            let shown = attributes.map_state == MapState::VIEWABLE;
            Some((id, Candidate { pid, app, width: geometry.width.into(), height: geometry.height.into(), shown }))
        })
        .collect();
    let candidates: Vec<Candidate> = windows.iter().map(|(_, c)| c.clone()).collect();
    let Some((id, chosen)) = choose(&candidates, pid, app_name).and_then(|i| windows.get(i)) else {
        let missing = no_window();
        return Err(if on_wayland() {
            missing.with_remediation("Give it longer to start (wait_secs). On Wayland only windows drawn through XWayland can be pictured; a program that draws for Wayland itself can't be yet.")
        } else {
            missing
        });
    };
    let at = conn.translate_coordinates(*id, screen.root, 0, 0).map_err(x_error)?.reply().map_err(|e| gone_or(e, x_error))?;
    let (x, y, width, height) = on_screen(at.dst_x, at.dst_y, chosen.width, chosen.height, screen.width_in_pixels, screen.height_in_pixels)
        .ok_or_else(|| ToolError::new(ErrorCode::NotFound, "The program's window is off the screen, so it can't be pictured."))?;
    let (image, visual) = Image::get(&conn, *id, x, y, width, height)
        .map_err(|e| gone_or(e, |e| ToolError::new(ErrorCode::ProgramFailed, format!("Couldn't picture the window: {e}"))))?;
    let layout = screen
        .allowed_depths
        .iter()
        .flat_map(|d| &d.visuals)
        .find(|v| v.visual_id == visual)
        .and_then(|v| PixelLayout::from_visual_type(*v).ok())
        .ok_or_else(|| ToolError::new(ErrorCode::FeatureMissing, "The window's colours come from a palette, which can't be pictured yet."))?;
    let mut rgba = Vec::with_capacity(usize::from(width) * usize::from(height) * 4);
    for row in 0..height {
        for column in 0..width {
            let (r, g, b) = layout.decode(image.get_pixel(column, row));
            rgba.extend([r.to_be_bytes()[0], g.to_be_bytes()[0], b.to_be_bytes()[0], u8::MAX]);
        }
    }
    let title = property(&conn, *id, wm_name, utf8, 1024)
        .or_else(|| property(&conn, *id, AtomEnum::WM_NAME.into(), AtomEnum::ANY, 1024))
        .map(|p| String::from_utf8_lossy(&p.value).into_owned())
        .unwrap_or_default();
    Ok(WindowShot { title, app: chosen.app.clone(), width: width.into(), height: height.into(), rgba })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_class_names_the_program() {
        assert_eq!(class_name(b"mygame\0MyGame\0"), "MyGame");
        assert_eq!(class_name(b"mygame\0\0"), "mygame", "only an instance name");
        assert_eq!(class_name(b""), "");
    }

    /// A window that closed while it was looked at is "no window" (so the search goes on to other
    /// processes and looks again), but another refusal is still a failure.
    #[test]
    fn a_window_that_closed_meanwhile_is_not_found() {
        let refused = |error_kind| {
            ReplyError::X11Error(x11rb::x11_utils::X11Error {
                error_kind,
                error_code: 0,
                sequence: 0,
                bad_value: 0,
                minor_opcode: 0,
                major_opcode: 0,
                extension_name: None,
                request_name: None,
            })
        };
        for kind in [ErrorKind::Window, ErrorKind::Drawable, ErrorKind::Match] {
            assert_eq!(gone_or(refused(kind), x_error).code, ErrorCode::NotFound, "{kind:?}");
        }
        assert_eq!(gone_or(refused(ErrorKind::Alloc), x_error).code, ErrorCode::ProgramFailed);
    }

    #[test]
    fn only_the_part_on_screen_is_pictured() {
        assert_eq!(on_screen(100, 50, 640, 480, 1920, 1080), Some((0, 0, 640, 480)));
        let (left, top, width, height) = on_screen(-100, 900, 640, 480, 1920, 1080).unwrap();
        assert!(left > 0 && top == 0, "starts where the screen does");
        assert!(width < 640 && height < 480, "cut at the edges");
        assert_eq!(on_screen(2000, 0, 640, 480, 1920, 1080), None, "entirely off the screen");
    }
}
