use std::{cell::Cell, rc::Rc};

use gpui::{
    Context, Entity, InteractiveElement as _, IntoElement, Keystroke, MouseButton, ParentElement,
    Render, SharedString, Styled, Window, div, prelude::FluentBuilder as _,
};
use gpui_component::{
    ActiveTheme as _, Selectable as _, Sizable as _, StyledExt,
    button::{Button, ButtonVariants as _},
    kbd::Kbd,
    menu::{ContextMenuExt as _, PopupMenuItem},
};

use crate::library::{
    Library, tag_matches_search, tag_search_group_sort_key, tag_search_match_sort_key,
};
use crate::model::split_subtag;
use crate::ui::{CONTENT_PX, ROW_PANEL_HEIGHT};

pub struct FilterPanel {
    library: Entity<Library>,
    snapshot: Option<Arc<FilterPanelSnapshot>>,
}

struct FilterPanelSnapshot {
    revision: u64,
    keys: Vec<String>,
    single_match: Option<(String, String)>,
    rows: Vec<FilterRowSnapshot>,
    schema_key_count: usize,
    schema_value_count: usize,
}

struct FilterRowSnapshot {
    key: String,
    label: String,
    indented: bool,
    checked: BTreeSet<String>,
    values: Vec<String>,
}

impl FilterPanel {
    pub fn new(library: Entity<Library>, cx: &mut Context<Self>) -> Self {
        cx.observe(&library, |_, _, cx| cx.notify()).detach();
        Self {
            library,
            snapshot: None,
        }
    }

    fn build_snapshot(library: &Library) -> FilterPanelSnapshot {
        let revision = library.filter_panel_revision();
        let state = library.active_state();
        let schema = library.tag_panel_schema();
        let selected = state.selected.clone();
        let tag_search = library.search().to_string();
        let single_match = library.single_tag_search_match_in(&schema);
        let keys = schema.keys().cloned().collect();
        let include_hidden_groups = !tag_search.is_empty();

        let mut matching_groups = Vec::new();
        for (key, values) in &schema {
            if !include_hidden_groups && !library.tag_group_is_visible(key) {
                continue;
            }

            let checked = selected.get(key).cloned().unwrap_or_default();
            let root_values: BTreeSet<String> = values
                .iter()
                .map(|value| {
                    split_subtag(value)
                        .map(|(parent, _)| parent.to_string())
                        .unwrap_or_else(|| value.clone())
                })
                .collect();
            let children_by_parent: BTreeMap<&str, Vec<&str>> = values
                .iter()
                .filter_map(|value| split_subtag(value).map(|(parent, _)| (parent, value.as_str())))
                .fold(BTreeMap::new(), |mut by_parent, (parent, value)| {
                    by_parent.entry(parent).or_default().push(value);
                    by_parent
                });
            let mut matching_values: Vec<String> = root_values
                .into_iter()
                .filter(|root| {
                    tag_matches_search(root, &tag_search)
                        || children_by_parent
                            .get(root.as_str())
                            .is_some_and(|children| {
                                children
                                    .iter()
                                    .any(|value| tag_matches_search(value, &tag_search))
                            })
                })
                .filter(|value| library.tag_is_visible_in_panel(key, value))
                .collect();
            if matching_values.is_empty() {
                continue;
            }
            matching_values.sort_by_key(|value| tag_search_match_sort_key(value, &tag_search));
            let group_sort_key = tag_search_group_sort_key(
                key,
                matching_values.iter().map(String::as_str),
                &tag_search,
            );
            matching_groups.push((group_sort_key, key, checked, matching_values, values));
        }
        matching_groups.sort_by_key(|(sort_key, _, _, _, _)| sort_key.clone());

        let mut rows = Vec::new();
        for (_, key, checked, matching_values, raw_values) in matching_groups {
            rows.push(FilterRowSnapshot {
                key: key.clone(),
                label: key.clone(),
                indented: false,
                checked: checked.clone(),
                values: matching_values,
            });
            let mut expanded_parents: BTreeSet<String> = checked
                .iter()
                .filter(|value| !value.contains('/'))
                .cloned()
                .collect();
            if !tag_search.is_empty() {
                expanded_parents.extend(raw_values.iter().filter_map(|value| {
                    let (parent, child) = split_subtag(value)?;
                    (tag_matches_search(child, &tag_search)
                        || (tag_search.contains('/') && tag_matches_search(value, &tag_search)))
                    .then(|| parent.to_string())
                }));
            }
            for parent in expanded_parents {
                let mut children: Vec<(String, String)> = raw_values
                    .iter()
                    .filter_map(|value| {
                        let (candidate_parent, child) = split_subtag(value)?;
                        (candidate_parent == parent
                            && (tag_search.is_empty()
                                || tag_matches_search(child, &tag_search)
                                || (tag_search.contains('/')
                                    && tag_matches_search(value, &tag_search)))
                            && library.tag_is_visible_in_panel(key, value))
                        .then(|| (child.to_string(), value.clone()))
                    })
                    .collect();
                children.sort_by_key(|(child, _)| tag_search_match_sort_key(child, &tag_search));
                if !children.is_empty() {
                    rows.push(FilterRowSnapshot {
                        key: key.clone(),
                        label: parent,
                        indented: true,
                        checked: checked.clone(),
                        values: children.into_iter().map(|(_, value)| value).collect(),
                    });
                }
            }
        }

        FilterPanelSnapshot {
            revision,
            keys,
            single_match,
            rows,
            schema_key_count: schema.len(),
            schema_value_count: schema.values().map(Vec::len).sum(),
        }
    }
}

