use gpui::{
    App, AppContext as _, Context, DismissEvent, Entity, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement, MouseButton, OwnedMenu, OwnedMenuItem,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window, anchored, deferred, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    GlobalState, Selectable as _, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    menu::PopupMenu,
};

fn add_menu_items(
    mut popup: PopupMenu,
    items: Vec<OwnedMenuItem>,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    for item in items {
        popup = match item {
            OwnedMenuItem::Action {
                name,
                action,
                checked,
                disabled,
                ..
            } => popup.menu_with_check_and_disabled(name, checked, action.boxed_clone(), disabled),
            OwnedMenuItem::Separator => popup.separator(),
            OwnedMenuItem::Submenu(submenu) => {
                popup.submenu(submenu.name, window, cx, move |popup, window, cx| {
                    add_menu_items(popup, submenu.items.clone(), window, cx)
                })
            }
            OwnedMenuItem::SystemMenu(_) => popup,
        };
    }
    popup
}

/// A compact Windows app menu bar backed by the same application menus as the
/// native-sized gpui-component menu bar.
pub(crate) struct WindowsMenuBar {
    menus: Vec<Entity<WindowsMenu>>,
    selected_index: Option<usize>,
    action_context: Option<FocusHandle>,
}

impl WindowsMenuBar {
    pub(crate) fn new(cx: &mut App) -> Entity<Self> {
        cx.new(|cx| {
            let menus = GlobalState::global(cx).app_menus().to_vec();
            let menu_bar = cx.entity();
            Self {
                menus: menus
                    .iter()
                    .enumerate()
                    .map(|(ix, menu)| WindowsMenu::new(ix, menu, menu_bar.clone(), cx))
                    .collect(),
                selected_index: None,
                action_context: None,
            }
        })
    }

    fn select(
        &mut self,
        selected_index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_index.is_none() && selected_index.is_some() {
            self.action_context = window.focused(cx);
        } else if selected_index.is_none() {
            if let Some(focus) = self.action_context.as_ref() {
                focus.focus(window, cx);
            }
            self.action_context = None;
        }
        self.selected_index = selected_index;
        cx.notify();
    }
}

impl Render for WindowsMenuBar {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("windows-app-menu-bar")
            .h_flex()
            .size_full()
            .gap_x_1()
            .overflow_x_scroll()
            .children(self.menus.clone())
    }
}

struct WindowsMenu {
    menu_bar: Entity<WindowsMenuBar>,
    ix: usize,
    name: SharedString,
    menu: OwnedMenu,
    popup_menu: Option<Entity<PopupMenu>>,
    subscription: Option<Subscription>,
}

impl WindowsMenu {
    fn new(
        ix: usize,
        menu: &OwnedMenu,
        menu_bar: Entity<WindowsMenuBar>,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|_| Self {
            menu_bar,
            ix,
            name: menu.name.clone(),
            menu: menu.clone(),
            popup_menu: None,
            subscription: None,
        })
    }

    fn popup(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<PopupMenu> {
        let action_context = self.menu_bar.read(cx).action_context.clone();
        let popup = self.popup_menu.clone().unwrap_or_else(|| {
            let items = self.menu.items.clone();
            let popup = PopupMenu::build(window, cx, |menu, window, cx| {
                let menu = if let Some(focus) = action_context.clone() {
                    menu.action_context(focus)
                } else {
                    menu
                };
                add_menu_items(menu, items, window, cx)
            });
            self.subscription = Some(cx.subscribe_in(&popup, window, Self::dismissed));
            self.popup_menu = Some(popup.clone());
            popup
        });
        popup.read(cx).focus_handle(cx).focus(window, cx);
        popup
    }

    fn dismissed(
        &mut self,
        _: &Entity<PopupMenu>,
        _: &DismissEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.subscription.take();
        self.popup_menu.take();
        self.menu_bar
            .update(cx, |menu_bar, cx| menu_bar.select(None, window, cx));
    }
}

impl Render for WindowsMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.menu_bar.read(cx).selected_index == Some(self.ix);
        let ix = self.ix;
        let menu_bar = self.menu_bar.clone();

        div()
            .id(ix)
            .relative()
            .child(
                Button::new("menu")
                    .xsmall()
                    .compact()
                    .ghost()
                    .label(self.name.clone())
                    .selected(selected)
                    .on_mouse_down(MouseButton::Left, |_, window, cx| {
                        window.prevent_default();
                        cx.stop_propagation();
                    })
                    .on_click(move |_, window, cx| {
                        menu_bar.update(cx, |menu_bar, cx| {
                            menu_bar.select(
                                (menu_bar.selected_index != Some(ix)).then_some(ix),
                                window,
                                cx,
                            );
                        });
                    }),
            )
            .on_hover({
                let menu_bar = self.menu_bar.clone();
                move |hovered, window, cx| {
                    if *hovered && menu_bar.read(cx).selected_index.is_some() {
                        menu_bar.update(cx, |menu_bar, cx| menu_bar.select(Some(ix), window, cx));
                    }
                }
            })
            .when(selected, |this| {
                this.child(deferred(
                    anchored()
                        .anchor(gpui::Anchor::TopLeft)
                        .snap_to_window_with_margin(px(8.))
                        .child(
                            div()
                                .size_full()
                                .occlude()
                                .top_1()
                                .child(self.popup(window, cx)),
                        ),
                ))
            })
    }
}
