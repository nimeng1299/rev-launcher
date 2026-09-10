use gpui_kit::{Context, IntoElement, Render, Window, div};

pub struct DevelopPage {}

impl DevelopPage {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {}
    }
}

impl Render for DevelopPage {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
