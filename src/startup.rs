//! Opening the window, and never failing silently.
//!
//! The window is drawn with OpenGL (glow). Where OpenGL can't start (no
//! OpenGL 2 driver: a virtual machine without 3D, some Remote Desktop
//! sessions, the Microsoft Basic Display Adapter) the app starts itself
//! again with wgpu: Direct3D 12 on Windows, which falls back to Windows' own
//! software renderer where there's no GPU, and Vulkan on Linux. (A new
//! process, because winit allows one event loop per process.) wgpu, once it
//! has opened the window, is kept for the next start. A start that dies
//! without a word (a driver crash) is noticed on the next one, which uses
//! the other renderer; another copy still opening its window at the same
//! time (several files opened at once) isn't taken for one that died.
//! Whatever still stops it (both failing, a bug) is shown in a message box
//! and written to startup.log in the settings folder, with what was tried.
//!
//! `--renderer=glow` or `--renderer=wgpu` (or IS_ALIVE_RENDERER) picks one.

use crate::theme;
use eframe::Renderer;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// Picks the renderer, like `--renderer=`.
const RENDERER_ENV: &str = "IS_ALIVE_RENDERER";
/// Set on the second start: why the first renderer failed.
const FALLBACK_ENV: &str = "IS_ALIVE_FALLBACK";
const ISSUES: &str = "https://github.com/hammerknife404/is-alive/issues";
/// startup.log is cut back to its newest half past this.
const LOG_MAX: usize = 64 * 1024;

#[cfg(windows)]
const EXE: &str = "is-alive.exe";
#[cfg(not(windows))]
const EXE: &str = "is-alive";
#[cfg(windows)]
const WGPU_NOTE: &str = "Direct3D 12; works without a graphics card";
#[cfg(not(windows))]
const WGPU_NOTE: &str = "Vulkan";

/// How this start picked its renderer.
#[derive(Clone, Debug, PartialEq)]
pub enum Why {
    Default,
    /// `--renderer=` or IS_ALIVE_RENDERER.
    Asked,
    /// The other one failed a moment ago, in the first process: its error.
    Fallback(String),
    /// Remembered from a start where the other one failed.
    Remembered,
    /// The last start with the other one stopped before its window opened.
    LastStartDied,
}

impl Why {
    /// Whether a failure may still try the other renderer.
    fn may_fall_back(&self) -> bool {
        matches!(self, Why::Default | Why::Remembered | Why::LastStartDied)
    }

    fn describe(&self) -> String {
        match self {
            Why::Default => "the default".into(),
            Why::Asked => "asked for".into(),
            Why::Fallback(e) => format!("the other one failed: {e}"),
            Why::Remembered => "remembered from an earlier start".into(),
            Why::LastStartDied => "the last start with the other one stopped".into(),
        }
    }
}

/// What this start does.
#[derive(Debug, PartialEq)]
pub struct Plan {
    pub renderer: Renderer,
    pub why: Why,
    /// Files named on the command line ("Open with").
    pub files: Vec<PathBuf>,
}

/// The plan and whether its window is up, for the panic hook.
struct State {
    renderer: Renderer,
    why: Why,
    open: bool,
    /// What draws, as the renderer reports it (the GPU or software
    /// renderer, and its driver), for the log.
    device: Option<String>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static OPEN: AtomicBool = AtomicBool::new(false);

fn other(r: Renderer) -> Renderer {
    match r {
        Renderer::Glow => Renderer::Wgpu,
        Renderer::Wgpu => Renderer::Glow,
    }
}

/// The renderer in plain words, for the log and the message box.
fn name(r: Renderer) -> &'static str {
    match r {
        Renderer::Glow => "OpenGL (glow)",
        Renderer::Wgpu if cfg!(windows) => "Direct3D 12 (wgpu)",
        Renderer::Wgpu => "Vulkan (wgpu)",
    }
}

