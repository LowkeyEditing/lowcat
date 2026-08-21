use super::*;

struct PreviewWaveformElement {
    id: ElementId,
    table: Entity<FileTable>,
    path: PathBuf,
    waveform: Option<WaveformBinary256>,
    trim: Option<TrimRange>,
    trim_enabled: bool,
    playhead_bits: Arc<AtomicU32>,
}

pub(super) fn element(
    id: ElementId,
    table: Entity<FileTable>,
    path: PathBuf,
    waveform: Option<WaveformBinary256>,
    trim: Option<TrimRange>,
    trim_enabled: bool,
    playhead_bits: Arc<AtomicU32>,
) -> impl IntoElement {
    PreviewWaveformElement {
        id,
        table,
        path,
        waveform,
        trim,
        trim_enabled,
        playhead_bits,
    }
}

#[derive(Clone, Copy)]
enum PreviewScrubAction {
    Begin(Option<TrimEdge>),
    Continue,
    End,
}

struct PreviewHitboxes {
    surface: Hitbox,
    start_edge: Option<Hitbox>,
    end_edge: Option<Hitbox>,
}

impl IntoElement for PreviewWaveformElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for PreviewWaveformElement {
    type RequestLayoutState = ();
    type PrepaintState = PreviewHitboxes;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (
            window.request_layout(
                Style {
                    size: size(relative(1.).into(), relative(1.).into()),
                    ..Style::default()
                },
                [],
                cx,
            ),
            (),
        )
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        let surface = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let (start_edge, end_edge) =
            self.trim
                .filter(|_| self.trim_enabled)
                .map_or((None, None), |trim| {
                    let width = bounds.size.width.as_f32().max(1.);
                    let edge_bounds = |ratio: f32| {
                        Bounds::new(
                            point(
                                px(bounds.left().as_f32() + width * ratio - 3.),
                                bounds.top(),
                            ),
                            size(px(6.), bounds.size.height),
                        )
                    };
                    (
                        Some(
                            window.insert_hitbox(
                                edge_bounds(trim.start_ratio),
                                HitboxBehavior::Normal,
                            ),
                        ),
                        Some(
                            window
                                .insert_hitbox(edge_bounds(trim.end_ratio), HitboxBehavior::Normal),
                        ),
                    )
                });
        PreviewHitboxes {
            surface,
            start_edge,
            end_edge,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        paint_preview_waveform(
            bounds,
            self.waveform,
            self.trim,
            FileTable::load_preview_playhead(&self.playhead_bits),
            window,
        );
        let hitbox = prepaint.surface.clone();
        window.set_cursor_style(CursorStyle::PointingHand, &hitbox);
        if let Some(handle) = prepaint.start_edge.as_ref() {
            window.set_cursor_style(CursorStyle::ResizeLeftRight, handle);
        }
        if let Some(handle) = prepaint.end_edge.as_ref() {
            window.set_cursor_style(CursorStyle::ResizeLeftRight, handle);
        }

        window.on_mouse_event({
            let table = self.table.clone();
            let path = self.path.clone();
            let hitbox = hitbox.clone();
            let start_edge = prepaint.start_edge.clone();
            let end_edge = prepaint.end_edge.clone();
            let trim = self.trim;
            move |event: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble
                    || event.button != MouseButton::Left
                    || !event.modifiers.alt
                    || !hitbox.is_hovered(window)
                {
                    return;
                }
                let trim_enabled = event.modifiers.platform;
                if event.click_count == 2 && trim_enabled {
                    cx.update_entity(&table, |table, cx| {
                        table.cancel_preview_trim(&path, cx);
                    });
                    cx.stop_propagation();
                    window.prevent_default();
                    return;
                }
                let edge = if start_edge
                    .as_ref()
                    .is_some_and(|handle| handle.is_hovered(window))
                {
                    Some(TrimEdge::Start)
                } else if end_edge
                    .as_ref()
                    .is_some_and(|handle| handle.is_hovered(window))
                {
                    Some(TrimEdge::End)
                } else {
                    None
                };
                scrub_preview_from_position(
                    &table,
                    &path,
                    event.position,
                    bounds,
                    PreviewScrubAction::Begin(edge),
                    trim,
                    trim_enabled,
                    cx,
                );
                cx.stop_propagation();
                window.prevent_default();
            }
        });

