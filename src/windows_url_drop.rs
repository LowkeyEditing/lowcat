use std::{
    cell::{Cell, RefCell},
    ffi::c_void,
    path::PathBuf,
    sync::OnceLock,
};

use futures::channel::mpsc;
use gpui::{Hsla, Window};
use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
use windows::{
    Win32::{
        Foundation::{
            COLORREF, HGLOBAL, HINSTANCE, HWND, LPARAM, LRESULT, POINT, POINTL, RECT, WPARAM,
        },
        Graphics::Gdi::{
            BeginPaint, ClientToScreen, CreateSolidBrush, DeleteObject, EndPaint, FillRect,
            PAINTSTRUCT, ScreenToClient, UpdateWindow,
        },
        System::{
            Com::{DVASPECT_CONTENT, FORMATETC, IDataObject, STGMEDIUM, TYMED_HGLOBAL},
            DataExchange::RegisterClipboardFormatW,
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
            Ole::{
                CF_HDROP, CF_UNICODETEXT, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE,
                IDropTarget, IDropTarget_Impl, RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop,
            },
            SystemServices::MODIFIERKEYS_FLAGS,
        },
        UI::{
            HiDpi::GetDpiForWindow,
            Shell::{DragQueryFileW, HDROP},
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_HINSTANCE, GWLP_USERDATA,
                GetClientRect, GetWindowLongPtrW, HWND_TOP, LWA_ALPHA, RegisterClassW,
                SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW,
                SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos, WM_ERASEBKGND,
                WM_PAINT, WNDCLASSW, WS_DISABLED, WS_EX_LAYERED, WS_EX_NOACTIVATE,
                WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
            },
        },
    },
    core::{Ref, implement, w},
};

use crate::model::Category;

const DRAGDROP_GET_FILES_COUNT: u32 = 0xFFFF_FFFF;

#[derive(Clone, Debug)]
pub enum WindowsDropPayload {
    Url(String),
    Paths(Vec<PathBuf>),
}

