use std::ops::Range;

use egui::{
    emath::TSTransform,
    epaint::{PathShape, PathStroke, TextShape},
    lerp, pos2, vec2, Align, Align2, Button, Color32, CornerRadius, CursorIcon, Frame, Id, Key,
    LayerId, Layout, NumExt, Order, Popup, PopupCloseBehavior, Pos2, Rect, Response, ScrollArea,
    Sense, Shape, Stroke, StrokeKind, TextStyle, Ui, UiBuilder, Vec2, WidgetText,
};

use crate::dock_area::tab_removal::{ForcedRemoval, TabRemoval};
use crate::node::LeafNode;
use crate::tab_viewer::OnCloseResponse;
use crate::NodePath;
use crate::{
    dock_area::{
        drag_and_drop::{DragData, DragDropState, HoverData, TreeComponent},
        state::State,
    },
    utils::{fade_visuals, rect_set_size_centered, rect_stroke_box},
    DockArea, Node, NodeIndex, Style, SurfaceIndex, TabAddAlign, TabIndex, TabStyle, TabViewer,
};

impl<Tab> DockArea<'_, Tab> {
    pub(super) fn show_leaf(
        &mut self,
        ui: &mut Ui,
        state: &mut State,
        path: NodePath,
        tab_viewer: &mut impl TabViewer<Tab = Tab>,
        fade_style: Option<(&Style, f32)>,
    ) {
        assert!(self.dock_state[path].is_leaf());
        let collapsed = self.dock_state[path].is_collapsed();

        let rect = self.dock_state[path]
            .rect()
            .expect("This node must be a leaf");

        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }

        let ui = &mut ui.new_child(
            UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::top_down_justified(Align::Min))
                .id_salt((path.node, "node")),
        );
        let spacing = ui.spacing().item_spacing;
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        ui.set_clip_rect(rect);

        if self.dock_state[path].tabs_count() == 0 {
            return;
        }

        let hide_tab_bar = self.dock_state[path].tabs_count() == 1 && {
            let tabs = self.dock_state[path].tabs().unwrap();
            tab_viewer.solo_tab_no_bar(&tabs[0])
        };

        // Reset per-leaf outline state; `tab_bar` opts a single-row leaf into the combined
        // tab + body silhouette (see `body_owns_top_border`), and `tabs` records the active tab's
        // rect that the silhouette traces.
        self.body_owns_top_border = false;
        self.active_tab_rect = None;
        let tabbar_rect = if hide_tab_bar {
            Rect::NOTHING
        } else {
            self.tab_bar(
                ui,
                state,
                path,
                tab_viewer,
                fade_style.map(|(style, _)| style),
                collapsed,
            )
        };
        self.tab_body(
            ui,
            state,
            path,
            tab_viewer,
            spacing,
            tabbar_rect,
            fade_style,
            collapsed,
        );

        let tabs = self.dock_state[path]
            .tabs_mut()
            .expect("This node must be a leaf here");
        for (tab_index, tab) in tabs.iter_mut().enumerate() {
            if tab_viewer.force_close(tab) {
                self.to_remove.push(TabRemoval::Tab(
                    (path, TabIndex(tab_index)).into(),
                    ForcedRemoval(true),
                ));
            }
        }
    }

    fn tab_bar(
        &mut self,
        ui: &mut Ui,
        state: &mut State,
        path: NodePath,
        tab_viewer: &mut impl TabViewer<Tab = Tab>,
        fade_style: Option<&Style>,
        collapsed: bool,
    ) -> Rect {
        assert!(self.dock_state[path].is_leaf());

        // Pre-check for multi-row mode before allocating any height.
        if self.multi_row_tabs {
            let inner_margin_h = {
                let style = fade_style.unwrap_or_else(|| self.style.as_ref().unwrap());
                style.tab_bar.inner_margin.sum().x
            };
            // Tab rows use the full inner width; toolbar controls sit on the drag strip only, so
            // do not subtract their width when deciding how many tabs fit per row.
            let inner_tab_row_width = (ui.available_width() - inner_margin_h).max(0.0);
            if inner_tab_row_width > 0.0 {
                let tab_layout = self.compute_tab_layout(ui, path, tab_viewer, fade_style);
                let row_ranges = distribute_tabs_for_width(&tab_layout, inner_tab_row_width);
                if row_ranges.len() >= 2 {
                    return self.tab_bar_multi_row(
                        ui,
                        state,
                        path,
                        tab_viewer,
                        fade_style,
                        collapsed,
                        &tab_layout,
                        row_ranges,
                    );
                }
            }
        }

        // Single row: the active tab and body are stroked as one silhouette by `tab_body` (rather
        // than a separate tab outline + seam hline). `tabs` records the active tab's rect that the
        // silhouette traces; `tab_title` skips that tab's own outline.
        self.body_owns_top_border = true;

        let style = fade_style.unwrap_or_else(|| self.style.as_ref().unwrap());
        let (tabbar_outer_rect, tabbar_response) = ui.allocate_exact_size(
            vec2(ui.available_width().max(0.0), style.tab_bar.height),
            Sense::hover(),
        );
        ui.painter().rect_filled(
            tabbar_outer_rect,
            style.tab_bar.corner_radius,
            style.tab_bar.bg_fill,
        );

        let tabbar_outer_rect = tabbar_outer_rect - style.tab_bar.inner_margin;

        let mut available_width = tabbar_outer_rect.width().max(0.0);
        let scroll_bar_width = available_width;
        if available_width == 0.0 {
            return tabbar_outer_rect;
        }

        // Reserve space for the buttons at the ends of the tab bar.
        // Also track the total width of right-side buttons for empty-space detection.
        let mut buttons_right_width = 0.0_f32;

        if self.show_add_buttons {
            available_width -= Style::TAB_ADD_BUTTON_SIZE;
            buttons_right_width += Style::TAB_ADD_BUTTON_SIZE;
        }

        if self.show_leaf_close_all_buttons {
            available_width -= Style::TAB_CLOSE_ALL_BUTTON_SIZE;
            buttons_right_width += Style::TAB_CLOSE_ALL_BUTTON_SIZE;
        }

        if self.show_leaf_collapse_buttons {
            available_width -= Style::TAB_COLLAPSE_BUTTON_SIZE;
        }

        let (actual_width, tab_hovered, tabs_right) = {
            let leaf = self
                .dock_state
                .leaf_mut(path)
                .expect("This node must be a leaf");

            let tabbar_inner_rect = Rect::from_min_size(
                (tabbar_outer_rect.min - pos2(-leaf.scroll, 0.0)
                    + vec2(
                        if self.show_leaf_collapse_buttons {
                            Style::TAB_COLLAPSE_BUTTON_SIZE
                        } else {
                            0.0
                        },
                        0.0,
                    ))
                .to_pos2(),
                vec2(tabbar_outer_rect.width(), tabbar_outer_rect.height()),
            );

            let tabs_ui = &mut ui.new_child(
                UiBuilder::new()
                    .max_rect(tabbar_inner_rect)
                    .layout(Layout::left_to_right(Align::Center))
                    .id_salt("tabs"),
            );

            let mut clip_rect = tabbar_outer_rect;
            clip_rect.set_width(available_width);
            if self.show_leaf_collapse_buttons {
                clip_rect = clip_rect.translate(vec2(Style::TAB_COLLAPSE_BUTTON_SIZE, 0.0));
            }
            tabs_ui.set_clip_rect(clip_rect);

            // fill_tab_bar: split extra horizontal space evenly across tabs (same slack, not same width).
            let tabs_len = leaf.tabs.len();
            let tab_layout_fill = (style.tab_bar.fill_tab_bar && tabs_len > 0)
                .then(|| self.compute_tab_layout(ui, path, tab_viewer, fade_style));
            let fill_row_width = tab_layout_fill.as_ref().map(|_| available_width);

            let tab_hovered = self.tabs(
                tabs_ui,
                state,
                path,
                tab_viewer,
                tab_layout_fill.as_deref(),
                fill_row_width,
                fade_style,
                0..tabs_len,
            );

            // The body draws the top border itself (see `tab_body`), so no seam is painted here.
            // Re-borrow the style after the `&mut self` tab drawing above for the buttons below.
            let style = fade_style.unwrap_or_else(|| self.style.as_ref().unwrap());

            // Add button at the ends of the tab bar.
            if self.show_add_buttons {
                let offset = match style.buttons.add_tab_align {
                    TabAddAlign::Left => {
                        (clip_rect.width() - tabs_ui.min_rect().width()).at_least(0.0)
                    }
                    TabAddAlign::Right => 0.0,
                } + if self.show_leaf_close_all_buttons {
                    Style::TAB_CLOSE_ALL_BUTTON_SIZE
                } else {
                    0.0
                };
                self.tab_plus(ui, path, tab_viewer, tabbar_outer_rect, offset, fade_style);
            }

            if self.show_leaf_close_all_buttons {
                // Current leaf contains non-closable tabs.
                let disabled = self
                    .dock_state
                    .leaf_mut(path)
                    .map(|leaf| !leaf.tabs.iter_mut().all(|tab| tab_viewer.is_closeable(tab)))
                    .expect("This node must be a leaf");

                // Current window contains non-closable tabs.
                let close_window_disabled = disabled
                    || !self.dock_state[path.surface].iter_mut().all(|node| {
                        node.get_leaf_mut().is_none_or(|leaf| {
                            leaf.tabs.iter_mut().all(|tab| tab_viewer.is_closeable(tab))
                        })
                    });

                self.tab_close_all(
                    ui,
                    path,
                    tabbar_outer_rect,
                    fade_style,
                    disabled,
                    close_window_disabled,
                )
            }

            if self.show_leaf_collapse_buttons {
                self.tab_collapse(ui, path, tabbar_outer_rect, fade_style, collapsed)
            }

            (
                tabs_ui.min_rect().width(),
                tab_hovered,
                tabs_ui.min_rect().right(),
            )
        };

        self.tab_bar_scroll(
            ui,
            state,
            path,
            actual_width,
            available_width,
            scroll_bar_width,
            &tabbar_response,
            tab_hovered,
            fade_style,
        );

        // Node group drag: allow dragging all tabs in this leaf by grabbing the empty
        // space in the tab bar (the area after the rendered tabs, before right-side buttons).
        if self.draggable_tabs && actual_width < available_width {
            let empty_space_right = tabbar_outer_rect.right() - buttons_right_width;
            if tabs_right < empty_space_right {
                let empty_rect = Rect::from_x_y_ranges(
                    tabs_right..=empty_space_right,
                    tabbar_outer_rect.y_range(),
                );
                let node_drag_id = self
                    .id
                    .with((path.surface, "surface"))
                    .with((path.node, "node_group_drag"));
                let response = ui.interact(empty_rect, node_drag_id, Sense::click_and_drag());

                let is_being_dragged = ui.ctx().is_being_dragged(node_drag_id)
                    && ui.input(|i| i.pointer.is_decidedly_dragging());

                if is_being_dragged {
                    ui.output_mut(|o| o.cursor_icon = CursorIcon::Grabbing);
                    if let Some(pointer_pos) = ui.ctx().pointer_interact_pos() {
                        let start = *state.drag_start.get_or_insert(pointer_pos);
                        let delta = pointer_pos - start;
                        if delta.x.abs() > 30.0 || delta.y.abs() > 6.0 {
                            let node_rect = self.dock_state[path].rect().unwrap_or(Rect::NOTHING);
                            ui.memory_mut(|mem| {
                                mem.data.insert_temp(
                                    self.id.with("drag_data"),
                                    Some(DragData {
                                        src: TreeComponent::Node(path),
                                        rect: node_rect,
                                    }),
                                );
                            });
                        }
                    }
                } else if response.hovered() {
                    ui.output_mut(|o| o.cursor_icon = CursorIcon::Grab);
                }
            }
        }

        tabbar_outer_rect
    }

    /// Renders a contiguous range of tabs.
    ///
    /// When `tab_layout` and `fill_row_width` are both set, widths come from
    /// [`compute_tab_widths_for_row`] (equal slack on top of each tab's minimum). Otherwise each
    /// tab uses its natural minimum width (see [`Self::tab_title`]).
    ///
    /// `tab_layout` is `(min_width, gap_before_this_tab)` per index from [`Self::compute_tab_layout`].
    /// `fill_row_width` is the clip width for this row; use `Some` together with `tab_layout`, or
    /// `None` for both.
    #[allow(clippy::too_many_arguments)]
    fn tabs(
        &mut self,
        tabs_ui: &mut Ui,
        state: &mut State,
        path: NodePath,
        tab_viewer: &mut impl TabViewer<Tab = Tab>,
        tab_layout: Option<&[(f32, f32)]>,
        fill_row_width: Option<f32>,
        fade: Option<&Style>,
        tab_range: Range<usize>,
    ) -> bool {
        let mut tab_hovered = false;

        assert!(self.dock_state[path].is_leaf());

        let focused = self.dock_state.focused_leaf();
        let range_start = tab_range.start;

        let row_target_widths = match (tab_layout, fill_row_width) {
            (Some(layout), Some(row_w)) => {
                Some(compute_tab_widths_for_row(row_w, layout, &tab_range))
            }
            _ => None,
        };

        for tab_index in tab_range {
            let id = self
                .id
                .with((path.surface, "surface"))
                .with((path.node, "node"))
                .with((tab_index, "tab"));
            let tab_index = TabIndex(tab_index);
            let is_being_dragged = tabs_ui.ctx().is_being_dragged(id)
                && tabs_ui.input(|i| i.pointer.is_decidedly_dragging())
                && self.draggable_tabs;

            if is_being_dragged {
                tabs_ui.output_mut(|o| o.cursor_icon = CursorIcon::Grabbing);
            }

            let (is_active, label, tab_style, closeable) = {
                let leaf = self.dock_state[path]
                    .get_leaf_mut()
                    .expect("This node must be a leaf");
                let style = fade.unwrap_or_else(|| self.style.as_ref().unwrap());
                let tab_style = tab_viewer.tab_style_override(&leaf.tabs[tab_index.0], &style.tab);
                (
                    leaf.active == tab_index || is_being_dragged,
                    tab_viewer.title(&mut leaf.tabs[tab_index.0]),
                    tab_style.unwrap_or(style.tab.clone()),
                    tab_viewer.is_closeable(&leaf.tabs[tab_index.0]),
                )
            };

            let show_close_button = self.show_close_buttons && closeable;

            let target_width = row_target_widths
                .as_ref()
                .map(|w| w[tab_index.0 - range_start]);

            let (response, title_id) = if is_being_dragged {
                let layer_id = LayerId::new(Order::Tooltip, id);
                let response = tabs_ui
                    .scope_builder(UiBuilder::new().layer_id(layer_id), |ui| {
                        self.tab_title(
                            ui,
                            &tab_style,
                            id,
                            label,
                            is_active && Some(path) == focused,
                            is_active,
                            is_being_dragged,
                            target_width,
                            show_close_button,
                            fade,
                        )
                    })
                    .response;
                let title_id = response.id;

                let response =
                    tabs_ui.interact(response.rect, id.with("dragged"), Sense::click_and_drag());

                if let Some(pointer_pos) = tabs_ui.ctx().pointer_interact_pos() {
                    let start = *state.drag_start.get_or_insert(pointer_pos);
                    let delta = pointer_pos - start;
                    if delta.x.abs() > 30.0 || delta.y.abs() > 6.0 {
                        tabs_ui
                            .ctx()
                            .transform_layer_shapes(layer_id, TSTransform::new(delta, 1.0));

                        tabs_ui.memory_mut(|mem| {
                            mem.data.insert_temp(
                                self.id.with("drag_data"),
                                Some(DragData {
                                    src: TreeComponent::Tab((path, tab_index).into()),
                                    rect: self.dock_state[path].rect().unwrap(),
                                }),
                            );
                        });
                    }
                }

                (response, title_id)
            } else {
                if tab_index.0 != range_start {
                    tabs_ui.allocate_space(vec2(tab_style.spacing, 0.0));
                }
                let (mut response, close_response) = self.tab_title(
                    tabs_ui,
                    &tab_style,
                    id,
                    label,
                    is_active && Some(path) == focused,
                    is_active,
                    is_being_dragged,
                    target_width,
                    show_close_button,
                    fade,
                );
                let title_id = response.id;
                let close_clicked = close_response.is_some_and(|res| res.clicked());
                let is_lonely_tab = self.dock_state[path.surface].num_tabs() == 1;

                if self.show_tab_name_on_hover {
                    let tabs = self.dock_state[path]
                        .tabs_mut()
                        .expect("This node must be a leaf");
                    let tab = &mut tabs[tab_index.0];
                    response = response.on_hover_ui(|ui| {
                        ui.label(tab_viewer.title(tab));
                    });
                }

                if self.tab_context_menus {
                    let eject_button =
                        Button::new(&self.dock_state.translations.tab_context_menu.eject_button);
                    let close_button =
                        Button::new(&self.dock_state.translations.tab_context_menu.close_button);

                    response.context_menu(|ui| {
                        let leaf = self.dock_state[path]
                            .get_leaf_mut()
                            .expect("This node must be a leaf");
                        let tab = &mut leaf.tabs[tab_index.0];

                        tab_viewer.context_menu(ui, tab, path);
                        if (path.surface.is_main() || !is_lonely_tab)
                            && tab_viewer.allowed_in_windows(tab)
                            && ui.add(eject_button).clicked()
                        {
                            self.to_detach.push((path, tab_index).into());
                            ui.close();
                        }
                        if show_close_button && ui.add(close_button).clicked() {
                            match tab_viewer.on_close(tab) {
                                OnCloseResponse::Close => self.to_remove.push(TabRemoval::Tab(
                                    (path, tab_index).into(),
                                    ForcedRemoval(false),
                                )),
                                OnCloseResponse::Focus => {
                                    leaf.active = tab_index;
                                    self.new_focused = Some(path);
                                }
                                OnCloseResponse::Ignore => (),
                            }
                            ui.close();
                        }
                    });
                }

                if close_clicked {
                    self.to_remove.push(TabRemoval::Tab(
                        (path, tab_index).into(),
                        ForcedRemoval(false),
                    ));
                }

                if let Some(pos) = state.last_hover_pos {
                    // Use response.rect.contains instead of
                    // response.hovered as the dragged tab covers
                    // the underlying tab
                    if state.drag_start.is_some() && response.rect.contains(pos) {
                        self.tab_hover_rect = Some((response.rect, tab_index));
                    }
                }

                (response, title_id)
            };

            if response.hovered() {
                tab_hovered = true;
            }

            // Record the active tab's rect so the body can trace the active tab + body as one
            // continuous silhouette (see `tab_body`); `tab_title` correspondingly skips this tab's
            // own outline + connector. An inactive tab — or an active tab that opts into an hline
            // beneath its name — keeps its own outline and just sits on the body's top border.
            let leaf = self.dock_state.leaf_mut(path).unwrap();
            let tab = &mut leaf.tabs[tab_index.0];
            let style = fade.unwrap_or_else(|| self.style.as_ref().unwrap());
            let tab_style = tab_viewer.tab_style_override(tab, &style.tab);
            let tab_style = tab_style.as_ref().unwrap_or(&style.tab);

            if is_active && !is_being_dragged && !tab_style.hline_below_active_tab_name {
                self.active_tab_rect = Some(response.rect);
            }

            if response.clicked()
                || (tabs_ui.memory(|m| m.has_focus(title_id))
                    && tabs_ui.input(|i| i.key_pressed(Key::Enter) || i.key_pressed(Key::Space)))
            {
                leaf.active = tab_index;
                self.new_focused = Some(path);
            }

            tab_viewer.on_tab_button(tab, &response);

            if self.show_close_buttons && tab_viewer.is_closeable(tab) && response.middle_clicked()
            {
                self.to_remove.push(TabRemoval::Tab(
                    (path, tab_index).into(),
                    ForcedRemoval(false),
                ));
            }
        }

        tab_hovered
    }

    /// Draws the tab add button.
    #[allow(clippy::too_many_arguments)]
    fn tab_plus(
        &mut self,
        ui: &mut Ui,
        path: NodePath,
        tab_viewer: &mut impl TabViewer<Tab = Tab>,
        tabbar_outer_rect: Rect,
        offset: f32,
        fade_style: Option<&Style>,
    ) {
        let rect = Rect::from_min_max(
            tabbar_outer_rect.right_top() - vec2(Style::TAB_ADD_BUTTON_SIZE + offset, 0.0),
            tabbar_outer_rect.right_bottom() - vec2(offset, 2.0),
        );

        let ui = &mut ui.new_child(
            UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::left_to_right(Align::Center))
                .id_salt((path.node, "tab_add")),
        );

        let (rect, mut response) =
            ui.allocate_exact_size(ui.available_size().max(Vec2::ZERO), Sense::click());

        response = response.on_hover_cursor(CursorIcon::PointingHand);

        let style = fade_style.unwrap_or_else(|| self.style.as_ref().unwrap());
        let color = if response.hovered() || response.has_focus() {
            ui.painter()
                .rect_filled(rect, CornerRadius::ZERO, style.buttons.add_tab_bg_fill);
            style.buttons.add_tab_active_color
        } else {
            style.buttons.add_tab_color
        };

        let mut plus_rect = rect;

        rect_set_size_centered(&mut plus_rect, Vec2::splat(Style::TAB_ADD_PLUS_SIZE));

        ui.painter().line_segment(
            [plus_rect.center_top(), plus_rect.center_bottom()],
            Stroke::new(1.0, color),
        );
        ui.painter().line_segment(
            [plus_rect.right_center(), plus_rect.left_center()],
            Stroke::new(1.0, color),
        );

        // Draw button left border.
        ui.painter().vline(
            rect.left(),
            rect.y_range(),
            Stroke::new(
                ui.ctx().pixels_per_point().recip(),
                style.buttons.add_tab_border_color,
            ),
        );

        let popup_id = ui.id().with("tab_add_popup");
        if self.show_add_popup {
            Popup::from_toggle_button_response(&response)
                .id(popup_id)
                .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| {
                    tab_viewer.add_popup(ui, path);
                });
        }

        if response.clicked() {
            tab_viewer.on_add(path);
        }
    }

    /// Draws the close all button.
    #[allow(clippy::too_many_arguments)]
    #[allow(unused_assignments)]
    fn tab_close_all(
        &mut self,
        ui: &mut Ui,
        path: NodePath,
        tabbar_outer_rect: Rect,
        fade_style: Option<&Style>,
        disabled: bool,
        close_window_disabled: bool,
    ) {
        let rect = Rect::from_min_max(
            tabbar_outer_rect.right_top() - vec2(Style::TAB_CLOSE_ALL_BUTTON_SIZE, 0.0),
            tabbar_outer_rect.right_bottom() - vec2(0.0, 2.0),
        );

        let ui = &mut ui.new_child(
            UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::left_to_right(Align::Center))
                .id_salt((path.node, "tab_close_all")),
        );

        let (rect, mut response) =
            ui.allocate_exact_size(ui.available_size().max(Vec2::ZERO), Sense::click());

        let style = fade_style.unwrap_or_else(|| self.style.as_ref().unwrap());

        // Whether we're on "secondary button mode" due to modifier keys
        let on_secondary_button = self.is_on_secondary_button(path.surface, ui, &response);

        let mut stroke_color = if disabled {
            style.buttons.close_all_tabs_disabled_color
        } else if response.hovered() || response.has_focus() {
            if !(close_window_disabled && on_secondary_button) {
                ui.painter().rect_filled(
                    rect,
                    CornerRadius::ZERO,
                    style.buttons.close_all_tabs_bg_fill,
                );
            }
            style.buttons.close_all_tabs_active_color
        } else {
            style.buttons.close_all_tabs_color
        };

        let mut close_all_rect = rect;

        rect_set_size_centered(&mut close_all_rect, Vec2::splat(Style::TAB_CLOSE_ALL_SIZE));

        if !disabled {
            response = response.on_hover_cursor(CursorIcon::PointingHand);
        }

        if on_secondary_button {
            // Close the entire window
            if close_window_disabled {
                stroke_color = style.buttons.close_all_tabs_disabled_color;
                response = response
                    .on_hover_cursor(CursorIcon::NotAllowed)
                    .on_hover_text(
                        self.dock_state
                            .translations
                            .leaf
                            .close_all_button_disabled_tooltip
                            .as_str(),
                    );
            }
            Self::draw_close_window_symbol(ui, stroke_color, close_all_rect);
        } else {
            // Close all tabs in this leaf
            if !disabled {
                if !path.surface.is_main() && self.secondary_button_context_menu {
                    response.context_menu(|ui| {
                        ui.add_enabled_ui(!close_window_disabled, |ui| {
                            if ui
                                .button(&self.dock_state.translations.leaf.close_all_button)
                                .on_disabled_hover_text(
                                    self.dock_state
                                        .translations
                                        .leaf
                                        .close_all_button_disabled_tooltip
                                        .as_str(),
                                )
                                .clicked()
                            {
                                self.to_remove.push(TabRemoval::Window(path.surface));
                            }
                        });
                    });
                }
            } else {
                response = response
                    .on_hover_cursor(CursorIcon::NotAllowed)
                    .on_hover_text(
                        self.dock_state
                            .translations
                            .leaf
                            .close_button_disabled_tooltip
                            .as_str(),
                    );
            }

            if response.clicked() {
                if on_secondary_button {
                    if !close_window_disabled {
                        self.to_remove.push(TabRemoval::Window(path.surface));
                    }
                } else if !disabled {
                    self.to_remove.push(TabRemoval::Node(path));
                }
            }

            ui.painter().line_segment(
                [close_all_rect.left_top(), close_all_rect.right_bottom()],
                Stroke::new(1.0, stroke_color),
            );
            ui.painter().line_segment(
                [close_all_rect.right_top(), close_all_rect.left_bottom()],
                Stroke::new(1.0, stroke_color),
            );
        }

        // Draw button left border.
        ui.painter().vline(
            rect.left(),
            rect.y_range(),
            Stroke::new(
                ui.ctx().pixels_per_point().recip(),
                style.buttons.close_all_tabs_border_color,
            ),
        );

        if !disabled && !on_secondary_button {
            response = self.show_tooltip_hints(path.surface, response);
        }
    }

    /// Draws the collapse button.
    fn tab_collapse(
        &mut self,
        ui: &mut Ui,
        path: NodePath,
        tabbar_outer_rect: Rect,
        fade_style: Option<&Style>,
        collapsed: bool,
    ) {
        let rect = Rect::from_min_max(
            tabbar_outer_rect.left_top(),
            tabbar_outer_rect.left_bottom() + vec2(Style::TAB_COLLAPSE_BUTTON_SIZE, 0.0),
        );

        let ui = &mut ui.new_child(
            UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::left_to_right(Align::Center))
                .id_salt((path.node, "tab_collapse")),
        );

        let (rect, mut response) =
            ui.allocate_exact_size(ui.available_size().max(Vec2::ZERO), Sense::click());

        response = response.on_hover_cursor(CursorIcon::PointingHand);

        let style = fade_style.unwrap_or_else(|| self.style.as_ref().unwrap());

        // Whether we're on "secondary button mode" due to modifier keys
        let on_secondary_button = self.is_on_secondary_button(path.surface, ui, &response);

        let color = if response.hovered() || response.has_focus() {
            ui.painter().rect_filled(
                rect,
                CornerRadius::ZERO,
                style.buttons.collapse_tabs_bg_fill,
            );
            style.buttons.collapse_tabs_active_color
        } else {
            style.buttons.collapse_tabs_color
        };

        let mut arrow_rect = rect;
        rect_set_size_centered(&mut arrow_rect, Vec2::splat(Style::TAB_COLLAPSE_ARROW_SIZE));

        if on_secondary_button {
            // Collapse the entire window
            Self::draw_chevron_down(ui, style, color, arrow_rect);
        } else {
            // Draw arrow.
            Self::draw_arrow(collapsed, ui, color, arrow_rect);
        }

        // Draw button right border.
        ui.painter().vline(
            rect.right(),
            rect.y_range(),
            Stroke::new(
                ui.ctx().pixels_per_point().recip(),
                style.buttons.collapse_tabs_border_color,
            ),
        );

        if response.clicked() {
            if on_secondary_button {
                self.window_toggle_minimized(path.surface);
            } else {
                self.dock_state[path].set_collapsed(!collapsed);
                self.dock_state[path.surface].node_update_collapsed(path.node);
                self.window_update_collapsed(path);
            }
        }

        if !path.surface.is_main() && self.secondary_button_context_menu {
            response.context_menu(|ui| {
                if ui
                    .button(&self.dock_state.translations.leaf.minimize_button)
                    .clicked()
                {
                    ui.close();
                    self.window_toggle_minimized(path.surface);
                }
            });
        }

        if !on_secondary_button {
            self.show_tooltip_hints(path.surface, response);
        }
    }

    fn show_tooltip_hints(&mut self, surface_index: SurfaceIndex, response: Response) -> Response {
        if !surface_index.is_main()
            && self.show_secondary_button_hint
            && (self.secondary_button_context_menu || self.secondary_button_on_modifier)
        {
            let hint = if self.secondary_button_context_menu && self.secondary_button_on_modifier {
                &self
                    .dock_state
                    .translations
                    .leaf
                    .minimize_button_modifier_menu_hint
            } else if self.secondary_button_context_menu {
                &self.dock_state.translations.leaf.minimize_button_menu_hint
            } else {
                &self
                    .dock_state
                    .translations
                    .leaf
                    .minimize_button_modifier_hint
            };
            return response.on_hover_text(hint);
        }
        response
    }

    fn is_on_secondary_button(
        &self,
        surface_index: SurfaceIndex,
        ui: &mut Ui,
        response: &Response,
    ) -> bool {
        !surface_index.is_main()
            && self.secondary_button_on_modifier
            && ui.input(|i| {
                i.modifiers
                    .matches_logically(self.secondary_button_modifiers)
            })
            && (response.hovered() || response.has_focus() || response.is_pointer_button_down_on())
    }

    fn draw_close_window_symbol(ui: &mut Ui, stroke_color: Color32, close_all_rect: Rect) {
        ui.painter().add(Shape::line(
            vec![
                close_all_rect
                    .right_center()
                    .lerp(close_all_rect.right_bottom(), 0.5),
                close_all_rect.right_bottom(),
                close_all_rect.left_bottom(),
                close_all_rect.left_top(),
                close_all_rect
                    .center_top()
                    .lerp(close_all_rect.left_top(), 0.5),
            ],
            Stroke::new(1.0, stroke_color),
        ));
        ui.painter().line_segment(
            [close_all_rect.center_top(), close_all_rect.right_center()],
            Stroke::new(1.0, stroke_color),
        );
        ui.painter().line_segment(
            [close_all_rect.center(), close_all_rect.right_top()],
            Stroke::new(1.0, stroke_color),
        );
    }

    fn draw_arrow(collapsed: bool, ui: &mut Ui, color: Color32, arrow_rect: Rect) {
        ui.painter().add(Shape::convex_polygon(
            if collapsed {
                // Arrow pointing rightwards.
                vec![
                    arrow_rect.left_top(),
                    arrow_rect.right_center(),
                    arrow_rect.left_bottom(),
                ]
            } else {
                // Arrow pointing downwards.
                vec![
                    arrow_rect.left_top(),
                    arrow_rect.right_top(),
                    arrow_rect.center_bottom(),
                ]
            },
            color,
            Stroke::NONE,
        ));
    }

    fn draw_chevron_down(ui: &mut Ui, style: &Style, color: Color32, arrow_rect: Rect) {
        ui.painter().add(Shape::convex_polygon(
            // Arrow pointing downwards.
            vec![
                arrow_rect.left_top(),
                arrow_rect.right_top(),
                arrow_rect.center(),
            ],
            color,
            Stroke::NONE,
        ));

        // Chevron pointing downwards.
        ui.painter().add(Shape::convex_polygon(
            vec![
                arrow_rect.left_center(),
                arrow_rect.right_center(),
                arrow_rect.center_bottom(),
            ],
            color,
            Stroke::NONE,
        ));
        let color = style.buttons.minimize_window_bg_fill;
        ui.painter().add(Shape::convex_polygon(
            vec![
                arrow_rect
                    .left_center()
                    .lerp(arrow_rect.right_center(), 0.25),
                arrow_rect
                    .left_center()
                    .lerp(arrow_rect.right_center(), 0.75),
                arrow_rect.center().lerp(arrow_rect.center_bottom(), 0.5),
            ],
            color,
            Stroke::NONE,
        ));
    }

    /// Updates the collapsed state of the node and its parents.
    fn window_update_collapsed(&mut self, path: NodePath) {
        let surface = &mut self.dock_state[path.surface];
        let collapsed = surface[path.node].is_collapsed();
        if !collapsed {
            if let Some(window_state) = self.dock_state.get_window_state_mut(path.surface) {
                window_state.set_new(true);
            }
        } else if surface.root_node().is_some_and(|root| root.is_collapsed()) {
            let root_index = NodeIndex::root();
            let surface_height = if surface.root_node().is_some() {
                surface[root_index].rect().unwrap().height()
            } else {
                0.0
            };
            if let Some(window_state) = self.dock_state.get_window_state_mut(path.surface) {
                window_state.set_expanded_height(surface_height);
            }
        }
    }

    /// * `active` means "the tab that is opened in the parent panel".
    /// * `focused` means "the tab that was last interacted with".
    /// * `target_width` - if set, this width is used exactly (may be below the label minimum when
    ///   the row is squeezed); if `None`, the tab uses its natural minimum width.
    ///
    /// Returns the main button response plus the response of the close button, if any.
    #[allow(clippy::too_many_arguments)]
    fn tab_title(
        &mut self,
        ui: &mut Ui,
        tab_style: &TabStyle,
        id: Id,
        label: WidgetText,
        focused: bool,
        active: bool,
        is_being_dragged: bool,
        target_width: Option<f32>,
        show_close_button: bool,
        fade: Option<&Style>,
    ) -> (Response, Option<Response>) {
        let style = fade.unwrap_or_else(|| self.style.as_ref().unwrap());
        let galley = label.into_galley(ui, None, f32::INFINITY, TextStyle::Button);
        let x_spacing = 8.0;
        let text_width = galley.size().x + 2.0 * x_spacing;
        let close_button_size = if show_close_button {
            Style::TAB_CLOSE_BUTTON_SIZE.min(style.tab_bar.height)
        } else {
            0.0
        };

        // Minimum width so the label and optional close control fit.
        let minimum_width = tab_style
            .minimum_width
            .unwrap_or(0.0)
            .at_least(text_width + close_button_size);
        let tab_width = match target_width {
            None => minimum_width,
            Some(w) => w.max(0.0),
        };

        let (_, tab_rect) = ui.allocate_space(vec2(tab_width, ui.available_height().max(0.0)));
        let mut response = ui.interact(tab_rect, id, Sense::click_and_drag());
        if ui.ctx().dragged_id().is_none() && self.draggable_tabs {
            response = response.on_hover_cursor(CursorIcon::Grab);
        }

        // In a single-row leaf the active tab merges into the body: its outline and the body's are
        // stroked as one continuous silhouette by `tab_body`, so skip this tab's own outline and
        // name-area connector here. A dragged tab floats free (full outline), and an active tab that
        // opts into an hline beneath its name keeps the legacy look.
        let merge_active = active
            && !is_being_dragged
            && self.body_owns_top_border
            && !tab_style.hline_below_active_tab_name;
        // Every docked tab is pulled down at the top by the stroke inset so its outline doesn't sit
        // flush against the separator/row above; the fill is pulled down by the same amount (below)
        // so fill and outline coincide — unlike the legacy per-side `rect_stroke_box` inset, which
        // left the fill bleeding past the outline. Must match `tab_body`'s silhouette inset.
        // (Captured before `tab_style` is narrowed to a `TabInteractionStyle` below.)
        let tab_outline_width = tab_style.tab_body.stroke.width;
        let tab_top_inset = (tab_outline_width / 2.0).ceil();

        let tab_style = if focused || is_being_dragged {
            if response.has_focus() {
                &tab_style.focused_with_kb_focus
            } else {
                &tab_style.focused
            }
        } else if active {
            if response.has_focus() {
                &tab_style.active_with_kb_focus
            } else {
                &tab_style.active
            }
        } else if response.hovered() {
            &tab_style.hovered
        } else if response.has_focus() {
            &tab_style.inactive_with_kb_focus
        } else {
            &tab_style.inactive
        };

        // Draw the fill first, then the outline on top so the stroke doesn't mix with the fill.
        // A floating (dragged) tab fills its whole rect; every docked tab is pulled down at the top
        // so the fill matches the outline and leaves the separator gap above. The outline never
        // closes across the bottom — the line below (the body's top border for a single-row leaf,
        // or the row separator for a multi-row one) is the tab's bottom — and there is no `bg_fill`
        // connector, which is what produced the seam under inactive tabs (worse on hover).
        let fill_rect = if is_being_dragged {
            tab_rect
        } else {
            Rect::from_min_max(
                pos2(tab_rect.left(), tab_rect.top() + tab_top_inset),
                tab_rect.max,
            )
        };
        ui.painter()
            .rect_filled(fill_rect, tab_style.corner_radius, tab_style.bg_fill);

        let tab_outline = Stroke::new(tab_outline_width, tab_style.outline_color);
        if merge_active {
            // Active tab in a single-row leaf: its outline is part of the body silhouette (drawn by
            // `tab_body`), so nothing is stroked here.
        } else if is_being_dragged {
            // Floating tab: a full closed box, since it sits over nothing.
            ui.painter().rect_stroke(
                rect_stroke_box(tab_rect, tab_outline.width),
                tab_style.corner_radius,
                tab_outline,
                StrokeKind::Inside,
            );
        } else if self.body_owns_top_border {
            // Inactive tab in a single-row leaf: a rounded-top cap whose sides run all the way down
            // to the body's top border (which `tab_body` draws under it). No bottom edge, so it
            // meets that border in a clean T instead of doubling it.
            let cap = build_tab_cap_outline(tab_rect, tab_style.corner_radius, tab_top_inset);
            ui.painter().add(Shape::Path(PathShape {
                points: cap,
                closed: false,
                fill: Color32::TRANSPARENT,
                stroke: PathStroke::new(tab_outline.width, tab_outline.color)
                    .with_kind(StrokeKind::Inside),
            }));
        } else {
            // Multi-row tab: a full box whose bottom reaches the row separator below it (drawn by
            // `tab_bar_multi_row`), so the sides run the whole way down and the fill stays flush
            // with the outline.
            ui.painter().rect_stroke(
                fill_rect,
                tab_style.corner_radius,
                tab_outline,
                StrokeKind::Inside,
            );
        }

        let mut text_rect = tab_rect;
        text_rect.set_width(text_rect.width() - close_button_size);
        let text_pos = {
            let pos = Align2::CENTER_CENTER.pos_in_rect(&text_rect.shrink2(vec2(x_spacing, 0.0)));
            pos - galley.size() / 2.0
        };

        ui.painter()
            .add(TextShape::new(text_pos, galley, tab_style.text_color));

        let close_response = show_close_button.then(|| {
            let mut close_button_rect = tab_rect;
            close_button_rect.set_left(text_rect.right());
            close_button_rect =
                Rect::from_center_size(close_button_rect.center(), Vec2::splat(close_button_size));

            let close_response = ui
                .interact(close_button_rect, id.with("close-button"), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand);

            let color = if close_response.hovered() || close_response.has_focus() {
                style.buttons.close_tab_active_color
            } else {
                style.buttons.close_tab_color
            };

            if close_response.hovered() || close_response.has_focus() {
                let mut corner_radius = tab_style.corner_radius;
                corner_radius.nw = 0;
                corner_radius.sw = 0;

                ui.painter().rect_filled(
                    close_button_rect,
                    corner_radius,
                    style.buttons.close_tab_bg_fill,
                );
            }

            let mut x_rect = close_button_rect;
            rect_set_size_centered(&mut x_rect, Vec2::splat(Style::TAB_CLOSE_X_SIZE));
            ui.painter().line_segment(
                [x_rect.left_top(), x_rect.right_bottom()],
                Stroke::new(1.0, color),
            );
            ui.painter().line_segment(
                [x_rect.right_top(), x_rect.left_bottom()],
                Stroke::new(1.0, color),
            );

            close_response
        });

        (response, close_response)
    }

    #[allow(clippy::too_many_arguments)]
    fn tab_bar_multi_row(
        &mut self,
        ui: &mut Ui,
        state: &mut State,
        path: NodePath,
        tab_viewer: &mut impl TabViewer<Tab = Tab>,
        fade_style: Option<&Style>,
        collapsed: bool,
        tab_layout: &[(f32, f32)],
        row_ranges: Vec<Range<usize>>,
    ) -> Rect {
        let style = fade_style.unwrap_or_else(|| self.style.as_ref().unwrap());
        let row_height = style.tab_bar.height;
        let rows = row_ranges.len();
        let total_height = row_height * (rows as f32 + 1.0);

        let (outer_rect, _) = ui.allocate_exact_size(
            vec2(ui.available_width().max(0.0), total_height.max(0.0)),
            Sense::hover(),
        );
        ui.painter().rect_filled(
            outer_rect,
            style.tab_bar.corner_radius,
            style.tab_bar.bg_fill,
        );

        let inner_rect = outer_rect - style.tab_bar.inner_margin;
        let inner_width = inner_rect.width().max(0.0);
        let drag_strip_rect = Rect::from_min_size(inner_rect.min, vec2(inner_width, row_height));

        // Node group drag interaction on the drag strip.
        if self.draggable_tabs {
            let mut drag_left = drag_strip_rect.min.x;
            let mut drag_right = drag_strip_rect.max.x;
            if self.show_leaf_collapse_buttons {
                drag_left += Style::TAB_COLLAPSE_BUTTON_SIZE;
            }
            if self.show_add_buttons {
                drag_right -= Style::TAB_ADD_BUTTON_SIZE;
            }
            if self.show_leaf_close_all_buttons {
                drag_right -= Style::TAB_CLOSE_ALL_BUTTON_SIZE;
            }
            if drag_left < drag_right {
                let drag_rect =
                    Rect::from_x_y_ranges(drag_left..=drag_right, drag_strip_rect.y_range());
                let node_drag_id = self
                    .id
                    .with((path.surface, "surface"))
                    .with((path.node, "node_group_drag"));
                let response = ui.interact(drag_rect, node_drag_id, Sense::click_and_drag());
                let is_being_dragged = ui.ctx().is_being_dragged(node_drag_id)
                    && ui.input(|i| i.pointer.is_decidedly_dragging());
                if is_being_dragged {
                    ui.output_mut(|o| o.cursor_icon = CursorIcon::Grabbing);
                    if let Some(pointer_pos) = ui.ctx().pointer_interact_pos() {
                        let start = *state.drag_start.get_or_insert(pointer_pos);
                        let delta = pointer_pos - start;
                        if delta.x.abs() > 30.0 || delta.y.abs() > 6.0 {
                            let node_rect = self.dock_state[path].rect().unwrap_or(Rect::NOTHING);
                            ui.memory_mut(|mem| {
                                mem.data.insert_temp(
                                    self.id.with("drag_data"),
                                    Some(DragData {
                                        src: TreeComponent::Node(path),
                                        rect: node_rect,
                                    }),
                                );
                            });
                        }
                    }
                } else if response.hovered() {
                    ui.output_mut(|o| o.cursor_icon = CursorIcon::Grab);
                }
            }
        }

        // Buttons in the drag strip.
        if self.show_leaf_collapse_buttons {
            self.tab_collapse(ui, path, drag_strip_rect, fade_style, collapsed);
        }
        if self.show_add_buttons {
            let offset = if self.show_leaf_close_all_buttons {
                Style::TAB_CLOSE_ALL_BUTTON_SIZE
            } else {
                0.0
            };
            self.tab_plus(ui, path, tab_viewer, drag_strip_rect, offset, fade_style);
        }
        if self.show_leaf_close_all_buttons {
            let (disabled, close_window_disabled) = {
                let disabled = self
                    .dock_state
                    .leaf_mut(path)
                    .map(|leaf| !leaf.tabs.iter_mut().all(|tab| tab_viewer.is_closeable(tab)))
                    .expect("This node must be a leaf");
                let close_window_disabled = disabled
                    || !self.dock_state[path.surface].iter_mut().all(|node| {
                        node.get_leaf_mut().is_none_or(|leaf| {
                            leaf.tabs.iter_mut().all(|tab| tab_viewer.is_closeable(tab))
                        })
                    });
                (disabled, close_window_disabled)
            };
            self.tab_close_all(
                ui,
                path,
                drag_strip_rect,
                fade_style,
                disabled,
                close_window_disabled,
            );
        }

        // Tab rows below the drag strip.
        for (row_idx, range) in row_ranges.iter().enumerate() {
            let row_top = inner_rect.min.y + row_height * (row_idx as f32 + 1.0);
            let row_rect = Rect::from_min_size(
                pos2(inner_rect.min.x, row_top),
                vec2(inner_width, row_height),
            );
            let tabs_ui = &mut ui.new_child(
                UiBuilder::new()
                    .max_rect(row_rect)
                    .layout(Layout::left_to_right(Align::Center))
                    .id_salt(("mr_tabs", row_idx)),
            );
            tabs_ui.set_clip_rect(row_rect);

            self.tabs(
                tabs_ui,
                state,
                path,
                tab_viewer,
                Some(tab_layout),
                Some(inner_width),
                fade_style,
                range.clone(),
            );

            // The row separator doubles as the body's top border for the last row, so draw it at the
            // body stroke width (matching the body sides and the tabs' outlines) and align it with
            // the tabs' bottom edges (`StrokeKind::Inside` sits the stroke just inside the bottom).
            let style = fade_style.unwrap_or_else(|| self.style.as_ref().unwrap());
            let sep_width = style.tab.tab_body.stroke.width;
            ui.painter().hline(
                row_rect.x_range(),
                row_rect.bottom() - sep_width / 2.0,
                Stroke::new(sep_width, style.tab_bar.hline_color),
            );
        }

        outer_rect
    }

    /// One entry per tab: `(min_width, horizontal_gap_before_this_tab)`.
    /// The gap matches `allocate_space(spacing)` before each tab after the first in a row.
    fn compute_tab_layout(
        &mut self,
        ui: &Ui,
        path: NodePath,
        tab_viewer: &mut impl TabViewer<Tab = Tab>,
        fade_style: Option<&Style>,
    ) -> Vec<(f32, f32)> {
        let style = fade_style.unwrap_or_else(|| self.style.as_ref().unwrap());
        let tab_bar_height = style.tab_bar.height;
        let tabs_len = self.dock_state[path]
            .get_leaf_mut()
            .map(|l| l.tabs.len())
            .unwrap_or(0);
        let mut layout = Vec::with_capacity(tabs_len);
        for tab_index in 0..tabs_len {
            let (label, tab_style_opt, closeable) = {
                let leaf = self.dock_state[path].get_leaf_mut().unwrap();
                let tab = &mut leaf.tabs[tab_index];
                (
                    tab_viewer.title(tab),
                    tab_viewer.tab_style_override(tab, &style.tab),
                    tab_viewer.is_closeable(tab),
                )
            };
            let tab_style = tab_style_opt.unwrap_or_else(|| style.tab.clone());
            let galley = label.into_galley(ui, None, f32::INFINITY, TextStyle::Button);
            let x_spacing = 8.0;
            let text_width = galley.size().x + 2.0 * x_spacing;
            let close_button_size = if self.show_close_buttons && closeable {
                Style::TAB_CLOSE_BUTTON_SIZE.min(tab_bar_height)
            } else {
                0.0
            };
            let min_width = tab_style
                .minimum_width
                .unwrap_or(0.0)
                .max(text_width + close_button_size);
            layout.push((min_width, tab_style.spacing));
        }
        layout
    }

    #[allow(clippy::too_many_arguments)]
    fn tab_bar_scroll(
        &mut self,
        ui: &mut Ui,
        state: &State,
        path: NodePath,
        actual_width: f32,
        available_width: f32,
        scroll_bar_width: f32,
        tabbar_response: &Response,
        tab_hovered: bool,
        fade_style: Option<&Style>,
    ) {
        if available_width <= 0.0 {
            return;
        }

        let leaf = self.dock_state[path]
            .get_leaf_mut()
            .expect("This node must be a leaf");
        let overflow = (actual_width - available_width).at_least(0.0);
        let style = fade_style.unwrap_or_else(|| self.style.as_ref().unwrap());

        // Compare to 1.0 and not 0.0 to avoid drawing a scroll bar due
        // to floating point precision issue during tab drawing.
        if overflow > 1.0 {
            if style.tab_bar.show_scroll_bar_on_overflow {
                // Draw scroll bar
                let bar_height = 7.5;
                let (scroll_bar_rect, _scroll_bar_response) = ui.allocate_exact_size(
                    vec2(scroll_bar_width, bar_height),
                    Sense::click_and_drag(),
                );

                // Compute scroll bar handle position and size.
                let overflow_ratio = actual_width / available_width;
                let scroll_ratio = -leaf.scroll / overflow;

                let scroll_bar_handle_size = overflow_ratio.recip() * scroll_bar_rect.width();
                let scroll_bar_handle_start = lerp(
                    scroll_bar_rect.left()..=scroll_bar_rect.right() - scroll_bar_handle_size,
                    scroll_ratio,
                );
                let scroll_bar_handle_rect = Rect::from_min_size(
                    pos2(scroll_bar_handle_start, scroll_bar_rect.min.y),
                    vec2(scroll_bar_handle_size, bar_height),
                );

                let scroll_bar_handle_response = ui.interact(
                    scroll_bar_handle_rect,
                    self.id.with((path.node, "node")),
                    Sense::drag(),
                );

                // Coefficient to apply to input displacements so that we move the scroll by the correct amount.
                let points_to_scroll_coefficient =
                    overflow / (scroll_bar_rect.width() - scroll_bar_handle_size);

                leaf.scroll -=
                    scroll_bar_handle_response.drag_delta().x * points_to_scroll_coefficient;

                if let Some(pos) = state.last_hover_pos {
                    if scroll_bar_rect.contains(pos) {
                        leaf.scroll += ui
                            .input(|i| i.smooth_scroll_delta.y + i.smooth_scroll_delta.x)
                            * points_to_scroll_coefficient;
                    }
                }

                // Draw the bar.
                ui.painter()
                    .rect_filled(scroll_bar_rect, 0.0, ui.visuals().extreme_bg_color);

                ui.painter().rect_filled(
                    scroll_bar_handle_rect,
                    bar_height / 2.0,
                    ui.visuals()
                        .widgets
                        .style(&scroll_bar_handle_response)
                        .bg_fill,
                );
            }

            // Handle user input.
            if tabbar_response.hovered() || tab_hovered {
                leaf.scroll += ui.input(|i| i.smooth_scroll_delta.y + i.smooth_scroll_delta.x);
            }
        }

        leaf.scroll = leaf.scroll.clamp(-overflow, 0.0);
    }

    #[allow(clippy::too_many_arguments)]
    fn tab_body(
        &mut self,
        ui: &mut Ui,
        state: &State,
        path: NodePath,
        tab_viewer: &mut impl TabViewer<Tab = Tab>,
        spacing: Vec2,
        tabbar_rect: Rect,
        fade: Option<(&Style, f32)>,
        collapsed: bool,
    ) {
        let (body_rect, _body_response) =
            ui.allocate_exact_size(ui.available_size_before_wrap(), Sense::hover());

        // Captured before borrowing the leaf below (see `tab_bar`/`tabs`): whether the active tab
        // and body are stroked as one silhouette, and the active tab's rect that it traces.
        let body_owns_top = self.body_owns_top_border;
        let active_rect = self.active_tab_rect;

        let leaf = self
            .dock_state
            .leaf_mut(path)
            .expect("This node must be a leaf");
        let LeafNode {
            rect,
            viewport,
            tabs,
            active,
            ..
        } = leaf;
        if !collapsed {
            if let Some(tab) = tabs.get_mut(active.0) {
                if *viewport != body_rect {
                    *viewport = body_rect;
                    tab_viewer.on_rect_changed(tab);
                }

                if ui.input(|i| i.pointer.any_click()) {
                    if let Some(pos) = state.last_hover_pos {
                        if body_rect.contains(pos)
                            && Some(ui.layer_id()) == ui.ctx().layer_id_at(pos)
                        {
                            self.new_focused = Some(path);
                        }
                    }
                }

                let (style, fade_factor) =
                    fade.unwrap_or_else(|| (self.style.as_ref().unwrap(), 1.0));
                let tabs_styles = tab_viewer.tab_style_override(tab, &style.tab);

                let tabs_style = tabs_styles.as_ref().unwrap_or(&style.tab);

                let mut tab_body_corner_radius = tabs_style.tab_body.corner_radius;
                if tabbar_rect != Rect::NOTHING {
                    tab_body_corner_radius.nw = 0;
                    tab_body_corner_radius.ne = 0;
                }

                if tab_viewer.clear_background(tab) {
                    ui.painter().rect_filled(
                        body_rect,
                        tab_body_corner_radius,
                        tabs_style.tab_body.bg_fill,
                    );
                }

                // Construct a new ui with the correct tab id.
                //
                // We are forced to use `Ui::new` because other methods (eg: push_id) always mix
                // the provided id with their own which would cause tabs to change id when moved
                // from node to node.
                let id = self.id.with(tab_viewer.id(tab));
                ui.ctx().check_for_id_clash(id, body_rect, "a tab with id");
                let ui = &mut Ui::new(
                    ui.ctx().clone(),
                    id,
                    UiBuilder::new().max_rect(body_rect).layer_id(ui.layer_id()),
                );
                ui.set_clip_rect(Rect::from_min_max(ui.cursor().min, ui.clip_rect().max));

                // Use initial spacing for ui.
                ui.spacing_mut().item_spacing = spacing;

                let stroke = tabs_style.tab_body.stroke;
                if body_owns_top {
                    // Single-row leaf: stroke the active tab and the body as ONE continuous
                    // silhouette. Their interiors are already filled with the same colour (the tab's
                    // fill in `tab_title`, the body's fill above), so the union reads as a single
                    // shape; tracing one outline around it removes the seam between tab and content
                    // and avoids the stroke-meets-stroke corner over-fill and misaligned hairlines
                    // that independent tab / seam / body strokes produce at fractional DPI.
                    // `tab_title` skips the active tab's own outline + connector for this reason.
                    match active_rect {
                        Some(tab_rect) => {
                            let outline = build_leaf_outline(
                                tab_rect,
                                body_rect,
                                tabs_style.active.corner_radius,
                                tab_body_corner_radius,
                                (stroke.width / 2.0).ceil(),
                            );
                            // Clip to the whole leaf (not just the body) so the tab top, which sits
                            // above the body, isn't clipped; appended after the fills in this layer,
                            // so the outline sits on top of them.
                            let painter =
                                ui.ctx().layer_painter(ui.layer_id()).with_clip_rect(*rect);
                            painter.add(Shape::Path(PathShape {
                                points: outline,
                                closed: true,
                                fill: Color32::TRANSPARENT,
                                stroke: PathStroke::new(stroke.width, stroke.color)
                                    .with_kind(StrokeKind::Inside),
                            }));
                        }
                        None => {
                            // No active tab rect was recorded — e.g. the active tab is being
                            // dragged out, so it floats free and nothing merges into the body.
                            // Give the body its own full (flush) border.
                            ui.painter().rect_stroke(
                                body_rect,
                                tab_body_corner_radius,
                                stroke,
                                StrokeKind::Inside,
                            );
                        }
                    }
                } else {
                    // Multi-row (or no tab bar): the body's top edge is the last tab row's separator
                    // (drawn by `tab_bar_multi_row` at the body stroke width, so it matches these
                    // sides), so push the body's own top edge above the clip to hide it and avoid
                    // doubling. `effective_stroke_width` is the AA-rounded width so a fractional
                    // stroke still clears the clip cleanly. The sides and bottom are stroked flush on
                    // the rect boundary — outline coincident with the fill, like the single-row body,
                    // with no `rect_stroke_box` inset bleeding the fill past the outline.
                    let effective_stroke_width = (stroke.width / 2.0).ceil() * 2.0;
                    let tab_body_rect = Rect::from_min_max(
                        ui.clip_rect().min - vec2(0.0, effective_stroke_width),
                        ui.clip_rect().max,
                    );
                    ui.painter().rect_stroke(
                        tab_body_rect,
                        tab_body_corner_radius,
                        stroke,
                        StrokeKind::Inside,
                    );
                }

                ScrollArea::new(tab_viewer.scroll_bars(tab)).show(ui, |ui| {
                    Frame::new()
                        .inner_margin(tabs_style.tab_body.inner_margin)
                        .show(ui, |ui| {
                            if fade_factor != 1.0 {
                                fade_visuals(ui.visuals_mut(), fade_factor);
                            }
                            let available_rect = ui.available_rect_before_wrap();
                            ui.expand_to_include_rect(available_rect);
                            tab_viewer.ui(ui, tab);
                        });
                });
            }
        }

        // change hover destination
        if let Some(pointer) = state.last_hover_pos {
            // Prevent borrow checker issues.
            let rect = rect.to_owned();

            // if the dragged tab isn't allowed in a window,
            // it's unnecessary to change the hover state
            let is_dragged_valid = match &state.dnd {
                Some(DragDropState {
                    drag: DragData { src, .. },
                    ..
                }) => match *src {
                    TreeComponent::Tab(src_path) => {
                        if let Node::Leaf(leaf) =
                            &mut self.dock_state[src_path.surface][src_path.node]
                        {
                            tab_viewer.allowed_in_windows(&mut leaf.tabs[src_path.tab.0])
                                || path.surface == SurfaceIndex::main()
                        } else {
                            true
                        }
                    }
                    TreeComponent::Node(_) => true,
                    TreeComponent::Surface(_) => unreachable!("surface drags not supported"),
                },
                _ => true,
            };

            // Use rect.contains instead of response.hovered as the dragged tab covers
            // the underlying responses.
            if state.drag_start.is_some() && rect.contains(pointer) && is_dragged_valid {
                let on_title_bar = tabbar_rect.contains(pointer);
                let (dst, tab) = {
                    match self.tab_hover_rect {
                        Some((rect, tab_index)) => {
                            (TreeComponent::Tab((path, tab_index).into()), Some(rect))
                        }
                        None => (
                            TreeComponent::Node(path),
                            on_title_bar.then_some(tabbar_rect),
                        ),
                    }
                };

                ui.memory_mut(|mem| {
                    mem.data.insert_temp(
                        self.id.with("hover_data"),
                        Some(HoverData { rect, dst, tab }),
                    );
                });
            }
        }
    }
}