        window.on_mouse_event({
            let table = self.table.clone();
            let path = self.path.clone();
            move |event: &MouseMoveEvent, phase, _, cx| {
                if phase != DispatchPhase::Bubble || !event.dragging() || !event.modifiers.alt {
                    return;
                }
                scrub_preview_from_position(
                    &table,
                    &path,
                    event.position,
                    bounds,
                    PreviewScrubAction::Continue,
                    None,
                    event.modifiers.platform,
                    cx,
                );
                cx.stop_propagation();
            }
        });

        window.on_mouse_event({
            let table = self.table.clone();
            let path = self.path.clone();
            move |event: &MouseUpEvent, phase, _, cx| {
                if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                    return;
                }
                scrub_preview_from_position(
                    &table,
                    &path,
                    event.position,
                    bounds,
                    PreviewScrubAction::End,
                    None,
                    event.modifiers.alt && event.modifiers.platform,
                    cx,
                );
                cx.stop_propagation();
            }
        });
    }
}

fn paint_preview_waveform(
    bounds: Bounds<Pixels>,
    waveform: Option<WaveformBinary256>,
    trim: Option<TrimRange>,
    playhead_ratio: Option<f32>,
    window: &mut Window,
) {
    let row_width = bounds.size.width.as_f32().max(1.);
    let color = white();
    let trim_color = hsla(0.085, 0.9, 0.72, 1.);

    if let Some(trim) = trim {
        let x = bounds.left().as_f32() + row_width * trim.start_ratio;
        let width = row_width * (trim.end_ratio - trim.start_ratio);
        window.paint_quad(fill(
            Bounds::new(
                point(px(x), bounds.top()),
                size(px(width), bounds.size.height),
            ),
            trim_color.opacity(0.14),
        ));
    }

    if let Some(waveform) = waveform {
        let row_height = bounds.size.height.as_f32().max(1.);
        let bar_gap = 1.;
        let bar_width = ((row_width - bar_gap * (WAVEFORM_BAR_COUNT - 1) as f32)
            / WAVEFORM_BAR_COUNT as f32)
            .max(1.);

        for (ix, value) in waveform.into_iter().enumerate() {
            let height = if value == 0 {
                1.
            } else {
                ((value as f32 / 255.) * row_height).max(1.)
            };
            let x = bounds.left().as_f32() + ix as f32 * (bar_width + bar_gap);
            let y = bounds.bottom().as_f32() - height;
            let ratio = (ix as f32 + 0.5) / WAVEFORM_BAR_COUNT as f32;
            let bar_color =
                if trim.is_some_and(|trim| ratio >= trim.start_ratio && ratio <= trim.end_ratio) {
                    trim_color
                } else {
                    color
                };
            window.paint_quad(fill(
                Bounds::new(point(px(x), px(y)), size(px(bar_width), px(height))),
                bar_color,
            ));
        }
    }

    if let Some(trim) = trim {
        for ratio in [trim.start_ratio, trim.end_ratio] {
            let x = bounds.left().as_f32() + row_width * ratio;
            window.paint_quad(fill(
                Bounds::new(
                    point(px(x - 1.), bounds.top()),
                    size(px(2.), bounds.size.height),
                ),
                trim_color,
            ));
        }
    }

    if let Some(ratio) = playhead_ratio {
        let x = bounds.left().as_f32() + row_width * ratio.clamp(0., 1.);
        window.paint_quad(fill(
            Bounds::new(point(px(x), bounds.top()), size(px(2.), bounds.size.height)),
            color,
        ));
    }
}

fn scrub_preview_from_position(
    table: &Entity<FileTable>,
    path: &Path,
    position: Point<Pixels>,
    bounds: Bounds<Pixels>,
    action: PreviewScrubAction,
    persisted: Option<TrimRange>,
    trim_enabled: bool,
    cx: &mut App,
) {
    let x = position.x.as_f32() - bounds.left().as_f32();
    let width = bounds.size.width.as_f32().max(1.);
    let ratio = (x / width).clamp(0., 1.);
    cx.update_entity(table, |table, cx| match action {
        PreviewScrubAction::Begin(edge) => {
            table.begin_preview_scrub(
                path.to_path_buf(),
                ratio,
                x,
                width,
                edge,
                persisted,
                trim_enabled,
                cx,
            );
        }
        PreviewScrubAction::Continue => {
            table.continue_preview_scrub(path, ratio, x, trim_enabled, cx);
        }
        PreviewScrubAction::End => {
            table.continue_preview_scrub(path, ratio, x, trim_enabled, cx);
            table.end_preview_scrub(path, trim_enabled, cx);
        }
    });
}