/// `--renderer=X` / `--renderer X` and the files, from the arguments after
/// the program's name.
fn parse_args(
    args: impl IntoIterator<Item = OsString>,
) -> Result<(Option<Renderer>, Vec<PathBuf>), String> {
    let mut renderer = None;
    let mut files = Vec::new();
    let mut args = args.into_iter();
    while let Some(a) = args.next() {
        let value = match a.to_str() {
            Some("--renderer") => Some(args.next().unwrap_or_default()),
            Some(s) => s.strip_prefix("--renderer=").map(OsString::from),
            None => None,
        };
        match value {
            Some(v) => renderer = Some(parse_renderer(&v.to_string_lossy())?),
            None => files.push(a.into()),
        }
    }
    Ok((renderer, files))
}

fn parse_renderer(v: &str) -> Result<Renderer, String> {
    v.trim().parse().map_err(|_| {
        format!(
            "There's no renderer called \"{v}\". Use --renderer=glow (OpenGL) or --renderer=wgpu."
        )
    })
}

/// The renderer to use, most specific first: the command line, the second
/// start after a failure, the environment, a start that died, a fallback
/// remembered in Settings, then OpenGL.
fn choose(
    flag: Option<Renderer>,
    env: Option<Renderer>,
    fallback: Option<String>,
    died: Option<Renderer>,
    saved: Option<Renderer>,
) -> (Renderer, Why) {
    match (flag, env, fallback, died, saved) {
        (Some(r), ..) => (r, Why::Asked),
        (None, Some(r), Some(e), ..) => (r, Why::Fallback(e)),
        (None, Some(r), None, ..) => (r, Why::Asked),
        (None, None, _, Some(d), _) => (other(d), Why::LastStartDied),
        (None, None, _, None, Some(r)) => (r, Why::Remembered),
        _ => (Renderer::Glow, Why::Default),
    }
}

/// Read the command line, environment and last start; an unknown renderer
/// name is an error for the message box.
pub fn plan(saved: Option<Renderer>) -> Result<Plan, String> {
    let (flag, files) = parse_args(std::env::args_os().skip(1))?;
    let env = match std::env::var(RENDERER_ENV) {
        Ok(v) if !v.trim().is_empty() => Some(parse_renderer(&v)?),
        _ => None,
    };
    let fallback = std::env::var(FALLBACK_ENV).ok();
    // Only looked at (and cleared) when it can decide.
    let died = match (flag, env) {
        (None, None) => theme::data_dir().and_then(|d| died(&d, std::process::id(), alive)),
        _ => None,
    };
    let (renderer, why) = choose(flag, env, fallback, died, saved);
    Ok(Plan {
        renderer,
        why,
        files,
    })
}

/// This start's marker, `starting-<pid>` holding its renderer: present from
/// just before the window opens until its first frame is drawn. Each copy
/// has its own, so one opening at the same time as another neither reads
/// the other's as a death nor clears it.
fn marker() -> Option<PathBuf> {
    Some(theme::data_dir()?.join(format!("{MARKER}{}", std::process::id())))
}

const MARKER: &str = "starting-";

/// The renderer of the newest start that died before its window opened: a
/// marker in `dir` whose process is gone. Those markers are removed; those
/// of copies still running (still opening their windows) are left alone.
fn died(dir: &Path, me: u32, alive: impl Fn(u32) -> bool) -> Option<Renderer> {
    let mut dead = Vec::new();
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let name = e.file_name();
        let Some(pid) = name
            .to_str()
            .and_then(|n| n.strip_prefix(MARKER))
            .and_then(|p| p.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == me || alive(pid) {
            continue;
        }
        let when = e.metadata().and_then(|m| m.modified()).ok();
        let renderer = std::fs::read_to_string(e.path())
            .ok()
            .and_then(|s| s.trim().parse().ok());
        let _ = std::fs::remove_file(e.path());
        if let Some(r) = renderer {
            dead.push((when, r));
        }
    }
    dead.into_iter()
        .max_by_key(|(when, _)| *when)
        .map(|(_, r)| r)
}

/// Whether process `pid` is still running. A pid reused by another program
/// counts as running: the marker then waits for a later start.
#[cfg(windows)]
fn alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    // SAFETY: plain calls on a handle that's checked, then closed.
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return false;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(h, &mut code) != 0;
        CloseHandle(h);
        ok && code == STILL_ACTIVE as u32
    }
}

