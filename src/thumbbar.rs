//! Windows taskbar thumbnail toolbar: Like, Previous, Play/Pause and Next
//! under the window preview (hover the taskbar button), like Spotify.
//!
//! The buttons are added once the taskbar announces the button
//! (`TaskbarButtonCreated`), clicks arrive as `WM_COMMAND` in a window
//! subclass and are queued for the app's poll. Icons are drawn here
//! (white, anti-aliased) at the system's small-icon size.

use std::sync::Mutex;
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{
    DefSubclassProc, ITaskbarList3, SetWindowSubclass, TaskbarList, THBF_ENABLED, THB_FLAGS, THB_ICON, THB_TOOLTIP, THUMBBUTTON,
};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, DestroyIcon, GetSystemMetrics, RegisterWindowMessageW, HICON, ICONINFO, SM_CXSMICON, WM_COMMAND};

/// Button ids (also the click codes handed to the app).
pub const LIKE: u32 = 0;
pub const PREV: u32 = 1;
pub const PLAY: u32 = 2;
pub const NEXT: u32 = 3;

const THBN_CLICKED: u32 = 0x1800;
const SUBCLASS_ID: usize = 0x6c79_7068; // "glyph"

struct State {
    hwnd: isize,
    added: bool,
    playing: bool,
    liked: bool,
    tips: [String; 4],
    created_msg: u32,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static CLICKS: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// Clicks since the last call (button ids).
pub fn take_clicks() -> Vec<u32> {
    std::mem::take(&mut *CLICKS.lock().unwrap())
}

/// Hooks the window; buttons appear as soon as the taskbar is ready.
/// `tips`: tooltips for like, previous, play/pause, next.
pub fn init(hwnd: usize, tips: [String; 4]) {
    let created_msg = unsafe { RegisterWindowMessageW(w!("TaskbarButtonCreated")) };
    *STATE.lock().unwrap() = Some(State { hwnd: hwnd as isize, added: false, playing: false, liked: false, tips, created_msg });
    unsafe {
        let _ = SetWindowSubclass(HWND(hwnd as *mut _), Some(subclass), SUBCLASS_ID, 0);
    }
    // The button may exist already (window shown before we hooked it).
    apply();
}

/// New play / like state: swaps the icons.
pub fn set_state(playing: bool, liked: bool) {
    let changed = {
        let mut g = STATE.lock().unwrap();
        match g.as_mut() {
            Some(s) if s.playing != playing || s.liked != liked => {
                s.playing = playing;
                s.liked = liked;
                true
            }
            _ => false,
        }
    };
    if changed {
        apply();
    }
}

unsafe extern "system" fn subclass(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
    let created = STATE.lock().ok().and_then(|g| g.as_ref().map(|s| s.created_msg)).unwrap_or(0);
    if created != 0 && msg == created {
        // Taskbar (re)created our button, e.g. after Explorer restarted.
        if let Some(s) = STATE.lock().unwrap().as_mut() {
            s.added = false;
        }
        apply();
    } else if msg == WM_COMMAND && ((wparam.0 >> 16) & 0xffff) as u32 == THBN_CLICKED {
        let id = (wparam.0 & 0xffff) as u32;
        if id <= NEXT {
            CLICKS.lock().unwrap().push(id);
        }
    }
    DefSubclassProc(hwnd, msg, wparam, lparam)
}

/// Adds (first time) or updates the four buttons.
fn apply() {
    let (hwnd, added, playing, liked, tips) = {
        let g = STATE.lock().unwrap();
        let Some(s) = g.as_ref() else { return };
        (s.hwnd, s.added, s.playing, s.liked, s.tips.clone())
    };
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.clamp(16, 48) as usize;
    let icons = [
        icon(size, if liked { Glyph::HeartFull } else { Glyph::Heart }),
        icon(size, Glyph::Prev),
        icon(size, if playing { Glyph::Pause } else { Glyph::Play }),
        icon(size, Glyph::Next),
    ];
    let mut buttons: Vec<THUMBBUTTON> = Vec::new();
    for (i, ic) in icons.iter().enumerate() {
        let mut tip = [0u16; 260];
        for (j, c) in tips[i].encode_utf16().take(259).enumerate() {
            tip[j] = c;
        }
        buttons.push(THUMBBUTTON {
            dwMask: THB_ICON | THB_TOOLTIP | THB_FLAGS,
            iId: i as u32,
            iBitmap: 0,
            hIcon: ic.unwrap_or_default(),
            szTip: tip,
            dwFlags: THBF_ENABLED,
        });
    }
    let ok = unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let r: windows::core::Result<()> = (|| {
            let tb: ITaskbarList3 = CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER)?;
            tb.HrInit()?;
            let h = HWND(hwnd as *mut _);
            if added {
                tb.ThumbBarUpdateButtons(h, &buttons)
            } else {
                tb.ThumbBarAddButtons(h, &buttons)
            }
        })();
        r.is_ok()
    };
    // Windows copies the icons; ours can go.
    for ic in icons.into_iter().flatten() {
        unsafe {
            let _ = DestroyIcon(ic);
        }
    }
    if ok {
        if let Some(s) = STATE.lock().unwrap().as_mut() {
            s.added = true;
        }
    }
}