#[derive(Clone, Debug)]
pub enum WindowsDropEvent {
    Drop {
        category: Option<Category>,
        payload: WindowsDropPayload,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DropDestination {
    Category(Category),
    ActiveCategory,
}

pub fn install(
    window: &mut Window,
    category_hover_color: Hsla,
) -> mpsc::UnboundedReceiver<WindowsDropEvent> {
    let (tx, rx) = mpsc::unbounded();
    let Ok(handle) = window.window_handle() else {
        debug_url_drop(|| "install_failed reason=no_window_handle".to_string());
        return rx;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        debug_url_drop(|| "install_failed reason=not_win32_handle".to_string());
        return rx;
    };
    let hwnd = HWND(handle.hwnd.get() as *mut c_void);
    let target: IDropTarget = WindowsUrlDropTarget {
        hwnd,
        tx,
        payload: RefCell::new(None),
        hovered_destination: Cell::new(None),
        hover_overlay: Cell::new(None),
        category_hover_color: colorref(category_hover_color),
    }
    .into();

    unsafe {
        if let Err(error) = RevokeDragDrop(hwnd) {
            debug_url_drop(|| format!("revoke_gpui_target_failed error={error}"));
            return rx;
        }
        if let Err(error) = RegisterDragDrop(hwnd, &target) {
            debug_url_drop(|| format!("install_failed error={error}"));
            return rx;
        }
    }
    debug_url_drop(|| "installed formats=url,text,files".to_string());
    rx
}

#[implement(IDropTarget)]
struct WindowsUrlDropTarget {
    hwnd: HWND,
    tx: mpsc::UnboundedSender<WindowsDropEvent>,
    payload: RefCell<Option<WindowsDropPayload>>,
    hovered_destination: Cell<Option<DropDestination>>,
    hover_overlay: Cell<Option<HWND>>,
    category_hover_color: COLORREF,
}

impl WindowsUrlDropTarget {
    fn update_hover(&self, point: &POINTL, effect: *mut DROPEFFECT) {
        let destination = self
            .payload
            .borrow()
            .as_ref()
            .and_then(|_| destination_at_point(self.hwnd, point));
        unsafe {
            *effect = if destination.is_some() {
                DROPEFFECT_COPY
            } else {
                DROPEFFECT_NONE
            };
        }
        if self.hovered_destination.replace(destination) != destination {
            let category = destination.and_then(|destination| match destination {
                DropDestination::Category(category) => Some(category),
                DropDestination::ActiveCategory => None,
            });
            self.update_native_hover_overlay(category);
            debug_url_drop(|| {
                format!(
                    "native_hover destination={destination:?} thread={:?}",
                    std::thread::current().id()
                )
            });
        }
    }

    fn clear(&self) {
        self.destroy_native_hover_overlay();
        self.payload.borrow_mut().take();
        if self.hovered_destination.take().is_some() {
            debug_url_drop(|| {
                format!(
                    "native_hover destination=None thread={:?}",
                    std::thread::current().id()
                )
            });
        }
    }

    fn update_native_hover_overlay(&self, category: Option<Category>) {
        let Some(category) = category else {
            self.destroy_native_hover_overlay();
            return;
        };

        let mut bounds = RECT::default();
        if unsafe { GetClientRect(self.hwnd, &mut bounds) }.is_err() {
            return;
        }
        let mut origin = POINT {
            x: bounds.left,
            y: bounds.top,
        };
        if !unsafe { ClientToScreen(self.hwnd, &mut origin) }.as_bool() {
            return;
        }

        let width = bounds.right - bounds.left;
        let (category_top, category_height) = category_row_metrics(self.hwnd);
        let category_index = Category::ALL
            .iter()
            .position(|candidate| *candidate == category)
            .unwrap_or_default() as i32;
        let category_count = Category::ALL.len() as i32;
        let left = width * category_index / category_count;
        let right = width * (category_index + 1) / category_count;

        let overlay = if let Some(overlay) = self.hover_overlay.get() {
            overlay
        } else {
            let Some(instance) = register_hover_overlay_class(self.hwnd) else {
                debug_url_drop(|| "native_hover_overlay class_failed".to_string());
                return;
            };
            let Ok(overlay) = (unsafe {
                CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                    w!("Lowcat-Native-Drop-Hover"),
                    w!(""),
                    WS_POPUP | WS_DISABLED,
                    0,
                    0,
                    0,
                    0,
                    Some(self.hwnd),
                    None,
                    Some(instance),
                    None,
                )
            }) else {
                debug_url_drop(|| "native_hover_overlay create_failed".to_string());
                return;
            };
            let opacity = (crate::ui::CATEGORY_DRAG_HOVER_OPACITY * 255.).round() as u8;
            if unsafe { SetLayeredWindowAttributes(overlay, COLORREF(0), opacity, LWA_ALPHA) }
                .is_err()
            {
                unsafe { DestroyWindow(overlay) }.ok();
                debug_url_drop(|| "native_hover_overlay alpha_failed".to_string());
                return;
            }
            unsafe {
                SetWindowLongPtrW(overlay, GWLP_USERDATA, self.category_hover_color.0 as isize);
            }
            self.hover_overlay.set(Some(overlay));
            overlay
        };

        if unsafe {
            SetWindowPos(
                overlay,
                Some(HWND_TOP),
                origin.x + left,
                origin.y + category_top,
                right - left,
                category_height,
                SWP_NOACTIVATE,
            )
        }
        .is_err()
        {
            debug_url_drop(|| "native_hover_overlay position_failed".to_string());
            return;
        }

        if unsafe {
            SetWindowPos(
                overlay,
                None,
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_SHOWWINDOW,
            )
        }
        .is_err()
        {
            debug_url_drop(|| "native_hover_overlay show_failed".to_string());
            return;
        }
        if !unsafe { UpdateWindow(overlay) }.as_bool() {
            debug_url_drop(|| "native_hover_overlay paint_failed".to_string());
        }
    }

    fn destroy_native_hover_overlay(&self) {
        if let Some(overlay) = self.hover_overlay.take() {
            unsafe { DestroyWindow(overlay) }.ok();
        }
    }
}

#[allow(non_snake_case)]
impl IDropTarget_Impl for WindowsUrlDropTarget_Impl {
    fn DragEnter(
        &self,
        data_object: Ref<IDataObject>,
        _key_state: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let payload = data_object
            .ok()
            .ok()
            .and_then(|data| dropped_payload(&data));
        debug_url_drop(|| format!("drag_enter payload={payload:?}"));
        *self.payload.borrow_mut() = payload;
        self.update_hover(point, effect);
        Ok(())
    }

    fn DragOver(
        &self,
        _key_state: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        self.update_hover(point, effect);
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        debug_url_drop(|| "drag_leave".to_string());
        self.clear();
        Ok(())
    }