/// Builds the closed, clockwise outline of the union of the active tab and the body: a body
/// rectangle with the active tab protruding from its top edge. Tab-top corners are rounded by
/// `tab_cr.nw`/`ne` and body-bottom corners by `body_cr.sw`/`se`; the body's own top corners are
/// square (the tab bar sits above). Stroked as one path, this removes the seam between tab and
/// content and any stroke-meets-stroke corner over-fill.
///
/// Wound clockwise so that `StrokeKind::Inside` paints the stroke on the interior side (egui only
/// auto-corrects winding for filled paths, not stroke-only ones).
///
/// The tab's top edge is pulled down by `inset` so the tab sits below the tab-bar separator with a
/// gap rather than flush against it; the tab fill in `tab_title` is pulled down by the same amount
/// so fill and outline coincide. Every other edge stays on the rect boundary — outline flush with
/// the fill, like the rest of egui's content (the old per-side `rect_stroke_box` inset, which left
/// the fill bleeding past the outline, is deliberately not reproduced). The seam stays on the true
/// tab/body boundary.
fn build_leaf_outline(
    tab_rect: Rect,
    body_rect: Rect,
    tab_cr: CornerRadius,
    body_cr: CornerRadius,
    inset: f32,
) -> Vec<Pos2> {
    use std::f32::consts::PI;

    let seam_y = body_rect.top(); // == tab_rect.bottom(): the tab/body boundary
    let (tl, tr) = (tab_rect.left(), tab_rect.right());
    let (bl, br) = (body_rect.left(), body_rect.right());
    let tab_top = tab_rect.top() + inset;
    let body_bottom = body_rect.bottom();

    // Clamp radii so opposing arcs on a short/narrow edge can't overlap.
    let half_tab_w = tab_rect.width() * 0.5;
    let half_body_w = body_rect.width() * 0.5;
    let body_h = body_bottom - seam_y;
    let r_tnw = (tab_cr.nw as f32).clamp(0.0, half_tab_w);
    let r_tne = (tab_cr.ne as f32).clamp(0.0, half_tab_w);
    let r_bsw = (body_cr.sw as f32).clamp(0.0, half_body_w.min(body_h));
    let r_bse = (body_cr.se as f32).clamp(0.0, half_body_w.min(body_h));

    let mut p = Vec::new();
    // Clockwise from the tab's top-left, closing back up the tab's left side.
    push_corner_arc(
        &mut p,
        pos2(tl + r_tnw, tab_top + r_tnw),
        r_tnw,
        PI,
        1.5 * PI,
    ); // tab NW
    push_corner_arc(
        &mut p,
        pos2(tr - r_tne, tab_top + r_tne),
        r_tne,
        1.5 * PI,
        2.0 * PI,
    ); // tab NE
    p.push(pos2(tr, seam_y)); // tab right side meets body top (concave)
    p.push(pos2(br, seam_y)); // body top-right (square)
    push_corner_arc(
        &mut p,
        pos2(br - r_bse, body_bottom - r_bse),
        r_bse,
        0.0,
        0.5 * PI,
    ); // body SE
    push_corner_arc(
        &mut p,
        pos2(bl + r_bsw, body_bottom - r_bsw),
        r_bsw,
        0.5 * PI,
        PI,
    ); // body SW
    p.push(pos2(bl, seam_y)); // body top-left (square)
    p.push(pos2(tl, seam_y)); // body top meets tab left side (concave)
    p
}