#[cfg(not(windows))]
fn alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

/// Where the startup log is; shown in the message box and the README.
pub fn log_path() -> Option<PathBuf> {
    Some(theme::data_dir()?.join("startup.log"))
}

/// wgpu's setup. Direct3D 12 compiles its shaders with FXC, which comes
/// with Windows (WGPU_DX12_COMPILER can still pick another): left to itself
/// wgpu loads the first dxcompiler.dll on the search path (an SDK's, another
/// app's), and one too old to work stops wgpu from starting.
pub fn wgpu_options() -> eframe::egui_wgpu::WgpuConfiguration {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut c = eframe::egui_wgpu::WgpuConfiguration::default();
    #[cfg(windows)]
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(s) = &mut c.wgpu_setup
        && eframe::wgpu::Dx12Compiler::from_env().is_none()
    {
        s.instance_descriptor.backend_options.dx12.shader_compiler =
            eframe::wgpu::Dx12Compiler::Fxc;
    }
    c
}

/// Call just before `run_native`.
pub fn begin(plan: &Plan) {
    log(&format!(
        "is-alive {} on {}: opening the window with {} ({})",
        crate::VERSION,
        os(),
        name(plan.renderer),
        plan.why.describe()
    ));
    if let Some(p) = marker() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, plan.renderer.to_string());
    }
    if let Ok(mut s) = STATE.lock() {
        *s = Some(State {
            renderer: plan.renderer,
            why: plan.why.clone(),
            open: false,
            device: None,
        });
    }
}

/// Call from the app creator: note what draws, for the "window open" line
/// (on a virtual machine, whether it's WARP or a virtual GPU's driver).
pub fn created(cc: &eframe::CreationContext<'_>) {
    let device = if let Some(w) = &cc.wgpu_render_state {
        let i = w.adapter.get_info();
        let driver = [i.driver.as_str(), i.driver_info.as_str()]
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(" ");
        Some(format!(
            "{} ({:?}, {:?}{})",
            i.name,
            i.device_type,
            i.backend,
            if driver.is_empty() {
                String::new()
            } else {
                format!(", driver {driver}")
            }
        ))
    } else if let Some(gl) = &cc.gl {
        use eframe::glow::HasContext;
        // SAFETY: the context is current while the app is created.
        let (r, v) = unsafe {
            (
                gl.get_parameter_string(eframe::glow::RENDERER),
                gl.get_parameter_string(eframe::glow::VERSION),
            )
        };
        // OpenGL ES names itself; desktop OpenGL gives just the number.
        let gl = if v.starts_with("OpenGL") {
            ""
        } else {
            "OpenGL "
        };
        Some(format!("{r}, {gl}{v}"))
    } else {
        None
    };
    if let Ok(mut s) = STATE.lock()
        && let Some(s) = s.as_mut()
    {
        s.device = device.map(|d| one_line(&d));
    }
}

/// Call every frame: once the first frame is on screen, note that it
/// worked and remember which renderer did it.
pub fn running(ctx: &eframe::egui::Context, look: &mut theme::Settings) {
    if OPEN.load(Ordering::Relaxed) {
        return;
    }
    // The first frame is drawn after this pass: check on the next one.
    if ctx.cumulative_frame_nr() == 0 {
        ctx.request_repaint();
        return;
    }
    OPEN.store(true, Ordering::Relaxed);
    if let Some(p) = marker() {
        let _ = std::fs::remove_file(p);
    }
    let Some((renderer, device)) = STATE.lock().ok().and_then(|mut s| {
        let s = s.as_mut()?;
        s.open = true;
        Some((s.renderer, s.device.take()))
    }) else {
        return;
    };
    // wgpu opening, however it was picked, is kept for the next start;
    // OpenGL opening goes back to the default.
    let remember = (renderer == Renderer::Wgpu).then_some(renderer);
    let mut line = format!("window open with {}", name(renderer));
    if let Some(d) = device {
        line += &format!(" on {d}");
    }
    if remember != look.renderer {
        look.renderer = remember;
        theme::save(look);
        line += &match remember {
            Some(r) => format!(
                "; it's used from now on (start with --renderer={} to try {} again)",
                other(r),
                name(other(r))
            ),
            None => "; back to the default from now on".into(),
        };
    }
    log(&line);
}

