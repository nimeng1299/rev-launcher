use gpui_kit::{Context, IntoElement, Render, Window, div};

pub struct VersionControlPage {}

impl VersionControlPage {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {}
    }
}

impl Render for VersionControlPage {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
