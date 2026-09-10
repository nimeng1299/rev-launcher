use gpui_kit::component::button::Button;
use gpui_kit::component::select::{Select, SelectState};
use gpui_kit::component::{IndexPath, Sizable, StyledExt};
use gpui_kit::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled, Window,
    div, px,
};

pub struct StartPage {
    select_state: Entity<SelectState<Vec<SharedString>>>,
    selected: Option<SharedString>,
}

impl StartPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let select_state = cx.new(|cx| {
            SelectState::new(
                vec!["GPUI".into(), "Iced".into(), "egui".into()],
                Some(IndexPath::default()),
                window,
                cx,
            )
        });

        Self {
            select_state,
            selected: None,
        }
    }
}

impl Render for StartPage {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().v_flex().size_full().justify_end().child(
            div()
                .v_flex()
                .gap_2()
                .w_full()
                .px(px(15.))
                .pb(px(15.))
                .child(
                    div().h_flex().justify_end().child(
                        Button::new("launch")
                            .label("启动游戏")
                            .large()
                            .h(px(60.))
                            .px(px(40.))
                            .text_3xl()
                            .on_click(|_, _, _| {
                                println!("launch button clicked");
                            }),
                    ),
                )
                .child(
                    div().w_full().child(
                        Select::new(&self.select_state)
                            .disabled(false)
                            .large()
                            .h(px(48.)),
                    ),
                ),
        )
    }
}