    fn Drop(
        &self,
        data_object: Ref<IDataObject>,
        _key_state: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let payload = data_object
            .ok()
            .ok()
            .and_then(|data| dropped_payload(&data))
            .or_else(|| self.payload.borrow().clone());
        let destination = destination_at_point(self.hwnd, point);
        let accepted = payload.is_some() && destination.is_some();
        unsafe {
            *effect = if accepted {
                DROPEFFECT_COPY
            } else {
                DROPEFFECT_NONE
            };
        }
        debug_url_drop(|| {
            format!("drop destination={destination:?} payload={payload:?} accepted={accepted}")
        });
        self.clear();
        if let (Some(destination), Some(payload)) = (destination, payload) {
            let category = match destination {
                DropDestination::Category(category) => Some(category),
                DropDestination::ActiveCategory => None,
            };
            let _ = self
                .tx
                .unbounded_send(WindowsDropEvent::Drop { category, payload });
        }
        Ok(())
    }
}

fn dropped_payload(data: &IDataObject) -> Option<WindowsDropPayload> {
    for (format, encoding) in text_formats() {
        if let Some(text) = read_text_format(data, format, encoding)
            && let Some(url) = crate::ui::drop_overlay::extract_dropped_youtube_url(&text)
        {
            return Some(WindowsDropPayload::Url(url));
        }
    }
    read_file_paths(data).map(WindowsDropPayload::Paths)
}

#[derive(Clone, Copy)]
enum TextEncoding {
    Utf16,
    Bytes,
}

fn text_formats() -> [(u16, TextEncoding); 6] {
    [
        (
            registered_format(w!("UniformResourceLocatorW")),
            TextEncoding::Utf16,
        ),
        (
            registered_format(w!("UniformResourceLocator")),
            TextEncoding::Bytes,
        ),
        (registered_format(w!("text/uri-list")), TextEncoding::Bytes),
        (registered_format(w!("text/x-moz-url")), TextEncoding::Utf16),
        (CF_UNICODETEXT.0, TextEncoding::Utf16),
        (registered_format(w!("HTML Format")), TextEncoding::Bytes),
    ]
}

fn registered_format(name: windows::core::PCWSTR) -> u16 {
    unsafe { RegisterClipboardFormatW(name) as u16 }
}

fn data_format(format: u16) -> FORMATETC {
    FORMATETC {
        cfFormat: format,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

fn read_text_format(data: &IDataObject, format: u16, encoding: TextEncoding) -> Option<String> {
    let config = data_format(format);
    if unsafe { data.QueryGetData(&config) }.is_err() {
        return None;
    }
    let mut medium = unsafe { data.GetData(&config) }.ok()?;
    let global = unsafe { medium.u.hGlobal };
    let text = read_global_text(global, encoding);
    unsafe { ReleaseStgMedium(&mut medium) };
    text
}

fn read_global_text(global: HGLOBAL, encoding: TextEncoding) -> Option<String> {
    if global.is_invalid() {
        return None;
    }
    let size = unsafe { GlobalSize(global) };
    if size == 0 {
        return None;
    }
    let pointer = unsafe { GlobalLock(global) };
    if pointer.is_null() {
        return None;
    }
    let text = match encoding {
        TextEncoding::Utf16 => {
            let words = unsafe {
                std::slice::from_raw_parts(pointer.cast::<u16>(), size / std::mem::size_of::<u16>())
            };
            let length = words
                .iter()
                .position(|word| *word == 0)
                .unwrap_or(words.len());
            String::from_utf16_lossy(&words[..length])
        }
        TextEncoding::Bytes => {
            let bytes = unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), size) };
            let length = bytes
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(bytes.len());
            String::from_utf8_lossy(&bytes[..length]).into_owned()
        }
    };
    unsafe {
        let _ = GlobalUnlock(global);
    }
    Some(text)
}

fn read_file_paths(data: &IDataObject) -> Option<Vec<PathBuf>> {
    let config = data_format(CF_HDROP.0);
    if unsafe { data.QueryGetData(&config) }.is_err() {
        return None;
    }
    let mut medium: STGMEDIUM = unsafe { data.GetData(&config) }.ok()?;
    let global = unsafe { medium.u.hGlobal };
    if global.is_invalid() {
        unsafe { ReleaseStgMedium(&mut medium) };
        return None;
    }
    let drop = HDROP(global.0);
    let count = unsafe { DragQueryFileW(drop, DRAGDROP_GET_FILES_COUNT, None) };
    let mut paths = Vec::with_capacity(count as usize);
    for index in 0..count {
        let length = unsafe { DragQueryFileW(drop, index, None) } as usize;
        let mut buffer = vec![0u16; length + 1];
        if unsafe { DragQueryFileW(drop, index, Some(&mut buffer)) } > 0 {
            paths.push(PathBuf::from(String::from_utf16_lossy(&buffer[..length])));
        }
    }
    unsafe { ReleaseStgMedium(&mut medium) };
    (!paths.is_empty()).then_some(paths)
}

fn destination_at_point(hwnd: HWND, point: &POINTL) -> Option<DropDestination> {
    let mut point = POINT {
        x: point.x,
        y: point.y,
    };
    if !unsafe { ScreenToClient(hwnd, &mut point) }.as_bool() {
        return None;
    }
    let mut bounds = RECT::default();
    unsafe { GetClientRect(hwnd, &mut bounds) }.ok()?;
    let (category_top, category_height) = category_row_metrics(hwnd);

    destination_for_client_point(
        point.x as f32 - bounds.left as f32,
        point.y as f32 - bounds.top as f32,
        bounds.right as f32 - bounds.left as f32,
        bounds.bottom as f32 - bounds.top as f32,
        category_top as f32,
        category_height as f32,
    )
}