/// Builds the OPEN outline (a rounded-top cap) of an inactive tab in a single-row leaf: up the left
/// side, around the rounded top (`cr.nw`/`ne`), and down the right side. There is no bottom edge —
/// the body's top border, drawn under the tab by [`DockArea::tab_body`], is the tab's bottom, so the
/// sides meet it in a clean T rather than doubling it. The top is pulled down by `inset` to match
/// the tab fill (see `tab_title`); the sides stay flush on the rect boundary and run to the seam.
fn build_tab_cap_outline(tab_rect: Rect, cr: CornerRadius, inset: f32) -> Vec<Pos2> {
    use std::f32::consts::PI;

    let (l, r) = (tab_rect.left(), tab_rect.right());
    let top = tab_rect.top() + inset;
    let bottom = tab_rect.bottom();
    let half_w = tab_rect.width() * 0.5;
    let r_nw = (cr.nw as f32).clamp(0.0, half_w);
    let r_ne = (cr.ne as f32).clamp(0.0, half_w);

    let mut p = Vec::new();
    p.push(pos2(l, bottom)); // bottom of the left side
    push_corner_arc(&mut p, pos2(l + r_nw, top + r_nw), r_nw, PI, 1.5 * PI); // NW
    push_corner_arc(&mut p, pos2(r - r_ne, top + r_ne), r_ne, 1.5 * PI, 2.0 * PI); // NE
    p.push(pos2(r, bottom)); // bottom of the right side
    p
}