impl Render for FilterPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let render_start = crate::perf::start();
        let revision = self.library.read(cx).filter_panel_revision();
        if self
            .snapshot
            .as_ref()
            .is_none_or(|snapshot| snapshot.revision != revision)
        {
            let library = self.library.read(cx);
            self.snapshot = Some(Arc::new(Self::build_snapshot(library)));
        }
        let snapshot = self
            .snapshot
            .as_ref()
            .expect("filter snapshot was initialized")
            .clone();
        let menu_library = self.library.clone();
        let menu_keys = snapshot.keys.clone();
        let menu_visibility: Vec<_> = menu_keys
            .iter()
            .map(|key| {
                let visible = self.library.read(cx).tag_group_is_visible(key);
                (key.clone(), visible)
            })
            .collect();
        let all_groups_visible = menu_visibility.iter().all(|(_, visible)| *visible);
        let child_context_menu_claimed = Rc::new(Cell::new(false));
        let panel_context_menu_claimed = child_context_menu_claimed.clone();
        let mut panel = div()
            .v_flex()
            .w_full()
            .min_h(ROW_PANEL_HEIGHT)
            .px(CONTENT_PX)
            .py_1()
            .gap_2()
            .context_menu(move |mut menu, _, _| {
                if panel_context_menu_claimed.replace(false) {
                    return menu;
                }
                for (key, visible) in &menu_visibility {
                    let library = menu_library.clone();
                    menu = menu.item(PopupMenuItem::new(key.clone()).checked(*visible).on_click({
                        let key = key.clone();
                        move |_, _, cx| {
                            library.update(cx, |lib, cx| {
                                lib.toggle_tag_group_visibility(&key, cx);
                            });
                        }
                    }));
                }
                if !menu_keys.is_empty() {
                    let library = menu_library.clone();
                    let keys = menu_keys.clone();
                    menu = menu.separator().item(
                        PopupMenuItem::new(if all_groups_visible {
                            "Hide All"
                        } else {
                            "Show All"
                        })
                        .on_click(move |_, _, cx| {
                            library.update(cx, |lib, cx| {
                                if all_groups_visible {
                                    lib.hide_all_tag_groups(&keys, cx);
                                } else {
                                    lib.show_all_tag_groups(cx);
                                }
                            });
                        }),
                    );
                }
                menu
            });

        for row in &snapshot.rows {
            let key = &row.key;
            let row_label = &row.label;
            let checked = &row.checked;
            let mut group = div()
                .h_flex()
                .flex_wrap()
                .w_full()
                .items_center()
                .gap_1()
                .when(row.indented, |group| group.pl_4())
                .child(
                    div()
                        .id(SharedString::from(format!("filter-key:{key}:{row_label}")))
                        .h_flex()
                        .flex_shrink_0()
                        .items_center()
                        .gap_1()
                        .mr_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(SharedString::from(format!("{row_label}:"))),
                        ),
                );

            for value in &row.values {
                let is_active = checked.contains(value);
                let is_single_match =
                    snapshot
                        .single_match
                        .as_ref()
                        .is_some_and(|(match_key, match_value)| {
                            match_key == key && match_value == value
                        });
                let key_owned = key.clone();
                let value_owned = value.clone();
                let intersection_library = self.library.clone();
                let intersection_key = key.clone();
                let intersection_value = value.clone();
                let chip_context_menu_claimed = child_context_menu_claimed.clone();
                let intersection_checked =
                    self.library.read(cx).tag_shows_on_intersection(key, value);
                let display_value = split_subtag(value)
                    .map(|(_, child)| child.to_string())
                    .unwrap_or_else(|| value.clone());
                let chip_border = if is_single_match {
                    cx.theme().success
                } else if is_active {
                    cx.theme().primary
                } else {
                    cx.theme().border
                };

                group = group.child(
                    Button::new(format!("filter-{key}:{value}"))
                        .xsmall()
                        .compact()
                        .border_1()
                        .border_color(chip_border)
                        .label(display_value)
                        .selected(is_active)
                        .when(is_active, |button| button.primary())
                        .when(is_single_match, |button| {
                            button.child(Kbd::new(
                                Keystroke::parse("enter").expect("valid keystroke"),
                            ))
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let key = key_owned.clone();
                            let value = value_owned.clone();
                            this.library.update(cx, |lib, cx| {
                                lib.toggle_value(&key, &value, cx);
                            });
                        }))
                        .on_mouse_down(MouseButton::Right, move |_, _, _| {
                            chip_context_menu_claimed.set(true);
                        })
                        .context_menu(move |menu, _, _| {
                            let library = intersection_library.clone();
                            let key = intersection_key.clone();
                            let value = intersection_value.clone();
                            menu.item(
                                PopupMenuItem::new("Show on intersection")
                                    .checked(intersection_checked)
                                    .on_click(move |_, _, cx| {
                                        library.update(cx, |lib, cx| {
                                            lib.toggle_tag_intersection_visibility(
                                                &key, &value, cx,
                                            );
                                        });
                                    }),
                            )
                        }),
                );
            }

            panel = panel.child(group);
        }

        crate::perf::finish("filter_panel.render", render_start, || {
            format!(
                "keys={} values={}",
                snapshot.schema_key_count, snapshot.schema_value_count
            )
        });
        panel
    }
}
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