/// `run_native` returned an error: start again with the other renderer if
/// that's still allowed, else say what happened. Doesn't return.
pub fn failed(err: &eframe::Error) -> ! {
    let state = STATE
        .lock()
        .ok()
        .and_then(|s| s.as_ref().map(|s| (s.renderer, s.why.clone(), s.open)));
    let (renderer, why, open) = state.unwrap_or((Renderer::Glow, Why::Default, false));
    let text = one_line(&describe(err));
    // A window system error (no display) isn't the renderer's fault.
    let renderer_fault = !matches!(
        err,
        eframe::Error::Winit(_) | eframe::Error::WinitEventLoop(_) | eframe::Error::AppCreation(_)
    );
    if let Some(p) = marker() {
        let _ = std::fs::remove_file(p);
    }
    if !open && renderer_fault && why.may_fall_back() {
        try_other(renderer, &text);
    }
    log(&format!("{} failed: {text}", name(renderer)));
    if open {
        let what = format!(
            "is-alive {} stopped: {} failed:\n{text}\n{}",
            crate::VERSION,
            name(renderer),
            log_note()
        );
        tell("is-alive stopped", &what);
    } else {
        tell(
            "is-alive could not start",
            &cannot_open(renderer, &why, &text),
        );
    }
    std::process::exit(1);
}

/// The command line or IS_ALIVE_RENDERER names no renderer: say so and exit.
pub fn bad_arguments(error: &str) -> ! {
    log(error);
    tell("is-alive could not start", error);
    std::process::exit(2);
}

/// Start this program again with the other renderer and exit, if it can
/// be started; returns if it can't.
fn try_other(renderer: Renderer, error: &str) {
    let next = other(renderer);
    log(&format!(
        "{} failed before its window opened: {error}; starting again with {}",
        name(renderer),
        name(next)
    ));
    let appdir = std::env::var_os("APPDIR").map(|d| std::fs::canonicalize(&d).unwrap_or(d.into()));
    let exe = relaunch_path(
        std::env::current_exe().ok(),
        std::env::var_os("APPIMAGE"),
        appdir,
    );
    let started = exe.is_some_and(|exe| {
        std::process::Command::new(exe)
            .args(std::env::args_os().skip(1))
            .env(RENDERER_ENV, next.to_string())
            .env(FALLBACK_ENV, error.replace('\0', " "))
            .spawn()
            .is_ok()
    });
    if started {
        // Not a silent death: the next start shouldn't count it as one.
        if let Some(p) = marker() {
            let _ = std::fs::remove_file(p);
        }
        std::process::exit(0);
    }
    log("could not start this program again");
}

/// What to start again: in an AppImage this program lives on a mount that
/// goes away when it exits, so the AppImage itself. APPIMAGE is handed down
/// to everything an AppImage starts (a terminal, a file manager), so it's
/// only this program's when this program is inside APPDIR.
fn relaunch_path(
    exe: Option<PathBuf>,
    appimage: Option<OsString>,
    appdir: Option<PathBuf>,
) -> Option<PathBuf> {
    match (appimage, appdir) {
        (Some(image), Some(dir)) if exe.as_ref().is_some_and(|e| e.starts_with(&dir)) => {
            Some(image.into())
        }
        _ => exe,
    }
}