/// Appends points approximating a quarter-circle arc from `from` to `to` (radians) centred at
/// `center` with `radius`, used to round the corners traced by [`build_leaf_outline`] and
/// [`build_tab_cap_outline`]. A zero radius yields the single corner point (which equals `center`),
/// making a square corner.
fn push_corner_arc(points: &mut Vec<Pos2>, center: Pos2, radius: f32, from: f32, to: f32) {
    if radius <= 0.0 {
        points.push(center);
        return;
    }
    let segments = (radius * 0.75).clamp(3.0, 24.0).ceil() as usize;
    for i in 0..=segments {
        let t = i as f32 / segments as f32;
        let a = from + (to - from) * t;
        points.push(pos2(
            center.x + radius * a.cos(),
            center.y + radius * a.sin(),
        ));
    }
}

/// Near-equality when comparing available tab space (`content`) to the sum of minimum widths.
const TAB_ROW_SUM_EPSILON: f32 = 1e-3;

/// Sum of inter-tab gaps for `range`: spacing before each tab after the first (same rule as `tabs()`).
fn row_gap_sum(tab_layout: &[(f32, f32)], range: &Range<usize>) -> f32 {
    if range.len() <= 1 {
        return 0.0;
    }
    tab_layout[range.start + 1..range.end]
        .iter()
        .map(|(_, spacing)| *spacing)
        .sum()
}