fn category_row_metrics(hwnd: HWND) -> (i32, i32) {
    let scale = unsafe { GetDpiForWindow(hwnd) } as f32 / 96.;
    let top = (crate::ui::WINDOWS_TITLEBAR_ROW_HEIGHT_PX * scale).round() as i32;
    let height = (crate::ui::TITLEBAR_ROW_HEIGHT_PX * scale).round() as i32;
    (top, height)
}

fn destination_for_client_point(
    client_x: f32,
    client_y: f32,
    client_width: f32,
    client_height: f32,
    category_top: f32,
    category_height: f32,
) -> Option<DropDestination> {
    if client_x < 0.
        || client_x >= client_width
        || client_width <= 0.
        || client_y < 0.
        || client_y >= client_height
    {
        return None;
    }
    if client_y >= category_top && client_y < category_top + category_height {
        let index = ((client_x / client_width) * Category::ALL.len() as f32).floor() as usize;
        return Category::ALL
            .get(index)
            .copied()
            .map(DropDestination::Category);
    }
    (client_y >= category_top + category_height).then_some(DropDestination::ActiveCategory)
}

fn colorref(color: Hsla) -> COLORREF {
    let color = color.to_rgb();
    let red = (color.r * 255.).round() as u32;
    let green = (color.g * 255.).round() as u32;
    let blue = (color.b * 255.).round() as u32;
    COLORREF(red | green << 8 | blue << 16)
}

fn register_hover_overlay_class(owner: HWND) -> Option<HINSTANCE> {
    static INSTANCE: OnceLock<Option<isize>> = OnceLock::new();
    let instance = (*INSTANCE.get_or_init(|| {
        let instance = unsafe { GetWindowLongPtrW(owner, GWLP_HINSTANCE) };
        if instance == 0 {
            return None;
        }
        let class = WNDCLASSW {
            lpfnWndProc: Some(hover_overlay_window_proc),
            hInstance: HINSTANCE(instance as *mut c_void),
            lpszClassName: w!("Lowcat-Native-Drop-Hover"),
            ..Default::default()
        };
        (unsafe { RegisterClassW(&class) } != 0).then_some(instance)
    }))?;
    Some(HINSTANCE(instance as *mut c_void))
}

unsafe extern "system" fn hover_overlay_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let dc = unsafe { BeginPaint(hwnd, &mut paint) };
            let mut bounds = RECT::default();
            if unsafe { GetClientRect(hwnd, &mut bounds) }.is_ok() {
                let color = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as u32;
                let brush = unsafe { CreateSolidBrush(COLORREF(color)) };
                unsafe {
                    FillRect(dc, &bounds, brush);
                    let _ = DeleteObject(brush.into());
                }
            }
            let _ = unsafe { EndPaint(hwnd, &paint) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn debug_url_drop(details: impl FnOnce() -> String) {
    crate::diagnostics::debug("url-drop", details);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_tabs_choose_an_explicit_destination() {
        let destination_at = |x, y| destination_for_client_point(x, y, 800., 600., 30., 38.);

        assert_eq!(
            destination_at(40., 30.),
            Some(DropDestination::Category(Category::Music))
        );
        assert_eq!(
            destination_at(399., 67.),
            Some(DropDestination::Category(Category::Music))
        );
        assert_eq!(
            destination_at(400., 30.),
            Some(DropDestination::Category(Category::Sfx))
        );
        assert_eq!(
            destination_at(700., 67.),
            Some(DropDestination::Category(Category::Sfx))
        );
        assert_eq!(destination_at(800., 40.), None);
        assert_eq!(destination_at(400., 29.), None);
    }

    #[test]
    fn category_tabs_use_the_full_row_width() {
        assert_eq!(
            destination_for_client_point(0., 50., 1200., 600., 30., 38.),
            Some(DropDestination::Category(Category::Music))
        );
        assert_eq!(
            destination_for_client_point(1199., 50., 1200., 600., 30., 38.),
            Some(DropDestination::Category(Category::Sfx))
        );
    }

    #[test]
    fn content_area_uses_the_active_category() {
        assert_eq!(
            destination_for_client_point(400., 68., 800., 600., 30., 38.),
            Some(DropDestination::ActiveCategory)
        );
        assert_eq!(
            destination_for_client_point(400., 599., 800., 600., 30., 38.),
            Some(DropDestination::ActiveCategory)
        );
        assert_eq!(
            destination_for_client_point(400., 600., 800., 600., 30., 38.),
            None
        );
    }
}
