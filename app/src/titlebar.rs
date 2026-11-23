use crate::{BG, HOVER, LINE, MUTED, RULE, TEXT};
use gpui::{
    AnyElement, Bounds, BoxShadow, CursorStyle, Decorations, Div, HitboxBehavior, Hsla, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Pixels, Point, ResizeEdge, Size, Stateful, StatefulInteractiveElement, Styled, Tiling, TitlebarOptions, Window,
    WindowControlArea, WindowDecorations, canvas, div, point, prelude::FluentBuilder, px, rgb, size, transparent_black,
};

const BAR: f32 = 32.;
const BUTTON: f32 = 46.;
const SHADOW: f32 = 10.;

pub fn options() -> (TitlebarOptions, Option<WindowDecorations>) {
    let title = Some("Nano".into());
    if cfg!(target_os = "macos") {
        (TitlebarOptions { title, ..Default::default() }, None)
    } else if cfg!(windows) {
        (TitlebarOptions { title, appears_transparent: true, ..Default::default() }, None)
    } else {
        (TitlebarOptions { title, ..Default::default() }, Some(WindowDecorations::Client))
    }
}

fn client(window: &Window) -> Option<Tiling> {
    match window.window_decorations() {
        Decorations::Client { tiling } => Some(tiling),
        Decorations::Server => None,
    }
}

fn shown(window: &Window) -> bool {
    !cfg!(target_os = "macos") && !window.is_fullscreen() && (cfg!(windows) || client(window).is_some())
}

pub fn viewport(window: &Window) -> Size<Pixels> {
    let full = window.viewport_size();
    let edges = |a: bool, b: bool| (u8::from(!a) + u8::from(!b)) as f32 * (SHADOW + 1.);
    let (across, down) = client(window).map_or((0., 0.), |t| (edges(t.left, t.right), edges(t.top, t.bottom)));
    size(full.width - px(across), full.height - px(down + if shown(window) { BAR } else { 0. }))
}

pub fn origin(window: &Window) -> (f32, f32) {
    let edge = |tiled: bool| if tiled { 0. } else { SHADOW + 1. };
    let (left, top) = client(window).map_or((0., 0.), |t| (edge(t.left), edge(t.top)));
    (left, top + if shown(window) { BAR } else { 0. })
}

pub fn wrap(page: Div, window: &mut Window) -> AnyElement {
    if cfg!(target_os = "macos") {
        return page.into_any_element();
    }
    let tiling = client(window);
    let show = shown(window);
    let body = div()
        .size_full()
        .flex()
        .flex_col()
        .bg(rgb(BG))
        .children(show.then(|| bar(window)))
        .child(div().relative().flex_1().min_h_0().child(page));
    let Some(tiling) = tiling else { return body.into_any_element() };
    window.set_client_inset(px(SHADOW));
    frame(body, tiling)
}

fn bar(window: &Window) -> Stateful<Div> {
    let linux = !cfg!(windows);
    let maximized = window.is_maximized();
    let mut drag = div()
        .id("titlebar-drag")
        .flex_1()
        .h_full()
        .px(px(12.))
        .flex()
        .items_center()
        .window_control_area(WindowControlArea::Drag)
        .font_family(crate::fonts::sans())
        .text_size(px(12.))
        .text_color(rgb(MUTED))
        .child("Nano");
    if linux {
        drag = drag
            .on_mouse_down(MouseButton::Left, |e, window, _| {
                if e.click_count == 2 {
                    window.zoom_window();
                } else {
                    window.start_window_move();
                }
            })
            .on_mouse_down(MouseButton::Right, |e, window, _| window.show_window_menu(e.position));
    }
    div()
        .id("titlebar")
        .flex_none()
        .w_full()
        .h(px(BAR))
        .flex()
        .bg(rgb(BG))
        .border_b_1()
        .border_color(rgb(RULE))
        .child(drag)
        .child(button("min", "icons/win-min.svg", WindowControlArea::Min, false).when(linux, |b| b.on_click(|_, window, _| window.minimize_window())))
        .child(
            button("max", if maximized { "icons/win-restore.svg" } else { "icons/win-max.svg" }, WindowControlArea::Max, false)
                .when(linux, |b| b.on_click(|_, window, _| window.zoom_window())),
        )
        .child(button("close", "icons/win-close.svg", WindowControlArea::Close, true).when(linux, |b| b.on_click(|_, window, _| window.remove_window())))
}

fn button(id: &'static str, icon: &'static str, area: WindowControlArea, close: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .w(px(BUTTON))
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .window_control_area(area)
        .hover(move |s| s.bg(rgb(if close { 0xC42B1C } else { HOVER })))
        .child(gpui::svg().path(icon).flex_none().size(px(12.)).text_color(rgb(TEXT)))
}

fn frame(body: Div, tiling: Tiling) -> AnyElement {
    let inset = px(SHADOW);
    let shadow = BoxShadow::new(px(0.), px(0.), Hsla { h: 0., s: 0., l: 0., a: 0.4 }).blur_radius(inset / 2.);
    div()
        .id("frame")
        .size_full()
        .bg(transparent_black())
        .child(
            canvas(
                |_, window, _| window.insert_hitbox(Bounds::new(point(px(0.), px(0.)), window.window_bounds().get_bounds().size), HitboxBehavior::Normal),
                move |_, hitbox, window, _| {
                    let size = window.window_bounds().get_bounds().size;
                    if let Some(edge) = resize_edge(window.mouse_position(), inset, size) {
                        window.set_cursor_style(cursor(edge), &hitbox);
                    }
                },
            )
            .size_full()
            .absolute(),
        )
        .when(!tiling.top, |d| d.pt(inset))
        .when(!tiling.bottom, |d| d.pb(inset))
        .when(!tiling.left, |d| d.pl(inset))
        .when(!tiling.right, |d| d.pr(inset))
        .on_mouse_move(|_, window, _| window.refresh())
        .on_mouse_down(MouseButton::Left, move |e, window, _| {
            let size = window.window_bounds().get_bounds().size;
            if let Some(edge) = resize_edge(e.position, inset, size) {
                window.start_window_resize(edge);
            }
        })
        .child(
            body.cursor(CursorStyle::Arrow)
                .border_color(rgb(LINE))
                .when(!tiling.top, |d| d.border_t_1())
                .when(!tiling.bottom, |d| d.border_b_1())
                .when(!tiling.left, |d| d.border_l_1())
                .when(!tiling.right, |d| d.border_r_1())
                .when(!tiling.is_tiled(), |d| d.shadow(vec![shadow]))
                .on_mouse_move(|_, _, cx| cx.stop_propagation()),
        )
        .into_any_element()
}

fn cursor(edge: ResizeEdge) -> CursorStyle {
    match edge {
        ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
        ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
        ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
        ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
    }
}

fn resize_edge(pos: Point<Pixels>, inset: Pixels, size: Size<Pixels>) -> Option<ResizeEdge> {
    let (top, bottom, left, right) = (pos.y < inset, pos.y > size.height - inset, pos.x < inset, pos.x > size.width - inset);
    Some(match (top, bottom, left, right) {
        (true, _, true, _) => ResizeEdge::TopLeft,
        (true, _, _, true) => ResizeEdge::TopRight,
        (_, true, true, _) => ResizeEdge::BottomLeft,
        (_, true, _, true) => ResizeEdge::BottomRight,
        (true, ..) => ResizeEdge::Top,
        (_, true, ..) => ResizeEdge::Bottom,
        (_, _, true, _) => ResizeEdge::Left,
        (_, _, _, true) => ResizeEdge::Right,
        _ => return None,
    })
}