/// Per-tab widths for one row: each tab gets its minimum plus an equal share of leftover space.
/// If the row is too narrow, widths shrink proportionally. A final scale removes float overshoot.
fn compute_tab_widths_for_row(
    row_width: f32,
    tab_layout: &[(f32, f32)],
    range: &Range<usize>,
) -> Vec<f32> {
    let n = range.len();
    if n == 0 {
        return vec![];
    }
    let gap_sum = row_gap_sum(tab_layout, range);
    let content = (row_width - gap_sum).max(0.0);
    let mins: Vec<f32> = tab_layout[range.clone()].iter().map(|(m, _)| *m).collect();
    let sum_min: f32 = mins.iter().sum();

    let mut widths: Vec<f32> = if content + TAB_ROW_SUM_EPSILON >= sum_min {
        let slack = (content - sum_min).max(0.0);
        let extra = slack / n as f32;
        mins.iter().map(|m| m + extra).collect()
    } else if sum_min > 0.0 {
        let scale = content / sum_min;
        mins.iter().map(|m| m * scale).collect()
    } else {
        vec![0.0; n]
    };

    let total: f32 = widths.iter().sum();
    if total > content && total > 0.0 {
        let s = content / total;
        for w in &mut widths {
            *w *= s;
        }
    }

    widths
}

/// Wraps tabs into rows; each row's minimum width includes inter-tab spacing (same as `tabs()`).
fn distribute_tabs_for_width(tab_layout: &[(f32, f32)], available_width: f32) -> Vec<Range<usize>> {
    if tab_layout.is_empty() {
        return vec![];
    }

    let mut result = Vec::new();
    let mut row_start = 0usize;
    let mut row_width = 0.0_f32;

    for (idx, (min_width, spacing)) in tab_layout.iter().copied().enumerate() {
        let additional = if idx == row_start {
            min_width
        } else {
            spacing + min_width
        };

        if idx > row_start && row_width + additional > available_width {
            result.push(row_start..idx);
            row_start = idx;
            row_width = min_width;
        } else {
            row_width += additional;
        }
    }

    result.push(row_start..tab_layout.len());
    result
}