/// The message box when no renderer could open the window.
fn cannot_open(renderer: Renderer, why: &Why, error: &str) -> String {
    let mut s = format!(
        "is-alive {} could not open its window.\n\n{} failed:\n{error}\n",
        crate::VERSION,
        name(renderer)
    );
    if let Why::Fallback(first) = why {
        s += &format!(
            "\nIt tried {} first, which failed too:\n{first}\n",
            name(other(renderer))
        );
    }
    s += &format!(
        "\nWhat to try:\n\
         - Update the graphics driver. On a virtual machine, turn on 3D acceleration.\n\
         - Start it with the other renderer, from a shortcut or a command prompt:\n    \
         {EXE} --renderer=wgpu   ({WGPU_NOTE})\n    \
         {EXE} --renderer=glow   (OpenGL)\n"
    );
    s + &log_note()
}

fn log_note() -> String {
    match log_path() {
        Some(p) => format!("\nDetails are in {}", p.display()),
        None => String::new(),
    }
}

/// Show a panic (with `panic = "abort"`, every panic ends the app) in a
/// message box and the log, or, before the window is open, start again
/// with the other renderer. Call first thing in main.
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default(info);
        let p = info.payload();
        let msg = p
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| p.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".into());
        let at = info
            .location()
            .map(|l| format!(" (at {}:{})", l.file(), l.line()))
            .unwrap_or_default();
        let what = one_line(&format!("{msg}{at}"));
        // try_lock: the panic may have come from inside a lock of STATE.
        let state = STATE
            .try_lock()
            .ok()
            .and_then(|s| s.as_ref().map(|s| (s.renderer, s.why.clone(), s.open)));
        // Only the main thread opens the window: a worker's panic says
        // nothing about the renderer.
        let main = std::thread::current().name() == Some("main");
        if let Some((renderer, why, false)) = &state
            && main
            && why.may_fall_back()
        {
            try_other(*renderer, &format!("panic: {what}"));
        }
        log(&format!("panic: {what}"));
        let ends_app = cfg!(panic = "abort") || main;
        if ends_app {
            tell(
                "is-alive stopped",
                &format!(
                    "is-alive {} stopped because of a fault in the app:\n\n{what}\n\n\
                     Please report it at {ISSUES} with startup.log.{}",
                    crate::VERSION,
                    log_note()
                ),
            );
        }
    }));
}

/// Show `text` where the user will see it: a message box on Windows; on
/// Linux stderr and, when not started from a terminal, a zenity or
/// kdialog box if there is one.
fn tell(title: &str, text: &str) {
    let _ = writeln!(std::io::stderr(), "{title}\n{text}");
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MessageBoxW,
        };
        let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
        let (text, title) = (wide(text), wide(title));
        // SAFETY: both strings are NUL-terminated and outlive the call.
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
            );
        }
    }
    #[cfg(not(windows))]
    {
        use std::io::IsTerminal;
        if std::io::stderr().is_terminal() || std::env::var_os("IS_ALIVE_SHOTS").is_some() {
            return;
        }
        let zenity = std::process::Command::new("zenity")
            .args(["--error", "--title", title, "--text", &markup_escape(text)])
            .status();
        if zenity.is_err() {
            let _ = std::process::Command::new("kdialog")
                .args(["--title", title, "--error", text])
                .status();
        }
    }
}

/// zenity's text is Pango markup.
#[cfg(not(windows))]
fn markup_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The error in fewer words: glutin's "no config" error lists every field
/// of what it asked for.
fn describe(err: &eframe::Error) -> String {
    match err {
        eframe::Error::NoGlutinConfigs(_, e) => format!("no usable OpenGL setup ({e})"),
        e => e.to_string(),
    }
}

/// Append a timestamped line to startup.log (and stderr), keeping the file
/// small. Best effort: a read-only profile just means no log.
fn log(line: &str) {
    let _ = writeln!(std::io::stderr(), "is-alive: {line}");
    let Some(p) = log_path() else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Trim only when it's grown too big; otherwise append, so the two
    // processes of a fallback never write over each other's lines.
    if std::fs::metadata(&p).is_ok_and(|m| m.len() > LOG_MAX as u64) {
        let mut text = std::fs::read(&p).unwrap_or_default();
        trim_log(&mut text);
        let _ = std::fs::write(&p, text);
    }
    let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p)
    {
        let _ = writeln!(f, "{stamp} {line}");
    }
}

