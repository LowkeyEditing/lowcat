#[cfg(target_os = "windows")]
use std::ffi::c_void;

#[cfg(target_os = "windows")]
use gpui::{App, MouseDownEvent, WindowControlArea};
use gpui::{
    AsyncApp, ClickEvent, Context, DragMoveEvent, Entity, ExternalPaths, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement, PathPromptOptions, Pixels, Render, SharedString,
    StatefulInteractiveElement as _, Styled, Window, div, prelude::FluentBuilder as _, px,
};
#[cfg(target_os = "windows")]
use gpui_component::Icon;
use gpui_component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _, StyledExt,
    button::{Button, ButtonVariants as _},
};
#[cfg(target_os = "windows")]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
#[cfg(target_os = "windows")]
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::{
        Input::KeyboardAndMouse::ReleaseCapture,
        WindowsAndMessaging::{HTCAPTION, PostMessageW, WM_NCLBUTTONDOWN},
    },
};

use crate::library::Library;
use crate::model::Category;

use super::table::InternalFileDrag;
#[cfg(target_os = "windows")]
use super::windows_menu_bar::WindowsMenuBar;

/// Left inset of the category row. macOS shares that row with its traffic
/// lights, while Windows places categories on their own full-width row.
#[cfg(target_os = "macos")]
pub(crate) const TITLEBAR_LEFT_OFFSET_PX: f32 = 84.;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) const TITLEBAR_LEFT_OFFSET_PX: f32 = 0.;
#[cfg(not(target_os = "windows"))]
pub(crate) const TITLEBAR_LEFT_OFFSET: Pixels = px(TITLEBAR_LEFT_OFFSET_PX);

pub(crate) const TITLEBAR_ROW_HEIGHT_PX: f32 = 38.;
pub(crate) const CATEGORY_DRAG_HOVER_OPACITY: f32 = 0.55;
const TITLEBAR_ROW_HEIGHT: Pixels = px(TITLEBAR_ROW_HEIGHT_PX);
#[cfg(target_os = "windows")]
pub(crate) const WINDOWS_TITLEBAR_ROW_HEIGHT_PX: f32 = 30.;
#[cfg(target_os = "windows")]
const WINDOWS_TITLEBAR_ROW_HEIGHT: Pixels = px(WINDOWS_TITLEBAR_ROW_HEIGHT_PX);
#[cfg(target_os = "windows")]
const WINDOWS_CONTROLS_WIDTH: Pixels = px(90.);
#[cfg(target_os = "windows")]
pub(crate) const TITLEBAR_HEIGHT: Pixels = px(68.);
#[cfg(not(target_os = "windows"))]
pub(crate) const TITLEBAR_HEIGHT: Pixels = TITLEBAR_ROW_HEIGHT;

#[cfg(target_os = "windows")]
fn windows_caption_button(
    id: &'static str,
    icon: IconName,
    control_area: WindowControlArea,
    is_close: bool,
    cx: &App,
) -> impl IntoElement + use<> {
    let (hover_bg, hover_fg, active_bg, active_fg) = if is_close {
        (
            cx.theme().danger,
            cx.theme().danger_foreground,
            cx.theme().danger_active,
            cx.theme().danger_foreground,
        )
    } else {
        (
            cx.theme().secondary_hover,
            cx.theme().secondary_foreground,
            cx.theme().secondary_active,
            cx.theme().secondary_foreground,
        )
    };

    div()
        .id(id)
        .h_flex()
        .h_full()
        .w(WINDOWS_TITLEBAR_ROW_HEIGHT)
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .occlude()
        .text_color(cx.theme().foreground)
        .hover(move |style| style.bg(hover_bg).text_color(hover_fg))
        .active(move |style| style.bg(active_bg).text_color(active_fg))
        .window_control_area(control_area)
        .child(Icon::new(icon).small())
}

#[cfg(target_os = "windows")]
fn windows_window_controls(window: &Window, cx: &App) -> impl IntoElement + use<> {
    div()
        .id("windows-window-controls")
        .h_flex()
        .h_full()
        .flex_shrink_0()
        .child(windows_caption_button(
            "window-minimize",
            IconName::WindowMinimize,
            WindowControlArea::Min,
            false,
            cx,
        ))
        .child(windows_caption_button(
            "window-maximize",
            if window.is_maximized() {
                IconName::WindowRestore
            } else {
                IconName::WindowMaximize
            },
            WindowControlArea::Max,
            false,
            cx,
        ))
        .child(windows_caption_button(
            "window-close",
            IconName::WindowClose,
            WindowControlArea::Close,
            true,
            cx,
        ))
}

#[cfg(target_os = "windows")]
fn start_titlebar_window_move(window: &Window) {
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let hwnd = HWND(handle.hwnd.get() as *mut c_void);

    unsafe {
        let _ = ReleaseCapture();
        let _ = PostMessageW(
            Some(hwnd),
            WM_NCLBUTTONDOWN,
            WPARAM(HTCAPTION as usize),
            LPARAM(0),
        );
    }
}

