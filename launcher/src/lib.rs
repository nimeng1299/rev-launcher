use crate::data::account_data::AccountData;
use crate::data::app_data::AppData;
use crate::data::settings::AppSettings;
use crate::window::main_window::MainWindow;
use gpui_kit::component::{Root, Theme, ThemeMode, TitleBar};
use gpui_kit::{AppContext, Bounds, Point, Size, WindowBounds, px};

pub mod data;
pub mod window;

pub fn run_app() {
    let application = gpui_kit::application().with_assets(gpui_kit::assets::Assets);

    application.run(move |app| {
        gpui_kit::init(app);

        let display = app.displays().first().cloned().unwrap();
        let physical_size = display.bounds().size;

        let logical_w = physical_size.width.to_f64();
        let logical_h = physical_size.height.to_f64();

        let win_w = logical_w * 0.65;
        let win_h = logical_h * 0.65;
        let win_x = (logical_w - win_w) / 2.0;
        let win_y = (logical_h - win_h) / 2.0;

        let mut options = TitleBar::window_options();
        options.window_bounds = Some(WindowBounds::Windowed(Bounds {
            origin: Point {
                x: px(win_x as f32),
                y: px(win_y as f32),
            },
            size: Size {
                width: px(win_w as f32),
                height: px(win_h as f32),
            },
        }));

        app.set_global(AccountData::init());
        app.set_global(AppData::init());
        app.set_global(AppSettings::init());

        app.spawn(async move |cx| {
            cx.open_window(options, |window, cx| {
                Theme::change(ThemeMode::Light, Some(window), cx);
                let view = cx.new(|_| MainWindow::default());
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("Failed to open window");
        })
        .detach();
    });
}