/// Past `LOG_MAX`, drop the oldest lines down to half of it.
fn trim_log(text: &mut Vec<u8>) {
    if text.len() <= LOG_MAX {
        return;
    }
    let from = text.len() - LOG_MAX / 2;
    let cut = text[from..]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(text.len(), |n| from + n + 1);
    text.drain(..cut);
}

/// Errors can run over several lines; the log keeps one per event.
fn one_line(s: &str) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    match s.char_indices().nth(600) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s,
    }
}

/// The system, for the log: version, architecture and anything about the
/// session that matters to graphics.
fn os() -> String {
    format!("{} ({})", os_name(), std::env::consts::ARCH)
}

#[cfg(windows)]
fn os_name() -> String {
    use windows_sys::Win32::System::SystemInformation::{GetVersionExW, OSVERSIONINFOW};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_REMOTESESSION};
    // SAFETY: a zeroed OSVERSIONINFOW with its size set is what it expects.
    // The manifest says Windows 10/11, so the real version comes back.
    let mut v: OSVERSIONINFOW = unsafe { std::mem::zeroed() };
    v.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOW>() as u32;
    let mut s = if unsafe { GetVersionExW(&mut v) } != 0 {
        // Windows 11 still says 10.0; its builds start at 22000.
        let eleven = if v.dwMajorVersion == 10 && v.dwBuildNumber >= 22000 {
            " 11"
        } else {
            ""
        };
        format!(
            "Windows{eleven} {}.{} build {}",
            v.dwMajorVersion, v.dwMinorVersion, v.dwBuildNumber
        )
    } else {
        "Windows".to_string()
    };
    // SAFETY: no arguments to get wrong.
    if unsafe { GetSystemMetrics(SM_REMOTESESSION) } != 0 {
        s += ", Remote Desktop session";
    }
    s
}