#[derive(Clone, Copy)]
enum Glyph {
    Heart,
    HeartFull,
    Prev,
    Next,
    Play,
    Pause,
}

/// Point inside triangle (a, b, c)?
fn in_tri(p: (f32, f32), a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let s = |p: (f32, f32), q: (f32, f32), r: (f32, f32)| (p.0 - r.0) * (q.1 - r.1) - (q.0 - r.0) * (p.1 - r.1);
    let (d1, d2, d3) = (s(p, a, b), s(p, b, c), s(p, c, a));
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

fn in_rect(p: (f32, f32), x0: f32, y0: f32, x1: f32, y1: f32) -> bool {
    p.0 >= x0 && p.0 <= x1 && p.1 >= y0 && p.1 <= y1
}

/// Heart curve, `scale` 1 = full size.
fn in_heart(p: (f32, f32), scale: f32) -> bool {
    let x = (p.0 - 0.5) / (0.36 * scale);
    let y = -(p.1 - 0.54) / (0.36 * scale);
    let a = x * x + y * y - 1.0;
    a * a * a - x * x * y * y * y <= 0.0
}

fn inside(g: Glyph, p: (f32, f32)) -> bool {
    match g {
        Glyph::Play => in_tri(p, (0.3, 0.2), (0.3, 0.8), (0.8, 0.5)),
        Glyph::Pause => in_rect(p, 0.26, 0.2, 0.42, 0.8) || in_rect(p, 0.58, 0.2, 0.74, 0.8),
        Glyph::Prev => in_rect(p, 0.2, 0.22, 0.3, 0.78) || in_tri(p, (0.78, 0.22), (0.78, 0.78), (0.3, 0.5)),
        Glyph::Next => in_rect(p, 0.7, 0.22, 0.8, 0.78) || in_tri(p, (0.22, 0.22), (0.22, 0.78), (0.7, 0.5)),
        Glyph::HeartFull => in_heart(p, 1.0),
        Glyph::Heart => in_heart(p, 1.0) && !in_heart(p, 0.62),
    }
}

/// A white, anti-aliased icon of `size` px (4x4 samples per pixel).
fn icon(size: usize, g: Glyph) -> Option<HICON> {
    let mut px = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let mut hit = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let p = ((x as f32 + (sx as f32 + 0.5) / 4.0) / size as f32, (y as f32 + (sy as f32 + 0.5) / 4.0) / size as f32);
                    if inside(g, p) {
                        hit += 1;
                    }
                }
            }
            let a = (hit * 255 / 16) as u8;
            let i = (y * size + x) * 4;
            // Premultiplied BGRA, white.
            px[i] = a;
            px[i + 1] = a;
            px[i + 2] = a;
            px[i + 3] = a;
        }
    }
    unsafe {
        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size as i32,
            biHeight: -(size as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let color = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        std::ptr::copy_nonoverlapping(px.as_ptr(), bits as *mut u8, px.len());
        let mask = CreateBitmap(size as i32, size as i32, 1, 1, None);
        let info = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info).ok();
        let _ = DeleteObject(HGDIOBJ(color.0));
        let _ = DeleteObject(HGDIOBJ(mask.0));
        icon
    }
}