#[cfg(not(target_os = "windows"))]
fn start_titlebar_window_move(window: &Window) {
    window.start_window_move();
}

pub struct AppTitleBar {
    library: Entity<Library>,
    #[cfg(target_os = "windows")]
    app_menu_bar: Entity<WindowsMenuBar>,
    hovered_category: Option<Category>,
    drag_hovered_category: Option<Category>,
    folder_prompt_active: bool,
    should_move_window: bool,
}

impl AppTitleBar {
    pub fn new(library: Entity<Library>, cx: &mut Context<Self>) -> Self {
        cx.observe(&library, |_, _, cx| cx.notify()).detach();

        Self {
            library,
            #[cfg(target_os = "windows")]
            app_menu_bar: WindowsMenuBar::new(cx),
            hovered_category: None,
            drag_hovered_category: None,
            folder_prompt_active: false,
            should_move_window: false,
        }
    }

    fn update_category_drag_hover(
        &mut self,
        category: Category,
        has_paths: bool,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let is_current = self.drag_hovered_category == Some(category);
        if !has_paths || !hovered {
            if is_current {
                self.drag_hovered_category = None;
                cx.notify();
            }
            return;
        }

        if !is_current {
            self.drag_hovered_category = Some(category);
            cx.notify();
        }
    }

    fn drop_paths(
        &mut self,
        category: Category,
        paths: Vec<std::path::PathBuf>,
        cx: &mut Context<Self>,
    ) {
        self.drag_hovered_category = None;
        debug_titlebar_interaction(|| {
            format!("drop category={} paths={}", category.label(), paths.len())
        });
        self.library
            .update(cx, |lib, cx| lib.import_files(category, paths, cx));
    }

    fn choose_category_folder(&mut self, category: Category, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if self.folder_prompt_active {
            return;
        }

        self.folder_prompt_active = true;
        cx.notify();

        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(format!("Select {} folder", category.label()).into()),
        });
        let library = self.library.downgrade();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let path = paths
                .await
                .ok()
                .and_then(|paths| paths.ok())
                .flatten()
                .and_then(|paths| paths.into_iter().next());

            this.update(cx, |this, cx| {
                this.folder_prompt_active = false;
                cx.notify();
            })
            .ok()?;

            let Some(path) = path else {
                return Some(());
            };

            library
                .update(cx, |lib, cx| {
                    let _ = lib.set_category_folder(category, path, cx);
                })
                .ok()?;
            Some(())
        })
        .detach();
    }
}