#[cfg(not(windows))]
fn os_name() -> String {
    let pretty = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|t| {
            t.lines()
                .find_map(|l| l.strip_prefix("PRETTY_NAME="))
                .map(|v| v.trim_matches('"').to_string())
        })
        .unwrap_or_else(|| std::env::consts::OS.to_string());
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    let session = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        "Wayland"
    } else if std::env::var_os("DISPLAY").is_some() {
        "X11"
    } else {
        "no display"
    };
    format!("{pretty}, kernel {}, {session}", kernel.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Result<(Option<Renderer>, Vec<PathBuf>), String> {
        parse_args(a.iter().map(OsString::from))
    }

    #[test]
    fn renderer_flag_is_not_a_file() {
        assert_eq!(
            args(&["--renderer=wgpu", "a.json"]),
            Ok((Some(Renderer::Wgpu), vec!["a.json".into()]))
        );
        assert_eq!(
            args(&["b.exp", "--renderer", "GLOW"]),
            Ok((Some(Renderer::Glow), vec!["b.exp".into()]))
        );
        assert_eq!(args(&["c.dc.html"]), Ok((None, vec!["c.dc.html".into()])));
        assert!(
            args(&["--renderer=vulkan"])
                .unwrap_err()
                .contains("--renderer=wgpu")
        );
        assert!(args(&["--renderer"]).is_err());
    }

    #[test]
    fn the_most_specific_choice_wins() {
        use Renderer::{Glow, Wgpu};
        let pick = |f, e, fb: Option<&str>, d, s| choose(f, e, fb.map(String::from), d, s);
        assert_eq!(pick(None, None, None, None, None), (Glow, Why::Default));
        assert_eq!(
            pick(None, None, None, None, Some(Wgpu)),
            (Wgpu, Why::Remembered)
        );
        // A start with the remembered one died: the other one this time.
        assert_eq!(
            pick(None, None, None, Some(Wgpu), Some(Wgpu)),
            (Glow, Why::LastStartDied)
        );
        assert_eq!(
            pick(None, None, None, Some(Glow), None),
            (Wgpu, Why::LastStartDied)
        );
        assert_eq!(
            pick(None, Some(Wgpu), Some("no GL"), Some(Glow), None),
            (Wgpu, Why::Fallback("no GL".into()))
        );
        assert_eq!(
            pick(None, Some(Wgpu), None, Some(Glow), None),
            (Wgpu, Why::Asked)
        );
        assert_eq!(
            pick(Some(Glow), Some(Wgpu), Some("x"), Some(Glow), Some(Wgpu)),
            (Glow, Why::Asked)
        );
    }

    #[test]
    fn only_a_start_that_is_gone_counts_as_died() {
        let dir = std::env::temp_dir().join(format!("is-alive-starting-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let put = |name: &str, text: &str| std::fs::write(dir.join(name), text).unwrap();
        let has = |name: &str| dir.join(name).exists();
        // 10 is running (another copy opening its window), 7 is this one.
        let running = |pid| pid == 10;
        put("starting-10", "glow");
        put("starting-7", "glow");
        put("starting", "glow");
        put("starting-x", "glow");
        assert_eq!(died(&dir, 7, running), None);
        assert!(has("starting-10") && has("starting-7"));
        // 11 is gone: its renderer died, and its marker is cleared.
        put("starting-11", "wgpu");
        assert_eq!(died(&dir, 7, running), Some(Renderer::Wgpu));
        assert!(!has("starting-11") && has("starting-10"));
        assert_eq!(died(&dir, 7, running), None);
        // 10 has gone too.
        assert_eq!(died(&dir, 7, |_| false), Some(Renderer::Glow));
        assert!(!has("starting-10"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn this_process_is_alive() {
        assert!(alive(std::process::id()));
        assert!(!alive(u32::MAX - 1));
    }

    #[test]
    fn appimage_only_when_running_from_it() {
        let exe = Some(PathBuf::from("/tmp/.mount_isaXYZ/usr/bin/is-alive"));
        let image = || Some(OsString::from("/home/u/is-alive.AppImage"));
        assert_eq!(
            relaunch_path(exe.clone(), image(), Some("/tmp/.mount_isaXYZ".into())),
            Some("/home/u/is-alive.AppImage".into())
        );
        // Started from another AppImage's terminal: its APPIMAGE and APPDIR.
        let built = Some(PathBuf::from("/home/u/is-alive/target/release/is-alive"));
        assert_eq!(
            relaunch_path(built.clone(), image(), Some("/tmp/.mount_termAB".into())),
            built
        );
        assert_eq!(relaunch_path(built.clone(), image(), None), built);
        assert_eq!(relaunch_path(exe.clone(), None, None), exe);
    }

    #[test]
    fn only_an_automatic_choice_falls_back() {
        assert!(Why::Default.may_fall_back());
        assert!(Why::Remembered.may_fall_back());
        assert!(Why::LastStartDied.may_fall_back());
        assert!(!Why::Asked.may_fall_back());
        assert!(!Why::Fallback("x".into()).may_fall_back());
    }

    #[test]
    fn log_keeps_its_newest_whole_lines() {
        let mut t: Vec<u8> = (0..20_000)
            .flat_map(|n| format!("line {n}\n").into_bytes())
            .collect();
        trim_log(&mut t);
        let s = String::from_utf8(t).unwrap();
        assert!(s.len() <= LOG_MAX / 2 && s.len() > LOG_MAX / 2 - 20);
        assert!(s.starts_with("line ") && s.ends_with("line 19999\n"));
        let mut short = b"a\nb\n".to_vec();
        trim_log(&mut short);
        assert_eq!(short, b"a\nb\n");
    }

    #[test]
    fn errors_fit_on_one_line() {
        assert_eq!(one_line("a\n  b\tc "), "a b c");
        let long = "é".repeat(700);
        assert_eq!(one_line(&long).chars().count(), 601);
    }

    #[test]
    fn the_message_says_how_to_switch() {
        let m = cannot_open(
            Renderer::Wgpu,
            &Why::Fallback("no OpenGL".into()),
            "no adapter",
        );
        assert!(m.contains("no adapter") && m.contains("no OpenGL"));
        assert!(m.contains("--renderer=wgpu") && m.contains("--renderer=glow"));
    }
}
