use gpui_kit::{div, Context, IntoElement, Render, Styled, Window, ParentElement, rgb, Div};
use gpui_kit::component::scroll::ScrollableElement;

pub struct DevelopPage {}

impl DevelopPage {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {}
    }
}

impl Render for DevelopPage {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .overflow_y_scrollbar()
            .child(
                card_outline()
            )
    }
}

fn card_outline() -> Div {
    div()
        .flex_col()
        .border_1()
        .border_color(rgb(0xe5e7eb))
        .rounded_lg()
        .p_4()
}