impl Render for AppTitleBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let render_start = crate::perf::start();
        let active = self.library.read(cx).active();
        let internal_drag_active = self.library.read(cx).internal_file_drag_active();
        if !internal_drag_active
            && !cx.has_active_drag()
            && self.drag_hovered_category.take().is_some()
        {
            debug_titlebar_interaction(|| "clear drag hover: drag inactive".to_string());
        }
        let outline = cx.theme().title_bar_border;
        let selected_bg = outline.opacity(0.16);

        let mut categories = div()
            .h_flex()
            .h(px(37.))
            .w_full()
            .flex_1()
            .min_w_0()
            .mt(px(1.))
            .items_center()
            .overflow_hidden()
            .border_color(outline);
        for category in Category::ALL {
            let selected = category == active;
            let hovered = self.hovered_category == Some(category);
            let missing_folder = self.library.read(cx).category_needs_folder(category);
            let drag_hovered = self.drag_hovered_category == Some(category);
            let bg = if selected {
                selected_bg
            } else {
                cx.theme().background
            };
            let hover_bg = if selected {
                selected_bg
            } else {
                cx.theme().secondary
            };
            let fg = if selected {
                cx.theme().foreground
            } else {
                cx.theme().muted_foreground
            };
            let border = if selected {
                outline
            } else {
                cx.theme().transparent
            };
            let can_hover = !internal_drag_active;
            let drag_bg = cx.theme().secondary.opacity(CATEGORY_DRAG_HOVER_OPACITY);
            let folder_button = Button::new(SharedString::from(format!(
                "category-folder:{}",
                category.label()
            )))
            .icon(IconName::Folder)
            .small()
            .compact()
            .ghost()
            .disabled(self.folder_prompt_active)
            .tooltip(if self.folder_prompt_active {
                SharedString::from("Folder picker is already open")
            } else {
                SharedString::from(format!("Choose {} folder", category.label()))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.choose_category_folder(category, cx);
            }));

            let show_folder_button = can_hover && (hovered || missing_folder);

            categories = categories.child(
                div()
                    .id(SharedString::from(category.label()))
                    .relative()
                    .h_flex()
                    .h_full()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .justify_center()
                    .bg(bg)
                    .border_l_1()
                    .border_r_1()
                    .border_color(border)
                    .text_sm()
                    .text_color(fg)
                    .cursor_pointer()
                    .child(SharedString::from(category.label()))
                    .when(drag_hovered, |this| this.bg(drag_bg))
                    .when(can_hover, |this| this.hover(move |this| this.bg(hover_bg)))
                    .when(show_folder_button, |this| {
                        this.child(div().absolute().right(px(6.)).child(folder_button))
                    })
                    .on_drag_move::<ExternalPaths>(cx.listener(
                        move |this, event: &DragMoveEvent<ExternalPaths>, _, cx| {
                            let library = this.library.read(cx);
                            let has_paths = if library.internal_file_drag_active() {
                                library.internal_file_drag_paths().is_some()
                            } else {
                                !event.drag(cx).paths().is_empty()
                            };
                            this.update_category_drag_hover(
                                category,
                                has_paths,
                                event.bounds.contains(&event.event.position),
                                cx,
                            );
                        },
                    ))
                    .on_drag_move::<InternalFileDrag>(cx.listener(
                        move |this, event: &DragMoveEvent<InternalFileDrag>, _, cx| {
                            let has_paths = this.library.read(cx).internal_file_drag_active()
                                && !event.drag(cx).is_empty();
                            this.update_category_drag_hover(
                                category,
                                has_paths,
                                event.bounds.contains(&event.event.position),
                                cx,
                            );
                        },
                    ))
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if internal_drag_active {
                            if this.hovered_category.is_some() {
                                this.hovered_category = None;
                                cx.notify();
                            }
                            return;
                        }

                        if *hovered {
                            this.hovered_category = Some(category);
                        } else if this.hovered_category == Some(category) {
                            this.hovered_category = None;
                        }
                        cx.notify();
                    }))
                    .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                        if event.click_count() == 1 {
                            this.library
                                .update(cx, |lib, cx| lib.set_category(category, cx));
                        }
                    }))
                    .on_drop(cx.listener(move |this, paths: &ExternalPaths, _, cx| {
                        let internal_paths = this
                            .library
                            .read(cx)
                            .internal_file_drag_paths()
                            .map(ToOwned::to_owned);
                        this.drop_paths(
                            category,
                            internal_paths.unwrap_or_else(|| paths.paths().to_vec()),
                            cx,
                        );
                    }))
                    .on_drop(cx.listener(move |this, drag: &InternalFileDrag, _, cx| {
                        this.drop_paths(category, drag.paths(), cx);
                    })),
            );
        }

        #[cfg(not(target_os = "windows"))]
        let titlebar = div()
            .h_flex()
            .flex_shrink_0()
            .h(TITLEBAR_ROW_HEIGHT)
            .pl(TITLEBAR_LEFT_OFFSET)
            .bg(cx.theme().background)
            .border_b_1()
            .border_color(cx.theme().title_bar_border)
            .on_mouse_down_out(cx.listener(|this, _, _, _| {
                this.should_move_window = false;
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.should_move_window = true;
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.should_move_window = false;
                }),
            )
            .on_mouse_move(cx.listener(|this, _, window, _| {
                if this.should_move_window {
                    this.should_move_window = false;
                    start_titlebar_window_move(window);
                }
            }))
            .child(categories);

        #[cfg(target_os = "windows")]
        let titlebar = {
            let window_titlebar = div()
                .h_flex()
                .flex_shrink_0()
                .h(WINDOWS_TITLEBAR_ROW_HEIGHT)
                .bg(cx.theme().background)
                .border_b_1()
                .border_color(cx.theme().title_bar_border)
                .on_mouse_down_out(cx.listener(|this, _, _, _| {
                    this.should_move_window = false;
                }))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, event: &MouseDownEvent, window, _| {
                        this.should_move_window = event.position.x
                            < window.viewport_size().width - WINDOWS_CONTROLS_WIDTH;
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, _| {
                        this.should_move_window = false;
                    }),
                )
                .on_mouse_move(cx.listener(|this, _, window, _| {
                    if this.should_move_window {
                        this.should_move_window = false;
                        start_titlebar_window_move(window);
                    }
                }))
                .child(
                    div()
                        .h_full()
                        .flex_1()
                        .min_w_0()
                        .pl(px(8.))
                        .child(self.app_menu_bar.clone()),
                )
                .child(windows_window_controls(window, cx));

            div().v_flex().flex_shrink_0().child(window_titlebar).child(
                div()
                    .h_flex()
                    .flex_shrink_0()
                    .h(TITLEBAR_ROW_HEIGHT)
                    .bg(cx.theme().background)
                    .border_b_1()
                    .border_color(cx.theme().title_bar_border)
                    .child(categories),
            )
        };

        crate::perf::finish("titlebar.render", render_start, || {
            format!(
                "active={} internal_drag={internal_drag_active}",
                active.label()
            )
        });
        titlebar
    }
}

fn debug_titlebar_interaction(details: impl FnOnce() -> String) {
    crate::diagnostics::debug("titlebar", details);
}
